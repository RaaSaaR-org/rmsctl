//! Tests for `datasets download`.
//!
//! The flow runs against a small in-process HTTP server that implements the
//! download contract — a paged manifest and raw files with `Range` — and can
//! be told to misbehave, so "a dropped connection continues where it stopped"
//! and "a complete file is not asked for again" are checked on real requests
//! and real bytes.

use super::*;
use crate::config::RmsConfig;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};

// ── Pure helpers ────────────────────────────────────────────────────────

#[test]
fn only_plain_relative_paths_are_written() {
    for ok in [
        "meta/info.json",
        "videos/observation.images.head/chunk-000/file-000.mp4",
        "data/chunk-000/file-000.parquet",
        "meta/a b.json",
        "meta/..hidden",
    ] {
        assert!(safe_rel(ok).is_ok(), "{ok} should be accepted");
    }
    assert_eq!(
        safe_rel("meta/info.json").unwrap(),
        Path::new("meta").join("info.json")
    );
    for bad in [
        "",
        "/etc/passwd",
        "../outside",
        "meta/../../outside",
        "meta//info.json",
        "meta/./info.json",
        "meta/",
        "meta\\info.json",
        "C:/Users/x",
        "meta/in\nfo.json",
        "meta/info.json.rmsctl-part",
    ] {
        assert!(safe_rel(bad).is_err(), "{bad:?} should be refused");
    }
}

#[test]
fn content_range_is_read_or_refused() {
    assert_eq!(
        parse_content_range("bytes 100-199/200"),
        Some((100, 199, 200))
    );
    assert_eq!(parse_content_range(" bytes 0-0/1 "), Some((0, 0, 1)));
    assert_eq!(parse_content_range("bytes */200"), None);
    assert_eq!(parse_content_range("items 0-1/2"), None);
    assert_eq!(parse_content_range(""), None);
}

#[test]
fn a_dataset_name_becomes_one_directory_name() {
    assert_eq!(
        dir_name_for("g1-dex3-bottle-sorting-sim", "id"),
        "g1-dex3-bottle-sorting-sim"
    );
    assert_eq!(
        dir_name_for("Bottle sorting / run 2", "id"),
        "Bottle-sorting---run-2"
    );
    assert_eq!(dir_name_for("../../etc", "id"), "etc");
    assert_eq!(dir_name_for("  ", "abc-123"), "abc-123");
    assert_eq!(dir_name_for("..", "abc-123"), "abc-123");
}

#[test]
fn a_cursor_survives_the_query_string() {
    assert_eq!(encode_query("meta/info.json"), "meta/info.json");
    assert_eq!(encode_query("meta/a b&c=d.json"), "meta/a%20b%26c%3Dd.json");
    assert_eq!(encode_query("meta/é.json"), "meta/%C3%A9.json");
}

// ── Scratch directories ─────────────────────────────────────────────────

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rmsctl-dl-test-{}-{}-{tag}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u32 * 31 + seed as u32 * 7 + (i as u32 >> 8)) as u8)
        .collect()
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ── A server that speaks the download contract ──────────────────────────

const ID: &str = "ds-1";
const VIDEO: &str = "videos/observation.images.head/chunk-000/file-000.mp4";
const DATA: &str = "data/chunk-000/file-000.parquet";

#[derive(Clone)]
enum Fault {
    /// Answer with this refusal; an empty code is a proxy's answer.
    Refuse(u16, &'static str, Option<u64>),
    /// Send the headers and this many body bytes, then drop the connection.
    DropAfter(usize),
    /// Send the right number of bytes, one of them wrong.
    Corrupt,
    /// Ignore the Range header: 200 and the whole file.
    IgnoreRange,
}

#[derive(Default)]
struct Mock {
    name: String,
    files: Vec<(String, Vec<u8>)>,
    /// Files per manifest page, whatever `limit` asks for.
    page: usize,
    /// List a SHA-256 for every file, like a dataset that was uploaded with rmsctl.
    with_sha: bool,
    /// Faults for the content requests of a path, used up in order.
    faults: HashMap<String, VecDeque<Fault>>,
    /// Extra manifest entries the dataset does not really have.
    extra_listing: Vec<(String, u64)>,
    /// List these files with a size they do not have.
    listed_size: HashMap<String, u64>,
    /// Answer the manifest like a server that predates the API.
    no_files_api: bool,
    /// `METHOD path [range]`, one line per request.
    log: Vec<String>,
    max_in_flight: usize,
}

impl Mock {
    fn tree() -> Self {
        Mock {
            name: "g1 bottle/sorting".into(),
            page: 2,
            with_sha: true,
            files: vec![
                (DATA.into(), pattern(3000, 2)),
                ("meta/empty.jsonl".into(), Vec::new()),
                (
                    "meta/episodes/chunk-000/file-000.parquet".into(),
                    pattern(400, 1),
                ),
                ("meta/info.json".into(), br#"{"fps":30}"#.to_vec()),
                (VIDEO.into(), pattern(200_000, 3)),
            ],
            ..Default::default()
        }
    }

    fn fault(&mut self, path: &str, fault: Fault) {
        self.faults.entry(path.into()).or_default().push_back(fault);
    }

    fn content_requests(&self, path: &str) -> Vec<String> {
        let needle = format!("GET content {path} ");
        self.log
            .iter()
            .filter(|l| l.starts_with(&needle))
            .cloned()
            .collect()
    }

    fn manifest_page(&self, after: Option<&str>) -> Value {
        let mut all: Vec<(String, u64, Option<String>)> = self
            .files
            .iter()
            .map(|(p, b)| {
                let size = self.listed_size.get(p).copied().unwrap_or(b.len() as u64);
                (p.clone(), size, self.with_sha.then(|| sha(b)))
            })
            .chain(
                self.extra_listing
                    .iter()
                    .map(|(p, s)| (p.clone(), *s, None)),
            )
            .collect();
        all.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let rest: Vec<_> = all
            .into_iter()
            .filter(|(p, _, _)| after.is_none_or(|a| p.as_bytes() > a.as_bytes()))
            .collect();
        let page: Vec<_> = rest.iter().take(self.page).collect();
        let next = (rest.len() > self.page).then(|| page.last().unwrap().0.clone());
        json!({
            "datasetId": ID,
            "storage": "rustfs",
            "limit": self.page,
            "files": page.iter().map(|(p, s, h)| {
                let mut f = json!({ "path": p, "size": s, "etag": format!("etag-{s}") });
                if let Some(h) = h { f["sha256"] = json!(h); }
                f
            }).collect::<Vec<_>>(),
            "nextCursor": next,
        })
    }
}

fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn query_param(target: &str, key: &str) -> Option<String> {
    let (_, query) = target.split_once('?')?;
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| decode(v))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

async fn respond(
    sock: &mut TcpStream,
    status: u16,
    headers: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} X\r\n{headers}content-length: {}\r\n\r\n",
        body.len()
    );
    sock.write_all(head.as_bytes()).await?;
    sock.write_all(body).await
}

async fn respond_json(sock: &mut TcpStream, status: u16, extra: &str, value: &Value) -> bool {
    let text = value.to_string();
    respond(
        sock,
        status,
        &format!("content-type: application/json\r\n{extra}"),
        text.as_bytes(),
    )
    .await
    .is_ok()
}

async fn serve_connection(
    mut sock: TcpStream,
    mock: Arc<Mutex<Mock>>,
    in_flight: Arc<AtomicUsize>,
) {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = vec![0u8; 16 * 1024];
    loop {
        let head_end = loop {
            if let Some(p) = find(&buf, b"\r\n\r\n") {
                break p;
            }
            match sock.read(&mut tmp).await {
                Ok(n) if n > 0 => buf.extend_from_slice(&tmp[..n]),
                _ => return,
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        buf.drain(..head_end + 4);
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap_or("").split(' ');
        let method = first.next().unwrap_or("").to_string();
        let target = first.next().unwrap_or("").to_string();
        let headers: HashMap<String, String> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let route = target.split('?').next().unwrap_or("").to_string();

        if route == format!("/api/datasets/{ID}") {
            let body = {
                let mut m = mock.lock().unwrap();
                m.log.push(format!("{method} dataset"));
                json!({ "dataset": { "id": ID, "name": m.name, "status": "ready" } })
            };
            if !respond_json(&mut sock, 200, "", &body).await {
                return;
            }
            continue;
        }
        if route.starts_with("/api/datasets/") && !route.starts_with(&format!("/api/datasets/{ID}"))
        {
            let body = json!({ "error": "Dataset not found" });
            if !respond_json(&mut sock, 404, "", &body).await {
                return;
            }
            continue;
        }
        if route == format!("/api/datasets/{ID}/files") {
            let after = query_param(&target, "after");
            let (status, body) = {
                let mut m = mock.lock().unwrap();
                m.log.push(format!(
                    "{method} files {}",
                    after.as_deref().unwrap_or("-")
                ));
                if m.no_files_api {
                    (404, json!({ "error": "Cannot GET" }))
                } else {
                    (200, m.manifest_page(after.as_deref()))
                }
            };
            if !respond_json(&mut sock, status, "", &body).await {
                return;
            }
            continue;
        }
        if route != format!("/api/datasets/{ID}/files/content") {
            let _ = respond_json(&mut sock, 404, "", &json!({ "error": "no such route" })).await;
            return;
        }

        // One stored file.
        let path = query_param(&target, "path").unwrap_or_default();
        let range = headers.get("range").cloned();
        let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        let (found, fault) = {
            let mut m = mock.lock().unwrap();
            m.max_in_flight = m.max_in_flight.max(now);
            m.log.push(format!(
                "GET content {path} {}",
                range.as_deref().unwrap_or("-")
            ));
            let fault = m.faults.get_mut(&path).and_then(|q| q.pop_front());
            let found = m
                .files
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, b)| b.clone());
            (found, fault)
        };
        // Hold the request open for a moment so parallel files overlap.
        tokio::time::sleep(Duration::from_millis(3)).await;
        in_flight.fetch_sub(1, Ordering::SeqCst);

        if let Some(Fault::Refuse(status, code, retry_after)) = &fault {
            let mut body = json!({ "error": "refused by the test" });
            if !code.is_empty() {
                body["code"] = json!(code);
            }
            let extra = retry_after
                .map(|s| format!("retry-after: {s}\r\n"))
                .unwrap_or_default();
            if !respond_json(&mut sock, *status, &extra, &body).await {
                return;
            }
            continue;
        }
        let Some(bytes) = found else {
            let body = json!({ "error": "The dataset has no file", "code": "FILE_NOT_FOUND", "path": path });
            if !respond_json(&mut sock, 404, "", &body).await {
                return;
            }
            continue;
        };
        let size = bytes.len();
        let start = match (&range, &fault) {
            (_, Some(Fault::IgnoreRange)) | (None, _) => None,
            (Some(r), _) => {
                let from = r
                    .strip_prefix("bytes=")
                    .and_then(|r| r.strip_suffix('-'))
                    .and_then(|n| n.parse::<usize>().ok());
                match from {
                    Some(n) if n < size => Some(n),
                    _ => {
                        let body = json!({ "error": "outside the file", "code": "RANGE_NOT_SATISFIABLE", "size": size });
                        let extra = format!("content-range: bytes */{size}\r\n");
                        if !respond_json(&mut sock, 416, &extra, &body).await {
                            return;
                        }
                        continue;
                    }
                }
            }
        };
        let mut body = bytes[start.unwrap_or(0)..].to_vec();
        if matches!(fault, Some(Fault::Corrupt)) && !body.is_empty() {
            let last = body.len() - 1;
            body[last] ^= 0xff;
        }
        let etag = format!("etag: \"etag-{size}\"\r\n");
        let (status, extra) = match start {
            Some(n) => (
                206,
                format!(
                    "{etag}content-range: bytes {n}-{}/{size}\r\n",
                    size.saturating_sub(1)
                ),
            ),
            None => (200, etag),
        };
        if let Some(Fault::DropAfter(keep)) = fault {
            let head = format!(
                "HTTP/1.1 {status} X\r\n{extra}content-length: {}\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(head.as_bytes()).await;
            let _ = sock.write_all(&body[..keep.min(body.len())]).await;
            let _ = sock.flush().await;
            // Let the bytes reach the client before the connection goes.
            tokio::time::sleep(Duration::from_millis(30)).await;
            return;
        }
        if respond(&mut sock, status, &extra, &body).await.is_err() {
            return;
        }
    }
}

struct Server {
    mock: Arc<Mutex<Mock>>,
    client: ApiClient,
}

async fn server(mock: Mock) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = Arc::new(Mutex::new(mock));
    let shared = mock.clone();
    let in_flight = Arc::new(AtomicUsize::new(0));
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(serve_connection(sock, shared.clone(), in_flight.clone()));
        }
    });
    let client = ApiClient::new(&RmsConfig {
        server_url: format!("http://{addr}/api"),
        token: Some("test-token".into()),
    });
    Server { mock, client }
}

fn ctx() -> Ctx {
    // --json doubles as the quiet mode, which keeps the test output readable.
    Ctx {
        json: true,
        yes: false,
        page: None,
        limit: None,
    }
}

fn options(out: &Path) -> DownloadOptions {
    let ms = Duration::from_millis;
    DownloadOptions {
        id: ID.into(),
        out: Some(out.to_path_buf()),
        parallel: 4,
        retries: 3,
        resume: false,
        force: false,
        no_checksum: false,
        stall_timeout: Duration::from_secs(10),
        tuning: Tuning {
            backoff_base: ms(1),
            max_pause: ms(5),
            poll_interval: ms(2),
        },
    }
}

/// Every file the server holds is on disk, byte for byte, and nothing else is.
fn assert_tree_matches(mock: &Mock, out: &Path) {
    for (path, bytes) in &mock.files {
        let disk = std::fs::read(out.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(disk == *bytes, "{path} differs");
    }
    let mut found = Vec::new();
    collect(out, out, &mut found);
    found.sort();
    let mut expected: Vec<String> = mock.files.iter().map(|(p, _)| p.clone()).collect();
    expected.sort();
    assert_eq!(found, expected, "exactly the dataset's files are there");
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(root, &path, out);
        } else {
            out.push(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

// ── The flow ────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn downloads_the_tree_byte_for_byte_across_manifest_pages() {
    let s = server(Mock::tree()).await;
    let out = Scratch::new("straight");
    let target = out.path("dataset");

    let outcome = download(&s.client, &ctx(), &options(&target))
        .await
        .unwrap();

    let mock = s.mock.lock().unwrap();
    assert_tree_matches(&mock, &target);
    assert_eq!(outcome.files, 5);
    assert_eq!(outcome.bytes, 3000 + 400 + 10 + 200_000);
    assert_eq!(outcome.fetched_bytes, outcome.bytes);
    assert_eq!(outcome.skipped_files, 0);
    assert_eq!(
        outcome.verified_sha256, 5,
        "every listed SHA-256 was compared"
    );
    // Three pages of two: each asked for with the cursor of the one before.
    let pages: Vec<&str> = mock
        .log
        .iter()
        .filter(|l| l.starts_with("GET files"))
        .map(String::as_str)
        .collect();
    assert_eq!(
        pages,
        [
            "GET files -",
            "GET files meta/empty.jsonl",
            "GET files meta/info.json"
        ]
    );
    // An empty file needs no request; every other file is asked for once, whole.
    assert!(mock.content_requests("meta/empty.jsonl").is_empty());
    assert_eq!(
        mock.content_requests(VIDEO),
        [format!("GET content {VIDEO} -")]
    );
    assert!(mock.max_in_flight <= 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parallel_bounds_the_files_in_flight() {
    let mut mock = Mock::tree();
    for i in 0..12 {
        mock.files.push((
            format!("data/chunk-001/file-{i:03}.parquet"),
            pattern(500, i),
        ));
    }
    mock.page = 1000;
    let s = server(mock).await;
    let out = Scratch::new("parallel");
    let mut opts = options(&out.path("d"));
    opts.parallel = 2;

    download(&s.client, &ctx(), &opts).await.unwrap();

    let mock = s.mock.lock().unwrap();
    assert_tree_matches(&mock, &out.path("d"));
    assert!(
        mock.max_in_flight <= 2,
        "saw {} at once",
        mock.max_in_flight
    );
}

#[test]
fn the_default_directory_is_named_after_the_dataset() {
    assert_eq!(
        dir_name_for(&Mock::tree().name, ID),
        "g1-bottle-sorting",
        "no separator of the name reaches the path"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_empty_directory_is_refused_before_anything_is_fetched() {
    let s = server(Mock::tree()).await;
    let out = Scratch::new("occupied");
    out.write("notes.txt", b"mine");

    let err = download(&s.client, &ctx(), &options(&out.0))
        .await
        .unwrap_err();

    assert!(matches!(err, RmsError::InvalidInput(_)), "{err}");
    let text = err.to_string();
    assert!(
        text.contains("--resume") && text.contains("--force"),
        "{text}"
    );
    let mock = s.mock.lock().unwrap();
    assert!(mock.log.iter().all(|l| !l.starts_with("GET content")));
    assert!(mock.log.iter().all(|l| !l.starts_with("GET files")));
    assert_eq!(std::fs::read(out.path("notes.txt")).unwrap(), b"mine");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_keeps_complete_files_and_continues_partial_ones() {
    let mock = Mock::tree();
    let video = mock
        .files
        .iter()
        .find(|(p, _)| p == VIDEO)
        .unwrap()
        .1
        .clone();
    let data = mock
        .files
        .iter()
        .find(|(p, _)| p == DATA)
        .unwrap()
        .1
        .clone();
    let s = server(mock).await;
    let out = Scratch::new("resume");
    // An earlier run got this far: one file whole, the video 70 000 bytes in,
    // and one file under its real name with the wrong size.
    out.write("meta/info.json", br#"{"fps":30}"#);
    out.write(&format!("{VIDEO}{PART_SUFFIX}"), &video[..70_000]);
    out.write(DATA, &data[..100]);
    let mut opts = options(&out.0);
    opts.resume = true;

    let outcome = download(&s.client, &ctx(), &opts).await.unwrap();

    let mock = s.mock.lock().unwrap();
    assert_tree_matches(&mock, &out.0);
    assert_eq!(outcome.skipped_files, 1);
    assert!(mock.content_requests("meta/info.json").is_empty());
    assert_eq!(
        mock.content_requests(VIDEO),
        [format!("GET content {VIDEO} bytes=70000-")],
        "only the missing tail of the video is asked for"
    );
    assert_eq!(
        mock.content_requests(DATA),
        [format!("GET content {DATA} -")]
    );
    assert_eq!(outcome.fetched_bytes, 130_000 + 3000 + 400);
    // The resumed video was still verified against its SHA-256, whole.
    assert_eq!(outcome.verified_sha256, 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_in_a_complete_directory_fetches_nothing() {
    let s = server(Mock::tree()).await;
    let out = Scratch::new("again");
    download(&s.client, &ctx(), &options(&out.path("d")))
        .await
        .unwrap();
    s.mock.lock().unwrap().log.clear();
    let mut opts = options(&out.path("d"));
    opts.resume = true;

    let outcome = download(&s.client, &ctx(), &opts).await.unwrap();

    assert_eq!(outcome.skipped_files, 5);
    assert_eq!(outcome.fetched_bytes, 0);
    let mock = s.mock.lock().unwrap();
    assert!(mock.log.iter().all(|l| !l.starts_with("GET content")));
    assert_tree_matches(&mock, &out.path("d"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_fetches_everything_again_over_what_is_there() {
    let s = server(Mock::tree()).await;
    let out = Scratch::new("force");
    out.write("meta/info.json", b"{\"fps\":31}");
    out.write(
        &format!("{VIDEO}{PART_SUFFIX}"),
        b"not a prefix of the video",
    );
    let mut opts = options(&out.0);
    opts.force = true;

    let outcome = download(&s.client, &ctx(), &opts).await.unwrap();

    let mock = s.mock.lock().unwrap();
    assert_tree_matches(&mock, &out.0);
    assert_eq!(outcome.skipped_files, 0);
    assert_eq!(
        mock.content_requests(VIDEO),
        [format!("GET content {VIDEO} -")]
    );
    assert_eq!(mock.content_requests("meta/info.json").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_that_dies_mid_file_continues_where_it_stopped() {
    let mut mock = Mock::tree();
    mock.fault(VIDEO, Fault::DropAfter(50_000));
    mock.fault(VIDEO, Fault::DropAfter(20_000));
    let s = server(mock).await;
    let out = Scratch::new("dropped");
    let mut opts = options(&out.path("d"));
    // Each of those made progress, so none of them uses up an attempt.
    opts.retries = 0;

    let outcome = download(&s.client, &ctx(), &opts).await.unwrap();

    let mock = s.mock.lock().unwrap();
    assert_tree_matches(&mock, &out.path("d"));
    let requests = mock.content_requests(VIDEO);
    assert_eq!(requests.len(), 3, "{requests:?}");
    assert_eq!(requests[0], format!("GET content {VIDEO} -"));
    // Never from zero again: each request starts where the bytes on disk end.
    let from = |line: &String| -> u64 {
        line.rsplit("bytes=")
            .next()
            .unwrap()
            .trim_end_matches('-')
            .parse()
            .unwrap()
    };
    let (second, third) = (from(&requests[1]), from(&requests[2]));
    assert!(second > 0 && second <= 50_000, "{second}");
    assert!(third > second && third <= second + 20_000, "{third}");
    assert_eq!(
        outcome.fetched_bytes, outcome.bytes,
        "no byte was fetched twice"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transient_refusals_are_retried_and_counted() {
    let mut mock = Mock::tree();
    mock.fault(VIDEO, Fault::Refuse(429, "RATE_LIMITED", Some(0)));
    mock.fault(VIDEO, Fault::Refuse(503, "STORAGE_TIMEOUT", Some(0)));
    mock.fault(DATA, Fault::Refuse(502, "", None));
    let s = server(mock).await;
    let out = Scratch::new("transient");

    download(&s.client, &ctx(), &options(&out.path("d")))
        .await
        .unwrap();

    let mock = s.mock.lock().unwrap();
    assert_tree_matches(&mock, &out.path("d"));
    assert_eq!(mock.content_requests(VIDEO).len(), 3);
    assert_eq!(mock.content_requests(DATA).len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_that_keeps_failing_ends_the_run_with_an_error() {
    let mut mock = Mock::tree();
    for _ in 0..10 {
        mock.fault(VIDEO, Fault::Refuse(503, "STORAGE_UNAVAILABLE", Some(0)));
    }
    let s = server(mock).await;
    let out = Scratch::new("exhausted");
    let mut opts = options(&out.path("d"));
    opts.retries = 2;

    let err = download(&s.client, &ctx(), &opts).await.unwrap_err();

    let text = err.to_string();
    assert!(text.contains(VIDEO), "{text}");
    assert!(text.contains("STORAGE_UNAVAILABLE"), "{text}");
    let mock = s.mock.lock().unwrap();
    // One first attempt and two retries.
    assert_eq!(mock.content_requests(VIDEO).len(), 3);
    assert!(
        !out.path("d").join(VIDEO).exists(),
        "no file under its real name"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refusal_that_cannot_improve_stops_at_once() {
    let mut mock = Mock::tree();
    mock.extra_listing.push(("meta/zz-gone.json".into(), 12));
    let s = server(mock).await;
    let out = Scratch::new("fatal");

    let err = download(&s.client, &ctx(), &options(&out.path("d")))
        .await
        .unwrap_err();

    assert!(matches!(err, RmsError::Api { status: 404, .. }), "{err}");
    assert!(err.to_string().contains("FILE_NOT_FOUND"), "{err}");
    let mock = s.mock.lock().unwrap();
    assert_eq!(
        mock.content_requests("meta/zz-gone.json").len(),
        1,
        "a 404 is not retried"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_whose_checksum_is_wrong_is_fetched_again() {
    let mut mock = Mock::tree();
    mock.fault(DATA, Fault::Corrupt);
    let s = server(mock).await;
    let out = Scratch::new("corrupt-once");

    let outcome = download(&s.client, &ctx(), &options(&out.path("d")))
        .await
        .unwrap();

    let mock = s.mock.lock().unwrap();
    assert_tree_matches(&mock, &out.path("d"));
    assert_eq!(
        mock.content_requests(DATA),
        [
            format!("GET content {DATA} -"),
            format!("GET content {DATA} -")
        ],
        "the second fetch starts from the first byte"
    );
    assert_eq!(outcome.verified_sha256, 5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_checksum_that_stays_wrong_is_an_error_and_leaves_no_file() {
    let mut mock = Mock::tree();
    for _ in 0..10 {
        mock.fault(DATA, Fault::Corrupt);
    }
    let s = server(mock).await;
    let out = Scratch::new("corrupt");
    let mut opts = options(&out.path("d"));
    opts.retries = 1;

    let err = download(&s.client, &ctx(), &opts).await.unwrap_err();

    assert!(err.to_string().contains("SHA-256 mismatch"), "{err}");
    assert!(err.to_string().contains(DATA), "{err}");
    assert!(!out.path("d").join(DATA).exists());
    assert!(!out.path("d").join(format!("{DATA}{PART_SUFFIX}")).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_checksum_verifies_sizes_only() {
    let mut mock = Mock::tree();
    mock.fault(DATA, Fault::Corrupt);
    let s = server(mock).await;
    let out = Scratch::new("no-checksum");
    let mut opts = options(&out.path("d"));
    opts.no_checksum = true;

    let outcome = download(&s.client, &ctx(), &opts).await.unwrap();

    assert_eq!(outcome.verified_sha256, 0);
    assert_eq!(s.mock.lock().unwrap().content_requests(DATA).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dataset_without_checksums_is_verified_by_size() {
    // What a Hugging Face import looks like: no SHA-256 in the manifest.
    let mut mock = Mock::tree();
    mock.with_sha = false;
    let s = server(mock).await;
    let out = Scratch::new("no-sha");

    let outcome = download(&s.client, &ctx(), &options(&out.path("d")))
        .await
        .unwrap();

    assert_eq!(outcome.verified_sha256, 0);
    assert_tree_matches(&s.mock.lock().unwrap(), &out.path("d"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_ignores_range_restarts_the_file_instead_of_appending() {
    let mut mock = Mock::tree();
    let video = mock
        .files
        .iter()
        .find(|(p, _)| p == VIDEO)
        .unwrap()
        .1
        .clone();
    mock.fault(VIDEO, Fault::IgnoreRange);
    let s = server(mock).await;
    let out = Scratch::new("ignored-range");
    out.write(&format!("{VIDEO}{PART_SUFFIX}"), &video[..70_000]);
    let mut opts = options(&out.0);
    opts.resume = true;

    download(&s.client, &ctx(), &opts).await.unwrap();

    let mock = s.mock.lock().unwrap();
    assert_tree_matches(&mock, &out.0);
    assert_eq!(
        mock.content_requests(VIDEO),
        [format!("GET content {VIDEO} bytes=70000-")]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_part_file_longer_than_the_file_is_discarded() {
    let mock = Mock::tree();
    let s = server(mock).await;
    let out = Scratch::new("too-long");
    out.write(&format!("{DATA}{PART_SUFFIX}"), &pattern(5000, 9));
    let mut opts = options(&out.0);
    opts.resume = true;

    download(&s.client, &ctx(), &opts).await.unwrap();

    let mock = s.mock.lock().unwrap();
    assert_tree_matches(&mock, &out.0);
    assert_eq!(
        mock.content_requests(DATA),
        [format!("GET content {DATA} -")]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_that_differs_from_its_listing_is_an_error() {
    for listed in [3012u64, 2990] {
        let mut mock = Mock::tree();
        mock.listed_size.insert(DATA.into(), listed);
        mock.with_sha = false;
        let s = server(mock).await;
        let out = Scratch::new("changed");

        let err = download(&s.client, &ctx(), &options(&out.path("d")))
            .await
            .unwrap_err();

        let text = err.to_string();
        assert!(text.contains(DATA), "{text}");
        assert!(text.contains("changed on the server"), "{text}");
        assert!(!out.path("d").join(DATA).exists());
        assert_eq!(
            s.mock.lock().unwrap().content_requests(DATA).len(),
            1,
            "not retried"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_path_listed_twice_is_refused() {
    let mut mock = Mock::tree();
    mock.extra_listing.push((DATA.into(), 3000));
    let s = server(mock).await;
    let out = Scratch::new("twice");
    let err = download(&s.client, &ctx(), &options(&out.path("d")))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("twice"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_path_that_would_leave_the_directory_is_refused_before_any_write() {
    let mut mock = Mock::tree();
    mock.extra_listing.push(("../escaped.txt".into(), 4));
    let s = server(mock).await;
    let out = Scratch::new("escape");
    let target = out.path("inner");

    let err = download(&s.client, &ctx(), &options(&target))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("will not write"), "{err}");
    assert!(!out.path("escaped.txt").exists());
    assert!(
        !target.exists(),
        "nothing was created before the manifest was checked"
    );
    let mock = s.mock.lock().unwrap();
    assert!(mock.log.iter().all(|l| !l.starts_with("GET content")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_dataset_and_an_old_server_say_what_is_wrong() {
    let s = server(Mock::tree()).await;
    let out = Scratch::new("unknown");
    let mut opts = options(&out.path("d"));
    opts.id = "nope".into();
    let err = download(&s.client, &ctx(), &opts).await.unwrap_err();
    assert!(matches!(err, RmsError::Api { status: 404, .. }), "{err}");

    let mut mock = Mock::tree();
    mock.no_files_api = true;
    let s = server(mock).await;
    let err = download(&s.client, &ctx(), &options(&out.path("d")))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no dataset download API"), "{err}");
}
