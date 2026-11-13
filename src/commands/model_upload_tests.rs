//! Tests for `models upload | upload-status | upload-cancel | files | download`.
//!
//! The part engine is tested with the dataset upload it comes from
//! (`dataset_upload_tests.rs`), the download engine with the dataset download
//! (`dataset_download_tests.rs`). These cover what the model commands add:
//! which files of a checkpoint directory go, how metrics are read, the shape
//! of the start request, the registry as the verdict, and that the shared
//! engines are pointed at the model routes — all against a hand-written
//! HTTP/1.1 server that speaks the model contract, because the bugs live in
//! paths, headers and byte ranges.

use super::*;
use crate::commands::dataset_download::{self, DownloadOptions};
use crate::commands::dataset_upload::{hex, part_range};
use crate::config::RmsConfig;
use sha2::Digest;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// ── Pure pieces ─────────────────────────────────────────────────────────

#[test]
fn exclude_patterns_follow_gitignore_habits() {
    // No slash: a file or directory of that name at any depth.
    assert!(excluded("*.log", "train_202492.log"));
    assert!(excluded("*.log", "wandb/run-1/debug.log"));
    assert!(excluded("wandb", "wandb/run-1/debug.log"));
    assert!(excluded("wandb/", "wandb/run-1/debug.log"));
    assert!(!excluded("wandb", "wandb_config.json"));
    assert!(excluded(
        "checkpoint-*",
        "checkpoint-1000/model.safetensors"
    ));
    assert!(!excluded("*.log", "config.json"));
    // With a slash: anchored at the checkpoint directory.
    assert!(excluded(
        "experiment_cfg/*.yaml",
        "experiment_cfg/conf.yaml"
    ));
    assert!(!excluded(
        "experiment_cfg/*.yaml",
        "other/experiment_cfg/conf.yaml"
    ));
    assert!(!excluded(
        "experiment_cfg/*.yaml",
        "experiment_cfg/sub/conf.yaml"
    ));
    assert!(excluded(
        "checkpoint-*/optimizer.pt",
        "checkpoint-500/optimizer.pt"
    ));
    assert!(!excluded(
        "checkpoint-*/optimizer.pt",
        "checkpoint-500/model.safetensors"
    ));
    // `**` crosses directories; a matched directory takes everything under it.
    assert!(excluded("**/optimizer.pt", "a/b/optimizer.pt"));
    assert!(excluded("**/optimizer.pt", "optimizer.pt"));
    assert!(excluded("runs/old", "runs/old/deep/file.bin"));
    assert!(excluded("./processor/", "processor/x.json"));
    // `?` is one character; an empty pattern excludes nothing.
    assert!(excluded(
        "model-0000?-of-*",
        "model-00001-of-00002.safetensors"
    ));
    assert!(!excluded("", "config.json"));
    assert!(!excluded("  ", "config.json"));
}

#[test]
fn metrics_read_numbers_as_numbers_and_merge_files() {
    let dir = Scratch::new("metrics");
    dir.write("m.json", br#"{"final_loss": 0.03, "trainer": "gr00t"}"#);
    let file = dir.path("m.json").display().to_string();
    let m = parse_metrics(
        "--metrics",
        &[
            file,
            "final_loss=0.0264".into(),
            "steps=5000".into(),
            "learning_rate=1e-4".into(),
            "frozen=true".into(),
            "eval=26/50".into(),
            "note = a=b ".into(),
        ],
    )
    .unwrap();
    // A later value wins over the file's.
    assert_eq!(m["final_loss"], json!(0.0264));
    assert_eq!(m["steps"], json!(5000));
    assert_eq!(m["learning_rate"], json!(0.0001));
    assert_eq!(m["frozen"], json!(true));
    assert_eq!(m["eval"], json!("26/50"));
    assert_eq!(m["trainer"], json!("gr00t"));
    assert_eq!(m["note"], json!("a=b"));
    assert!(parse_metrics("--metrics", &[]).unwrap().is_empty());
}

#[test]
fn metrics_that_cannot_be_read_say_why() {
    let dir = Scratch::new("metrics-bad");
    dir.write("list.json", b"[1,2]");
    dir.write("broken.json", b"{");
    let err = |v: String| parse_metrics("--metrics", &[v]).unwrap_err().to_string();
    assert!(err("loss".into()).contains("key=value or the path of a JSON file"));
    assert!(err("=1".into()).contains("the key is empty"));
    assert!(err(dir.path("list.json").display().to_string()).contains("must hold a JSON object"));
    assert!(err(dir.path("broken.json").display().to_string()).contains("not valid JSON"));
}

// ── Scratch directories ─────────────────────────────────────────────────

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rmsctl-model-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
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
        .map(|i| {
            (i as u8)
                .wrapping_mul(31)
                .wrapping_add(seed)
                .wrapping_add((i >> 8) as u8)
        })
        .collect()
}

const CHUNK: u64 = 1000;
const SHARD: &str = "model-00001-of-00002.safetensors";

/// A checkpoint in miniature: a shard of several parts, small files, a nested
/// one with a space in its name, an empty one, and a wandb directory.
fn checkpoint(tag: &str) -> Scratch {
    let dir = Scratch::new(tag);
    dir.write("config.json", br#"{"model_type":"Gr00tN1d7"}"#);
    dir.write(SHARD, &pattern(3500, 1));
    dir.write("model-00002-of-00002.safetensors", &pattern(1000, 2));
    dir.write("processor/processor config.json", &pattern(300, 3));
    dir.write("experiment_cfg/empty.yaml", b"");
    dir.write("train_1.log", &pattern(50, 4));
    dir.write("wandb/run-1/debug.log", b"junk");
    dir
}

/// Bytes of the checkpoint without `wandb/`.
const CHECKPOINT_BYTES: u64 = 26 + 3500 + 1000 + 300 + 50;

#[test]
fn scan_takes_every_file_and_leaves_out_what_is_excluded() {
    let dir = checkpoint("scan");
    let all = scan_dir(&dir.0, &[]).unwrap();
    assert_eq!(all.files.len(), 7);
    assert_eq!(all.excluded, 0);
    // Sorted, forward slashes, relative to the checkpoint directory.
    assert_eq!(all.files[0].rel, "config.json");
    assert!(
        all.files
            .iter()
            .any(|f| f.rel == "processor/processor config.json")
    );

    let some = scan_dir(&dir.0, &["wandb".into(), "*.log".into()]).unwrap();
    assert_eq!(some.files.len(), 5);
    assert_eq!(some.excluded, 2);
    assert!(some.files.iter().all(|f| !f.rel.ends_with(".log")));
    assert_eq!(some.total_bytes(), CHECKPOINT_BYTES - 50);
}

#[cfg(unix)]
#[test]
fn scan_follows_symlinks_like_a_hub_cache_snapshot() {
    let blobs = Scratch::new("blobs");
    blobs.write("abc", &pattern(700, 9));
    let dir = Scratch::new("links");
    dir.write("config.json", b"{}");
    std::os::unix::fs::symlink(blobs.path("abc"), dir.path("model.safetensors")).unwrap();
    let scan = scan_dir(&dir.0, &[]).unwrap();
    let linked = scan
        .files
        .iter()
        .find(|f| f.rel == "model.safetensors")
        .unwrap();
    assert_eq!(linked.size, 700);
}

// ── A server that speaks the model contract ─────────────────────────────

const UP: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

struct MockFile {
    path: String,
    size: u64,
    sha256: Option<String>,
}

#[derive(Default)]
struct Mock {
    files: Vec<MockFile>,
    parts: HashMap<(u64, u32), Vec<u8>>,
    /// "" until an upload exists.
    state: String,
    start_body: Option<Value>,
    /// Status polls answered `processing` before the verdict.
    polls_before_verdict: u32,
    /// The verdict is `failed` with this reason instead of `completed`.
    fail_with: Option<String>,
    /// Refuse the n-th part request (counted from 1) with a 400 nothing can fix.
    refuse_put_number: Option<usize>,
    /// Every request: `METHOD path?query [range]`.
    log: Vec<String>,
}

type Reply = (u16, Vec<(String, String)>, Vec<u8>);

fn json_reply(status: u16, body: Value) -> Reply {
    (
        status,
        vec![("content-type".into(), "application/json".into())],
        body.to_string().into_bytes(),
    )
}

/// The value of `key` in a form-encoded query string, decoded.
fn query_value(query: &str, key: &str) -> Option<String> {
    let raw = query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{key}=")))?;
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 3 <= bytes.len() => {
                out.push(u8::from_str_radix(&raw[i + 1..i + 3], 16).ok()?);
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

impl Mock {
    fn parts_of(&self, file: u64) -> u32 {
        part_count(self.files[file as usize].size, CHUNK)
    }

    fn complete(&self) -> bool {
        (0..self.files.len() as u64)
            .all(|f| (1..=self.parts_of(f)).all(|n| self.parts.contains_key(&(f, n))))
    }

    fn assembled(&self, file: u64) -> Vec<u8> {
        (1..=self.parts_of(file))
            .flat_map(|n| self.parts[&(file, n)].clone())
            .collect()
    }

    fn stored(&self, path: &str) -> Option<Vec<u8>> {
        let index = self.files.iter().position(|f| f.path == path)?;
        Some(self.assembled(index as u64))
    }

    fn count(&self, prefix: &str) -> usize {
        self.log.iter().filter(|l| l.starts_with(prefix)).count()
    }

    fn status_body(&self) -> Value {
        let uploading = self.state == "uploading";
        let files: Vec<Value> = self
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let parts = self.parts_of(i as u64);
                let held: Vec<u32> = (1..=parts)
                    .filter(|n| self.parts.contains_key(&(i as u64, *n)))
                    .collect();
                let complete = if uploading {
                    parts == 1 && held.len() == 1
                } else {
                    true
                };
                json!({ "id": i, "path": f.path, "size": f.size, "partCount": parts,
                        "complete": complete, "receivedParts": held })
            })
            .collect();
        let start = self.start_body.clone().unwrap_or(Value::Null);
        json!({
            "uploadId": UP, "state": self.state, "name": start["name"], "version": start["version"],
            "baseModel": start["baseModel"], "storage": "rustfs", "chunkSize": CHUNK,
            "totalBytes": self.files.iter().map(|f| f.size).sum::<u64>(),
            "receivedBytes": self.parts.values().map(|p| p.len()).sum::<usize>(),
            "totalFiles": self.files.len(),
            "completedFiles": files.iter().filter(|f| f["complete"] == true).count(),
            "readyToFinish": uploading && self.complete(), "files": files,
            "modelVersionId": if self.state == "completed" { json!(UP) } else { Value::Null },
            "error": if self.state == "failed" { json!(self.fail_with) } else { Value::Null },
            "createdAt": "2026-11-13T10:00:00.000Z", "lastActivityAt": "2026-11-13T10:00:00.000Z",
            "expiresAt": if uploading { json!("2026-11-13T16:00:00.000Z") } else { Value::Null },
        })
    }

    fn handle(
        &mut self,
        method: &str,
        target: &str,
        headers: &HashMap<String, String>,
        body: &[u8],
    ) -> Reply {
        let range = headers
            .get("range")
            .map(|r| format!(" {r}"))
            .unwrap_or_default();
        self.log.push(format!("{method} {target}{range}"));
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let exists = !self.state.is_empty() && self.state != "cancelled";
        let upload = format!("/api/models/uploads/{UP}");
        let files_root = format!("/api/models/versions/{UP}/files");
        let not_found = || {
            json_reply(
                404,
                json!({ "error": "Model upload not found", "code": "NOT_FOUND" }),
            )
        };

        match (method, path) {
            ("POST", "/api/models/uploads") => {
                let declared: Value = serde_json::from_slice(body).unwrap();
                self.files = declared["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|f| MockFile {
                        path: f["path"].as_str().unwrap().to_string(),
                        size: f["size"].as_u64().unwrap(),
                        sha256: f["sha256"].as_str().map(String::from),
                    })
                    .collect();
                self.parts.clear();
                self.state = "uploading".into();
                self.start_body = Some(declared);
                let files = self.status_body()["files"].clone();
                json_reply(
                    201,
                    json!({ "uploadId": UP, "state": "uploading", "storage": "rustfs",
                            "chunkSize": CHUNK, "files": files }),
                )
            }
            ("GET", "/api/models/uploads") => {
                let uploads = if exists && self.state != "completed" {
                    vec![self.status_body()]
                } else {
                    vec![]
                };
                json_reply(200, json!({ "uploads": uploads }))
            }
            ("GET", p) if p == upload => {
                if !exists {
                    return not_found();
                }
                if self.state == "processing" {
                    if self.polls_before_verdict > 0 {
                        self.polls_before_verdict -= 1;
                    } else {
                        self.state = if self.fail_with.is_some() {
                            "failed"
                        } else {
                            "completed"
                        }
                        .into();
                    }
                }
                json_reply(200, self.status_body())
            }
            ("PUT", p) if p.starts_with(&format!("{upload}/files/")) => {
                if !exists {
                    return not_found();
                }
                if self.state != "uploading" {
                    return json_reply(
                        409,
                        json!({ "error": "closed", "code": "UPLOAD_NOT_ACCEPTING" }),
                    );
                }
                let puts = self.count("PUT ");
                if self.refuse_put_number == Some(puts) {
                    return json_reply(
                        400,
                        json!({ "error": "refused for the test", "code": "PART_SIZE_MISMATCH" }),
                    );
                }
                let rest: Vec<&str> = p[upload.len()..].split('/').collect();
                let (file, part): (u64, u32) = (rest[2].parse().unwrap(), rest[4].parse().unwrap());
                assert_eq!(
                    headers.get("content-type").map(String::as_str),
                    Some("application/octet-stream")
                );
                let sha = hex(&sha2::Sha256::digest(body));
                assert_eq!(
                    headers.get("x-chunk-sha256"),
                    Some(&sha),
                    "every part carries its checksum"
                );
                self.parts.insert((file, part), body.to_vec());
                json_reply(
                    200,
                    json!({ "uploadId": UP, "fileId": file, "partNumber": part,
                            "size": body.len(), "sha256": sha }),
                )
            }
            ("POST", p) if p == format!("{upload}/finish") => {
                if !exists {
                    return not_found();
                }
                if !self.complete() {
                    return json_reply(
                        409,
                        json!({ "error": "incomplete", "code": "UPLOAD_INCOMPLETE" }),
                    );
                }
                self.state = "processing".into();
                json_reply(
                    202,
                    json!({ "uploadId": UP, "state": "processing", "modelVersionId": null }),
                )
            }
            ("POST", p) if p == format!("{upload}/cancel") => {
                if !exists {
                    return not_found();
                }
                if self.state == "completed" {
                    return json_reply(
                        409,
                        json!({ "error": "registered", "code": "UPLOAD_COMPLETED" }),
                    );
                }
                self.state = "cancelled".into();
                self.parts.clear();
                json_reply(200, json!({ "uploadId": UP, "status": "cancelled" }))
            }
            ("GET", p) if p == format!("/api/models/versions/{UP}") => json_reply(
                200,
                json!({ "modelVersion": { "id": UP, "name": "synth", "version": "v1",
                        "sourceKind": "uploaded", "deploymentStatus": "staging" } }),
            ),
            ("GET", p) if p == files_root => {
                if self.state != "completed" {
                    return json_reply(
                        404,
                        json!({ "error": "not uploaded here", "code": "NO_STORED_FILES" }),
                    );
                }
                let mut files: Vec<Value> = self
                    .files
                    .iter()
                    .map(|f| json!({ "path": f.path, "size": f.size, "sha256": f.sha256 }))
                    .collect();
                files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
                json_reply(
                    200,
                    json!({
                        "modelVersionId": UP,
                        "artifactUri": format!("s3://production-models/uploads/{UP}/"),
                        "storage": "rustfs",
                        "totalBytes": self.files.iter().map(|f| f.size).sum::<u64>(),
                        "fileCount": files.len(), "limit": 1000, "files": files,
                        "nextCursor": null,
                    }),
                )
            }
            ("GET", p) if p == format!("{files_root}/content") => {
                let rel = query_value(query, "path").expect("a file is asked for by ?path=");
                let Some(data) = self.stored(&rel) else {
                    return json_reply(
                        404,
                        json!({ "error": "no such file", "code": "FILE_NOT_FOUND" }),
                    );
                };
                match headers.get("range") {
                    None => (200, vec![], data),
                    Some(spec) => {
                        // The download asks for the rest of a file: `bytes=N-`.
                        let start: usize = spec
                            .trim_start_matches("bytes=")
                            .trim_end_matches('-')
                            .parse()
                            .unwrap();
                        let range = format!("bytes {start}-{}/{}", data.len() - 1, data.len());
                        (
                            206,
                            vec![("content-range".into(), range)],
                            data[start..].to_vec(),
                        )
                    }
                }
            }
            _ => json_reply(404, json!({ "error": format!("no route {method} {path}") })),
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

async fn serve_connection(mut sock: TcpStream, mock: Arc<Mutex<Mock>>) {
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
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap_or("").split(' ');
        let method = first.next().unwrap_or("").to_string();
        let path = first.next().unwrap_or("").to_string();
        let headers: HashMap<String, String> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        assert!(
            !headers.contains_key("transfer-encoding"),
            "{path}: a part must carry Content-Length"
        );
        let length: usize = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = buf.split_off(head_end + 4);
        buf.clear();
        while body.len() < length {
            match sock.read(&mut tmp).await {
                Ok(n) if n > 0 => body.extend_from_slice(&tmp[..n]),
                _ => return,
            }
        }
        buf = body.split_off(length);
        let (status, extra, reply) = mock.lock().unwrap().handle(&method, &path, &headers, &body);
        let extra: String = extra.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
        let head = format!(
            "HTTP/1.1 {status} X\r\n{extra}content-length: {}\r\n\r\n",
            reply.len()
        );
        if sock.write_all(head.as_bytes()).await.is_err() || sock.write_all(&reply).await.is_err() {
            return;
        }
    }
}

struct Server {
    mock: Arc<Mutex<Mock>>,
    client: ApiClient,
    state: Scratch,
}

async fn server(mock: Mock) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mock = Arc::new(Mutex::new(mock));
    let shared = mock.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(serve_connection(sock, shared.clone()));
        }
    });
    let client = ApiClient::new(&RmsConfig {
        server_url: format!("http://{addr}/api"),
        token: Some("test-token".into()),
    });
    Server {
        mock,
        client,
        state: Scratch::new("notes"),
    }
}

/// `--json` doubles as the quiet mode, which keeps the test output readable.
fn ctx(yes: bool) -> Ctx {
    Ctx {
        json: true,
        yes,
        page: None,
        limit: None,
    }
}

fn tuning() -> Tuning {
    let ms = Duration::from_millis;
    Tuning {
        backoff_base: ms(1),
        max_pause: ms(5),
        poll_interval: ms(2),
    }
}

fn options(dir: &Path, state: &Scratch) -> ModelUploadOptions {
    ModelUploadOptions {
        dir: dir.to_path_buf(),
        name: Some("synth".into()),
        version: "v1".into(),
        model_type: None,
        base_model: Some("groot_n1_7".into()),
        dataset: Some("ds-apple".into()),
        metrics: vec!["final_loss=0.0264".into()],
        eval_metrics: vec!["success_rate=0.52".into()],
        notes: Some("5000 steps".into()),
        exclude: vec!["wandb".into()],
        chunk_size: None,
        parallel: 3,
        retries: 3,
        resume: None,
        restart: false,
        no_checksum: false,
        no_wait: false,
        wait_timeout: Duration::from_secs(20),
        part_timeout: Duration::from_secs(20),
        state_dir: Some(state.0.clone()),
        tuning: tuning(),
    }
}

fn download_options(out: &Path) -> DownloadOptions {
    DownloadOptions {
        kind: dataset_download::Kind::Model,
        id: UP.into(),
        out: Some(out.to_path_buf()),
        parallel: 2,
        retries: 3,
        resume: false,
        force: false,
        no_checksum: false,
        stall_timeout: Duration::from_secs(10),
        tuning: tuning(),
    }
}

/// A mock that already holds `dir` as a registered model.
fn registered(dir: &Scratch) -> Mock {
    let scan = scan_dir(&dir.0, &["wandb".into()]).unwrap();
    let mut mock = Mock {
        state: "completed".into(),
        ..Default::default()
    };
    for (i, f) in scan.files.iter().enumerate() {
        let data = std::fs::read(&f.abs).unwrap();
        mock.files.push(MockFile {
            path: f.rel.clone(),
            size: f.size,
            sha256: Some(hex(&sha2::Sha256::digest(&data))),
        });
        for n in 1..=part_count(f.size, CHUNK) {
            let (offset, len) = part_range(f.size, CHUNK, n);
            mock.parts.insert(
                (i as u64, n),
                data[offset as usize..(offset + len) as usize].to_vec(),
            );
        }
    }
    mock
}

fn note(srv: &Server, dir: &Scratch) -> StateStore {
    StateStore::new(
        Some(srv.state.0.clone()),
        &note_server(&srv.client),
        &std::fs::canonicalize(&dir.0).unwrap(),
    )
}

fn assert_same_tree(dir: &Scratch, out: &Path) {
    let scan = scan_dir(&dir.0, &["wandb".into()]).unwrap();
    for f in &scan.files {
        assert_eq!(
            std::fs::read(&f.abs).unwrap(),
            std::fs::read(out.join(&f.rel)).unwrap(),
            "{}",
            f.rel
        );
    }
    let fetched = scan_dir(out, &[]).unwrap();
    assert_eq!(
        fetched.files.len(),
        scan.files.len(),
        "nothing extra, no part file left"
    );
}

// ── upload ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn uploads_a_checkpoint_and_waits_for_the_registry() {
    let dir = checkpoint("flow");
    let srv = server(Mock {
        polls_before_verdict: 2,
        ..Default::default()
    })
    .await;
    let outcome = upload(&srv.client, &ctx(true), &options(&dir.0, &srv.state))
        .await
        .unwrap();

    assert_eq!(outcome.upload_id, UP);
    assert_eq!(outcome.state, "completed");
    assert_eq!(outcome.model_version_id.as_deref(), Some(UP));
    assert!(outcome.executed);
    // 6 files: the shard in 4 parts, every other file (the empty one too) in 1.
    assert_eq!(outcome.parts_sent, 9);
    assert_eq!(outcome.bytes_sent, CHECKPOINT_BYTES);

    {
        let mock = srv.mock.lock().unwrap();
        let start = mock.start_body.as_ref().unwrap();
        assert_eq!(start["name"], "synth");
        assert_eq!(start["version"], "v1");
        assert_eq!(start["baseModel"], "groot_n1_7");
        assert_eq!(start["datasetId"], "ds-apple");
        assert_eq!(start["trainingMetrics"], json!({ "final_loss": 0.0264 }));
        assert_eq!(start["validationMetrics"], json!({ "success_rate": 0.52 }));
        assert_eq!(start["notes"], "5000 steps");
        assert!(
            start.get("modelType").is_none(),
            "left to the server's default"
        );
        let declared = start["files"].as_array().unwrap();
        assert_eq!(declared.len(), 6);
        assert!(
            !declared
                .iter()
                .any(|f| f["path"].as_str().unwrap().starts_with("wandb/")),
            "excluded files are not declared"
        );
        assert!(
            declared
                .iter()
                .all(|f| f["sha256"].as_str().is_some_and(|s| s.len() == 64))
        );
        // What the server holds is what is on disk.
        for (i, f) in mock.files.iter().enumerate() {
            assert_eq!(
                mock.assembled(i as u64),
                std::fs::read(dir.path(&f.path)).unwrap(),
                "{}",
                f.path
            );
        }
        assert_eq!(
            mock.count("POST /api/models/uploads"),
            2,
            "one start, one finish"
        );
    }
    // The note has done its job.
    assert!(note(&srv, &dir).load().is_none());
}

#[tokio::test]
async fn without_yes_nothing_is_sent() {
    let dir = checkpoint("gate");
    let srv = server(Mock::default()).await;
    let outcome = upload(&srv.client, &ctx(false), &options(&dir.0, &srv.state))
        .await
        .unwrap();
    assert!(!outcome.executed);
    assert!(srv.mock.lock().unwrap().log.is_empty());
}

#[tokio::test]
async fn running_again_sends_only_what_the_server_lacks() {
    let dir = checkpoint("resume");
    let srv = server(Mock {
        refuse_put_number: Some(4),
        ..Default::default()
    })
    .await;
    let mut opts = options(&dir.0, &srv.state);
    opts.parallel = 1;

    let err = upload(&srv.client, &ctx(true), &opts).await.unwrap_err();
    assert!(err.to_string().contains("PART_SIZE_MISMATCH"), "{err}");
    let held = srv.mock.lock().unwrap().parts.len();
    assert_eq!(held, 3, "three parts were stored before the refusal");

    // Same command again: no second start, only the six missing parts.
    {
        let mut m = srv.mock.lock().unwrap();
        m.refuse_put_number = None;
        m.log.clear();
    }
    let outcome = upload(&srv.client, &ctx(true), &opts).await.unwrap();
    assert_eq!(outcome.state, "completed");
    assert_eq!(outcome.parts_sent, 6);
    let mock = srv.mock.lock().unwrap();
    assert_eq!(mock.count("PUT "), 6);
    assert!(
        !mock.log.iter().any(|l| l == "POST /api/models/uploads"),
        "an upload is never started twice"
    );
}

#[tokio::test]
async fn a_changed_directory_cannot_be_resumed_into_the_upload() {
    let dir = checkpoint("changed");
    let srv = server(Mock {
        refuse_put_number: Some(2),
        ..Default::default()
    })
    .await;
    let opts = options(&dir.0, &srv.state);
    upload(&srv.client, &ctx(true), &opts).await.unwrap_err();
    srv.mock.lock().unwrap().refuse_put_number = None;

    dir.write(SHARD, &pattern(3600, 1));
    let err = upload(&srv.client, &ctx(true), &opts)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("no longer matches the upload"), "{err}");
}

#[tokio::test]
async fn a_failure_on_the_server_registers_nothing_and_says_why() {
    let dir = checkpoint("failed");
    let srv = server(Mock {
        fail_with: Some("Checksum mismatch for config.json".into()),
        ..Default::default()
    })
    .await;
    let err = upload(&srv.client, &ctx(true), &options(&dir.0, &srv.state))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("no model was registered"), "{err}");
    assert!(err.contains("Checksum mismatch for config.json"), "{err}");
    assert!(
        err.contains(&format!("models upload-cancel {UP} --yes")),
        "{err}"
    );
    let mock = srv.mock.lock().unwrap();
    assert!(!mock.log.iter().any(|l| l.contains("/models/versions/")));
}

#[tokio::test]
async fn no_wait_returns_after_finish_and_no_checksum_declares_no_hashes() {
    let dir = checkpoint("nowait");
    let srv = server(Mock {
        polls_before_verdict: 1000,
        ..Default::default()
    })
    .await;
    let mut opts = options(&dir.0, &srv.state);
    opts.no_wait = true;
    opts.no_checksum = true;
    let outcome = upload(&srv.client, &ctx(true), &opts).await.unwrap();
    assert_eq!(outcome.state, "processing");
    let mock = srv.mock.lock().unwrap();
    let declared = mock.start_body.as_ref().unwrap()["files"]
        .as_array()
        .unwrap();
    assert!(declared.iter().all(|f| f.get("sha256").is_none()));
}

#[tokio::test]
async fn bad_input_is_refused_before_any_request() {
    let dir = checkpoint("input");
    let srv = server(Mock::default()).await;
    let refused = async |opts: ModelUploadOptions| {
        upload(&srv.client, &ctx(true), &opts)
            .await
            .unwrap_err()
            .to_string()
    };

    let mut opts = options(&dir.0, &srv.state);
    opts.version = "  ".into();
    assert!(refused(opts).await.contains("--version is required"));

    let mut opts = options(&dir.0, &srv.state);
    opts.chunk_size = Some(1024);
    assert!(refused(opts).await.contains("at least 5 MiB"));

    let mut opts = options(&dir.0, &srv.state);
    opts.metrics = vec!["loss".into()];
    assert!(refused(opts).await.contains("key=value"));

    let mut opts = options(&dir.0, &srv.state);
    opts.exclude = vec!["*".into()];
    let all_excluded = refused(opts).await;
    assert!(
        all_excluded.contains("every file matched an --exclude"),
        "{all_excluded}"
    );

    assert!(srv.mock.lock().unwrap().log.is_empty());
}

// ── upload-status, upload-cancel ────────────────────────────────────────

#[tokio::test]
async fn status_reads_one_upload_and_lists_the_open_ones() {
    let dir = checkpoint("status");
    let srv = server(Mock {
        refuse_put_number: Some(3),
        ..Default::default()
    })
    .await;
    upload(&srv.client, &ctx(true), &options(&dir.0, &srv.state))
        .await
        .unwrap_err();
    srv.mock.lock().unwrap().log.clear();

    status(&srv.client, &ctx(true), Some(UP)).await.unwrap();
    status(&srv.client, &ctx(true), None).await.unwrap();
    let mock = srv.mock.lock().unwrap();
    assert_eq!(
        mock.log,
        vec![
            format!("GET /api/models/uploads/{UP}"),
            "GET /api/models/uploads".to_string()
        ]
    );
}

#[tokio::test]
async fn cancel_is_gated_then_removes_the_upload_and_its_note() {
    let dir = checkpoint("cancel");
    let srv = server(Mock {
        refuse_put_number: Some(3),
        ..Default::default()
    })
    .await;
    let opts = options(&dir.0, &srv.state);
    upload(&srv.client, &ctx(true), &opts).await.unwrap_err();
    assert_eq!(note(&srv, &dir).load().unwrap().dataset_id, UP);

    cancel(&srv.client, &ctx(false), UP, opts.state_dir.clone())
        .await
        .unwrap();
    assert_eq!(
        srv.mock.lock().unwrap().state,
        "uploading",
        "without --yes nothing is cancelled"
    );

    cancel(&srv.client, &ctx(true), UP, opts.state_dir.clone())
        .await
        .unwrap();
    assert_eq!(srv.mock.lock().unwrap().state, "cancelled");
    assert!(note(&srv, &dir).load().is_none());

    let err = cancel(&srv.client, &ctx(true), UP, opts.state_dir.clone())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("no model upload"), "{err}");
}

#[tokio::test]
async fn cancel_refuses_an_upload_that_became_a_model_and_names_the_delete() {
    let dir = checkpoint("cancel-done");
    let srv = server(registered(&dir)).await;
    let err = cancel(&srv.client, &ctx(true), UP, Some(srv.state.0.clone()))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("registered model now"), "{err}");
    assert!(
        err.contains(&format!("rmsctl models delete {UP} --yes")),
        "{err}"
    );
    let mock = srv.mock.lock().unwrap();
    assert!(!mock.log.iter().any(|l| l.ends_with("/cancel")));
}

// ── files, download ─────────────────────────────────────────────────────

#[tokio::test]
async fn files_reads_the_models_manifest() {
    let dir = checkpoint("files");
    let srv = server(registered(&dir)).await;
    // --json passes the server's first page through; the table reads every page.
    files(&srv.client, &ctx(true), UP).await.unwrap();
    let text = Ctx {
        json: false,
        yes: true,
        page: None,
        limit: None,
    };
    files(&srv.client, &text, UP).await.unwrap();
    let mock = srv.mock.lock().unwrap();
    assert_eq!(
        mock.log,
        vec![
            format!("GET /api/models/versions/{UP}/files"),
            format!("GET /api/models/versions/{UP}/files?limit=1000"),
        ]
    );
}

#[tokio::test]
async fn download_fetches_a_model_through_the_model_routes_and_verifies_it() {
    let dir = checkpoint("dl");
    let srv = server(registered(&dir)).await;
    let out = Scratch::new("dl-out");
    let outcome = dataset_download::download(&srv.client, &ctx(true), &download_options(&out.0))
        .await
        .unwrap();

    assert_eq!(outcome.dataset_id, UP);
    assert_eq!(outcome.files, 6);
    assert_eq!(outcome.fetched_bytes, CHECKPOINT_BYTES);
    // `models upload` declares a SHA-256 for every file, so every file is compared.
    assert_eq!(outcome.verified_sha256, 6);
    assert_same_tree(&dir, &out.0);

    let mock = srv.mock.lock().unwrap();
    let content = format!("GET /api/models/versions/{UP}/files/content?path=");
    assert!(mock.log.contains(&format!("GET /api/models/versions/{UP}")));
    assert!(
        mock.log
            .contains(&format!("GET /api/models/versions/{UP}/files?limit=1000"))
    );
    assert!(
        mock.log
            .iter()
            .any(|l| l.starts_with(&format!("{content}processor"))),
        "{:?}",
        mock.log
    );
    assert!(
        !mock.log.iter().any(|l| l.contains("/datasets/")),
        "nothing is asked of the dataset routes"
    );
}

#[tokio::test]
async fn download_resume_keeps_complete_files_and_continues_a_part() {
    let dir = checkpoint("dl-resume");
    let srv = server(registered(&dir)).await;
    let out = Scratch::new("dl-resume-out");
    // One file complete, and the first 1500 bytes of the shard in its part file.
    out.write(
        "config.json",
        &std::fs::read(dir.path("config.json")).unwrap(),
    );
    out.write(
        &format!("{SHARD}{}", dataset_download::PART_SUFFIX),
        &std::fs::read(dir.path(SHARD)).unwrap()[..1500],
    );

    // A non-empty directory is refused without --resume, naming the model.
    let err = dataset_download::download(&srv.client, &ctx(true), &download_options(&out.0))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("earlier download of this model with --resume"),
        "{err}"
    );

    let mut opts = download_options(&out.0);
    opts.resume = true;
    let outcome = dataset_download::download(&srv.client, &ctx(true), &opts)
        .await
        .unwrap();
    assert_eq!(outcome.skipped_files, 1);
    assert_eq!(outcome.fetched_bytes, CHECKPOINT_BYTES - 26 - 1500);
    assert_same_tree(&dir, &out.0);
    let mock = srv.mock.lock().unwrap();
    assert!(
        !mock
            .log
            .iter()
            .any(|l| l.contains("content?path=config.json"))
    );
    let shard = mock.log.iter().find(|l| l.contains("model-00001")).unwrap();
    assert!(shard.ends_with("bytes=1500-"), "{shard}");
}

#[tokio::test]
async fn a_model_that_was_not_uploaded_here_has_nothing_to_download() {
    let dir = checkpoint("dl-none");
    // An upload that never finished: nothing is stored under a model version.
    let srv = server(Mock {
        state: "uploading".into(),
        ..registered(&dir)
    })
    .await;
    let out = Scratch::new("dl-none-out");
    let err = dataset_download::download(&srv.client, &ctx(true), &download_options(&out.0))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("NO_STORED_FILES"), "{err}");
}
