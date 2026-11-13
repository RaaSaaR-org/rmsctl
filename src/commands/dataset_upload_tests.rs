//! Tests for the direct dataset upload.
//!
//! The pure parts (file selection, part arithmetic, retry classification,
//! resume planning) are tested directly. The flow is tested against a small
//! in-process HTTP server that implements the upload contract and can be told
//! to misbehave, so "resume sends only what is missing" and "a busy server is
//! waited out" are checked on real requests and real bytes.

use super::*;
use crate::config::RmsConfig;
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

// ── Pure helpers ────────────────────────────────────────────────────────

#[test]
fn only_files_of_a_lerobot_tree_are_selected() {
    for yes in [
        "meta/info.json",
        "meta/stats.json",
        "meta/episodes/chunk-000/file-000.parquet",
        "data/chunk-000/file-000.parquet",
        "videos/observation.images.head/chunk-000/file-000.mp4",
        "images/observation.images.head/episode_000000/frame_000000.png",
    ] {
        assert!(is_dataset_file(yes), "{yes} should be uploaded");
    }
    for no in [
        "README.md",
        ".gitattributes",
        "data/chunk-000/file-000.csv",
        "data/notes.txt",
        "videos/observation.images.head/chunk-000/file-000.mkv",
        "videos/poster.png",
        "metadata/info.json",
        "meta",
    ] {
        assert!(!is_dataset_file(no), "{no} should be left out");
    }
}

#[test]
fn part_arithmetic_matches_the_contract() {
    // partCount = max(1, ceil(size / chunkSize))
    assert_eq!(part_count(0, 8), 1);
    assert_eq!(part_count(1, 8), 1);
    assert_eq!(part_count(8, 8), 1);
    assert_eq!(part_count(9, 8), 2);
    assert_eq!(part_count(16, 8), 2);
    assert_eq!(part_count(17, 8), 3);
    // the real thing: a 210 MB video in 32 MiB parts
    assert_eq!(part_count(210_000_000, 32 * 1024 * 1024), 7);

    // part n is [(n-1)*chunk, min(n*chunk, size))
    assert_eq!(part_range(20, 8, 1), (0, 8));
    assert_eq!(part_range(20, 8, 2), (8, 8));
    assert_eq!(part_range(20, 8, 3), (16, 4));
    assert_eq!(part_range(16, 8, 2), (8, 8));
    // an empty file has one part of 0 bytes
    assert_eq!(part_range(0, 8, 1), (0, 0));

    // the parts of a file tile it exactly
    for size in [0u64, 1, 7, 8, 9, 63, 64, 65, 1000] {
        let mut next = 0;
        for n in 1..=part_count(size, 8) {
            let (offset, len) = part_range(size, 8, n);
            assert_eq!(offset, next, "size {size} part {n}");
            next += len;
        }
        assert_eq!(next, size);
    }
}

#[test]
fn chunk_size_accepts_bytes_and_binary_suffixes() {
    assert_eq!(parse_size("33554432"), Ok(33_554_432));
    assert_eq!(parse_size("32M"), Ok(32 * 1024 * 1024));
    assert_eq!(parse_size("32MiB"), Ok(32 * 1024 * 1024));
    assert_eq!(parse_size("16mb"), Ok(16 * 1024 * 1024));
    assert_eq!(parse_size(" 8m "), Ok(8 * 1024 * 1024));
    assert_eq!(parse_size("512K"), Ok(512 * 1024));
    assert_eq!(parse_size("1G"), Ok(1024 * 1024 * 1024));
    assert_eq!(parse_size("5.5M"), Ok(5_767_168));
    assert!(parse_size("lots").is_err());
    assert!(parse_size("0").is_err());
    assert!(parse_size("-4M").is_err());
    assert!(parse_size("").is_err());
}

#[test]
fn only_the_documented_refusals_are_retried() {
    assert_eq!(classify(503, Some("UPLOAD_BUSY")), Retry::Busy);
    for (status, code) in [
        (429, None),
        (502, Some("STORE_ERROR")),
        (503, Some("STORAGE_UNAVAILABLE")),
        (400, Some("PART_INTERRUPTED")),
        (422, Some("CHECKSUM_MISMATCH")),
        (504, None),
        (500, Some("UPLOAD_ERROR")),
        (524, None),
    ] {
        assert_eq!(
            classify(status, code),
            Retry::Transient,
            "{status} {code:?}"
        );
    }
    for (status, code) in [
        (400, Some("PART_SIZE_MISMATCH")),
        (400, Some("INVALID_CHECKSUM")),
        (401, None),
        (403, None),
        (404, Some("UNKNOWN_PART")),
        (404, Some("NOT_FOUND")),
        (409, Some("UPLOAD_NOT_ACCEPTING")),
        (411, Some("LENGTH_REQUIRED")),
        (413, Some("PART_SIZE_MISMATCH")),
        (415, Some("UNSUPPORTED_MEDIA_TYPE")),
        (507, Some("INSUFFICIENT_STORAGE")),
    ] {
        assert_eq!(classify(status, code), Retry::Fatal, "{status} {code:?}");
    }
}

#[test]
fn backoff_grows_respects_retry_after_and_is_capped() {
    let t = Tuning::default();
    let first = backoff(&t, 1, None);
    assert!(first >= Duration::from_secs(1) && first < Duration::from_millis(1300));
    let third = backoff(&t, 3, None);
    assert!(third >= Duration::from_secs(4) && third < Duration::from_millis(5100));
    assert!(backoff(&t, 1, Some(7)) >= Duration::from_secs(7));
    assert_eq!(backoff(&t, 20, None), Duration::from_secs(30));
    assert_eq!(backoff(&t, 1, Some(3600)), Duration::from_secs(30));
}

#[test]
fn sizes_and_durations_read_naturally() {
    assert_eq!(human_bytes(0), "0 B");
    assert_eq!(human_bytes(999), "999 B");
    assert_eq!(human_bytes(1_500), "1.5 kB");
    assert_eq!(human_bytes(446_100_000), "446.1 MB");
    assert_eq!(human_bytes(3_500_000_000), "3.5 GB");
    assert_eq!(human_chunk(33_554_432), "32 MiB");
    assert_eq!(human_chunk(5_000_000), "5.0 MB");
    assert_eq!(human_duration(0.4), "0s");
    assert_eq!(human_duration(59.0), "59s");
    assert_eq!(human_duration(75.0), "1m15s");
    assert_eq!(human_duration(3725.0), "1h02m");
}

#[test]
fn progress_line_shows_bytes_files_rate_and_eta() {
    colored::control::set_override(false);
    let line = progress_line(200_000_000, 400_000_000, 3, 7, 10_000_000.0);
    assert!(line.contains(" 50.0%"), "{line}");
    assert!(line.contains("200.0 MB / 400.0 MB"), "{line}");
    assert!(line.contains("3/7 files"), "{line}");
    assert!(line.contains("10.0 MB/s"), "{line}");
    assert!(line.contains("ETA 20s"), "{line}");
    assert_eq!(line.matches('█').count(), 10);
    assert_eq!(line.matches('░').count(), 10);

    let done = progress_line(400, 400, 7, 7, 100.0);
    assert!(done.contains("100.0%") && done.contains("done"), "{done}");
    // nothing sent yet: no rate to divide by
    let idle = progress_line(0, 400, 0, 7, 0.0);
    assert!(idle.contains("ETA -"), "{idle}");
    // an upload of only empty files is complete, not 0/0 = NaN
    assert!(progress_line(0, 0, 1, 1, 0.0).contains("100.0%"));
}

#[test]
fn error_text_carries_the_code_and_its_context() {
    let r = RawResponse {
        status: 400,
        retry_after: None,
        body: r#"{"error":"Not a LeRobot dataset","code":"NOT_A_LEROBOT_DATASET","missing":["data/**/*.parquet"]}"#.into(),
    };
    let text = api_error(&r).to_string();
    assert!(text.contains("API error (400)"), "{text}");
    assert!(
        text.contains("Not a LeRobot dataset [NOT_A_LEROBOT_DATASET]"),
        "{text}"
    );
    assert!(
        text.contains(r#""missing":["data/**/*.parquet"]"#),
        "{text}"
    );

    // a proxy's non-JSON answer is still shown
    let html = RawResponse {
        status: 502,
        retry_after: None,
        body: "<html>Bad gateway</html>".into(),
    };
    assert!(api_error(&html).to_string().contains("Bad gateway"));
}

// ── Scratch directories ─────────────────────────────────────────────────

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rmsctl-test-{}-{}-{tag}",
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

const INFO: &str = r#"{"codebase_version":"v3.0","robot_type":"unitree_g1_dex3","fps":30,"total_episodes":2,
  "features":{"action":{"dtype":"float32","shape":[31]},"observation.state":{"dtype":"float32","shape":[31]}}}"#;

/// A miniature v3.0 tree, plus things that do not belong to one. With the
/// mock's 16-byte parts the files cover: several full parts plus a remainder,
/// an exact multiple, a single part, and an empty file.
fn dataset(tag: &str) -> Scratch {
    let s = Scratch::new(tag);
    s.write("meta/info.json", INFO.as_bytes());
    s.write("meta/episodes/chunk-000/file-000.parquet", &pattern(40, 1));
    s.write("meta/empty.jsonl", b"");
    s.write("data/chunk-000/file-000.parquet", &pattern(32, 2));
    s.write(
        "videos/observation.images.head/chunk-000/file-000.mp4",
        &pattern(100, 3),
    );
    s.write("README.md", b"not part of the dataset");
    s.write(".git/objects/aa/bb", b"nor is this");
    s.write("data/chunk-000/notes.txt", b"nor this");
    s
}

#[test]
fn scan_collects_the_tree_and_leaves_the_rest() {
    let s = dataset("scan");
    let scan = scan_dir(&s.0).unwrap();
    let rels: Vec<&str> = scan.files.iter().map(|f| f.rel.as_str()).collect();
    assert_eq!(
        rels,
        [
            "data/chunk-000/file-000.parquet",
            "meta/empty.jsonl",
            "meta/episodes/chunk-000/file-000.parquet",
            "meta/info.json",
            "videos/observation.images.head/chunk-000/file-000.mp4",
        ]
    );
    assert_eq!(scan.total_bytes(), 32 + 40 + 100 + INFO.len() as u64);
    // README.md, .git (not walked: one entry) and data/…/notes.txt
    assert_eq!(scan.skipped, 3);
    assert!(scan.files.iter().all(|f| f.abs.is_file()));
}

#[cfg(unix)]
#[test]
fn scan_follows_symlinks_like_a_hub_cache_snapshot() {
    let s = dataset("links");
    s.write("blobs/abc", &pattern(24, 9));
    std::os::unix::fs::symlink(
        s.path("blobs/abc"),
        s.path("data/chunk-000/file-001.parquet"),
    )
    .unwrap();
    let scan = scan_dir(&s.0).unwrap();
    let linked = scan
        .files
        .iter()
        .find(|f| f.rel == "data/chunk-000/file-001.parquet")
        .expect("the linked file is part of the tree");
    assert_eq!(linked.size, 24);
}

#[test]
fn a_directory_without_info_json_is_refused_with_a_reason() {
    let s = Scratch::new("noinfo");
    s.write("data/chunk-000/file-000.parquet", b"x");
    let err = read_info(&s.0).unwrap_err().to_string();
    assert!(err.contains("not a LeRobot dataset root"), "{err}");
    assert!(err.contains("meta/ and data/"), "{err}");

    s.write("meta/info.json", b"{ not json");
    assert!(
        read_info(&s.0)
            .unwrap_err()
            .to_string()
            .contains("not valid JSON")
    );
}

#[test]
fn range_hash_reads_exactly_the_part_and_notices_a_truncated_file() {
    let s = Scratch::new("hash");
    s.write("f.bin", b"hello world");
    let f = s.path("f.bin");
    // sha256("hello") and sha256("world")
    assert_eq!(
        sha256_range(&f, 0, 5).unwrap(),
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    assert_eq!(
        sha256_range(&f, 6, 5).unwrap(),
        "486ea46224d1bb4fb680f34f7c9ad96a8f24ec88be73ea8e5a6c65260e9cb8a7"
    );
    // sha256("")
    assert_eq!(
        sha256_range(&f, 0, 0).unwrap(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    let err = sha256_range(&f, 6, 50).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
}

fn local(rel: &str, size: u64) -> LocalFile {
    LocalFile {
        rel: rel.into(),
        abs: PathBuf::from("/nowhere").join(rel),
        size,
    }
}

fn remote(id: u64, path: &str, size: u64, complete: bool, received: &[u32]) -> RemoteFile {
    RemoteFile {
        id,
        path: path.into(),
        size,
        part_count: None,
        complete,
        received_parts: received.to_vec(),
    }
}

#[test]
fn plan_lists_exactly_the_parts_the_server_lacks() {
    let locals = [
        local("meta/info.json", 10),
        local("videos/a.mp4", 100),
        local("data/a.parquet", 32),
    ];
    let remotes = [
        remote(0, "meta/info.json", 10, true, &[1]),
        remote(1, "videos/a.mp4", 100, false, &[1, 3, 7]),
        remote(2, "data/a.parquet", 32, false, &[]),
    ];
    let plan = build_plan(&locals, &remotes, &HashSet::new(), 16).unwrap();
    assert_eq!(plan.total_bytes, 142);
    assert!(plan.files[0].missing.is_empty());
    assert_eq!(plan.files[1].missing, [2, 4, 5, 6]);
    assert_eq!(plan.files[2].missing, [1, 2]);
    assert_eq!(plan.missing_parts(), 6);
    // parts 2,4,5,6 of the video are full (the short one, 7, is held) + 32
    assert_eq!(plan.missing_bytes(), 4 * 16 + 32);

    // a multi-part file reads complete:false with every part listed until
    // finish assembles it — nothing to send
    let all = [remote(
        1,
        "videos/a.mp4",
        100,
        false,
        &[1, 2, 3, 4, 5, 6, 7],
    )];
    let plan = build_plan(&locals[1..2], &all, &HashSet::new(), 16).unwrap();
    assert_eq!(plan.missing_parts(), 0);
}

#[test]
fn plan_refuses_a_directory_that_changed_since_the_upload_started() {
    let remotes = [remote(0, "meta/info.json", 10, false, &[])];

    let resized = build_plan(
        &[local("meta/info.json", 11)],
        &remotes,
        &HashSet::new(),
        16,
    );
    let text = resized.unwrap_err().to_string();
    assert!(
        text.contains("11 bytes on disk but was declared as 10"),
        "{text}"
    );
    assert!(text.contains("--restart"), "{text}");

    let gone = build_plan(&[], &remotes, &HashSet::new(), 16);
    assert!(
        gone.unwrap_err()
            .to_string()
            .contains("no longer in the directory")
    );

    let grown = build_plan(
        &[local("meta/info.json", 10), local("data/new.parquet", 5)],
        &remotes,
        &HashSet::new(),
        16,
    );
    assert!(grown.unwrap_err().to_string().contains("was not declared"));

    // …unless the server itself said it ignores that file
    let ignored: HashSet<String> = ["data/new.parquet".to_string()].into();
    assert!(
        build_plan(
            &[local("meta/info.json", 10), local("data/new.parquet", 5)],
            &remotes,
            &ignored,
            16
        )
        .is_ok()
    );
}

#[test]
fn the_note_is_keyed_by_server_and_directory() {
    let s = Scratch::new("state");
    let dir = Some(s.0.clone());
    let a = StateStore::new(dir.clone(), "http://one/api", Path::new("/data/set"));
    let b = StateStore::new(dir.clone(), "http://two/api", Path::new("/data/set"));
    let c = StateStore::new(dir.clone(), "http://one/api", Path::new("/data/other"));
    assert!(a.load().is_none());
    assert!(a.save("ds-a", "set"));
    assert!(b.save("ds-b", "set"));
    assert_eq!(a.load().unwrap().dataset_id, "ds-a");
    assert_eq!(b.load().unwrap().dataset_id, "ds-b");
    assert!(c.load().is_none());

    // a cancel knows only the id: it drops that note and no other
    forget_dataset(dir.clone(), "http://one/api", "ds-b");
    assert!(b.load().is_some(), "same id on another server is untouched");
    forget_dataset(dir, "http://one/api", "ds-a");
    assert!(a.load().is_none());
    assert!(b.load().is_some());
    b.remove();
    assert!(b.load().is_none());
}

// ── A server that speaks the contract ───────────────────────────────────

const DS: &str = "11111111-2222-3333-4444-555555555555";

enum Fault {
    /// Answer the next part request with this refusal instead of storing it.
    Refuse(u16, &'static str, Option<u64>),
    /// Store the part but report a different hash, like a proxy answering
    /// from somewhere else.
    WrongSha,
}

struct MockFile {
    path: String,
    size: u64,
    sha256: Option<String>,
}

#[derive(Default)]
struct Mock {
    chunk: u64,
    files: Vec<MockFile>,
    parts: HashMap<(u64, u32), Vec<u8>>,
    /// "" until an upload exists.
    state: String,
    status: String,
    verdict: String,
    import_error: Option<Value>,
    polls_before_verdict: u32,
    faults: VecDeque<Fault>,
    /// Answer the next start requests with these refusals; an empty code is a
    /// proxy's answer (no `code` in the body).
    start_refusals: VecDeque<(u16, &'static str)>,
    /// Answer this many part requests with UPLOAD_BUSY as soon as the headers
    /// are in, without reading the body — what the real server does.
    early_busy: u32,
    /// Refuse the n-th part request (counted from 1) with a 502.
    refuse_put_number: Option<usize>,
    /// Parts to "lose" when finish is first called.
    lose_on_finish: Vec<(u64, u32)>,
    start_body: Option<Value>,
    log: Vec<String>,
    bad_requests: Vec<String>,
    max_in_flight_seen: usize,
    /// Body bytes that still arrived after an early refusal.
    poured_after_refusal: Vec<usize>,
}

type Reply = (u16, Option<u64>, Value);

impl Mock {
    fn new(chunk: u64) -> Self {
        Mock {
            chunk,
            verdict: "ready".into(),
            ..Default::default()
        }
    }

    fn parts_of(&self, file: u64) -> u32 {
        part_count(self.files[file as usize].size, self.chunk)
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

    fn puts(&self) -> Vec<&String> {
        self.log.iter().filter(|l| l.starts_with("PUT ")).collect()
    }

    fn count(&self, line: &str) -> usize {
        self.log.iter().filter(|l| l.as_str() == line).count()
    }

    fn refusal(status: u16, code: &str) -> Reply {
        (
            status,
            None,
            json!({ "error": format!("refused: {code}"), "code": code }),
        )
    }

    fn status_body(&self) -> Value {
        let uploading = self.state == "uploading";
        let files: Vec<Value> = self
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let parts = self.parts_of(i as u64);
                let mut held: Vec<u32> = (1..=parts)
                    .filter(|n| self.parts.contains_key(&(i as u64, *n)))
                    .collect();
                held.sort();
                let complete = if uploading {
                    parts == 1 && held.len() == 1
                } else {
                    true
                };
                json!({ "id": i, "path": f.path, "size": f.size, "partCount": parts,
                        "complete": complete, "receivedParts": held })
            })
            .collect();
        let received: usize = self.parts.values().map(|p| p.len()).sum();
        json!({
            "datasetId": DS, "state": self.state, "status": self.status, "storage": "rustfs",
            "chunkSize": self.chunk, "totalBytes": self.files.iter().map(|f| f.size).sum::<u64>(),
            "receivedBytes": received, "totalFiles": self.files.len(),
            "completedFiles": files.iter().filter(|f| f["complete"] == true).count(),
            "readyToFinish": self.complete(), "files": files,
            "lastActivityAt": "2026-11-13T10:00:00.000Z",
            "expiresAt": if uploading { json!("2026-11-13T16:00:00.000Z") } else { Value::Null },
            "importError": self.import_error.clone().unwrap_or(Value::Null),
        })
    }

    fn handle(
        &mut self,
        method: &str,
        path: &str,
        headers: &HashMap<String, String>,
        body: &[u8],
    ) -> Reply {
        self.log.push(format!("{method} {path}"));
        let exists = !self.state.is_empty() && self.state != "cancelled";
        let base = format!("/api/datasets/{DS}");

        if method == "POST" && path == "/api/datasets/uploads" {
            match self.start_refusals.pop_front() {
                Some((status, "")) => return (status, None, Value::Null),
                Some((status, code)) => return Mock::refusal(status, code),
                None => {}
            }
            let req: Value = serde_json::from_slice(body).unwrap();
            self.files = req["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| MockFile {
                    path: f["path"].as_str().unwrap().into(),
                    size: f["size"].as_u64().unwrap(),
                    sha256: f["sha256"].as_str().map(String::from),
                })
                .collect();
            self.start_body = Some(req);
            self.state = "uploading".into();
            self.status = "uploading".into();
            self.parts.clear();
            let files: Vec<Value> = self
                .files
                .iter()
                .enumerate()
                .map(|(i, f)| json!({ "id": i, "path": f.path, "size": f.size, "partCount": self.parts_of(i as u64) }))
                .collect();
            return (
                201,
                None,
                json!({ "datasetId": DS, "status": "uploading", "state": "uploading", "robotTypeId": "rt-1",
                        "storage": "rustfs", "chunkSize": self.chunk, "files": files, "ignored": [],
                        "idleTimeoutSeconds": 21600, "expiresAt": "2026-11-13T16:00:00.000Z" }),
            );
        }
        if !path.starts_with(&base) || !exists {
            return Mock::refusal(404, "NOT_FOUND");
        }
        let rest = &path[base.len()..];

        match (method, rest) {
            ("GET", "/upload") => {
                if self.state == "processing" {
                    if self.polls_before_verdict == 0 {
                        self.state = if self.import_error.is_some() {
                            "failed"
                        } else {
                            "completed"
                        }
                        .into();
                        self.status = self.verdict.clone();
                    } else {
                        self.polls_before_verdict -= 1;
                    }
                }
                (200, None, self.status_body())
            }
            ("POST", "/upload/finish") => {
                if !self.lose_on_finish.is_empty() {
                    let lost = std::mem::take(&mut self.lose_on_finish);
                    for key in &lost {
                        self.parts.remove(key);
                    }
                    return (
                        409,
                        None,
                        json!({ "error": "parts are missing", "code": "UPLOAD_INCOMPLETE",
                                "missingFiles": 1, "missingBytes": 1, "missing": [] }),
                    );
                }
                if self.state != "uploading" {
                    return (200, None, json!({ "datasetId": DS, "state": self.state }));
                }
                if !self.complete() {
                    return Mock::refusal(409, "UPLOAD_INCOMPLETE");
                }
                self.state = "processing".into();
                (
                    202,
                    None,
                    json!({ "datasetId": DS, "state": "processing", "status": "uploading" }),
                )
            }
            ("DELETE", "") => {
                self.state = "cancelled".into();
                self.parts.clear();
                (200, None, Value::Null)
            }
            ("POST", "/upload/cancel") => {
                self.state = "cancelled".into();
                self.parts.clear();
                (
                    200,
                    None,
                    json!({ "datasetId": DS, "status": "cancelled", "message": "Upload cancelled" }),
                )
            }
            ("GET", "") => {
                let valid = self.status == "ready";
                (
                    200,
                    None,
                    json!({ "dataset": { "id": DS, "name": "g1", "status": self.status,
                        "demonstrationCount": 80, "totalFrames": 55695, "lerobotVersion": "v3.0",
                        "validation": { "valid": valid,
                            "errors": if valid { json!([]) } else { json!([{ "code": "EPISODE_COUNT", "message": "expected 80 episodes" }]) },
                            "warnings": [] } } }),
                )
            }
            ("PUT", part) => {
                let seg: Vec<&str> = part.split('/').collect();
                // "", "upload", "files", <id>, "parts", <n>
                let (file, n): (u64, u32) = (seg[3].parse().unwrap(), seg[5].parse().unwrap());
                if self.refuse_put_number == Some(self.puts().len()) {
                    self.refuse_put_number = None;
                    return Mock::refusal(502, "STORE_ERROR");
                }
                let fault = self.faults.pop_front();
                if let Some(Fault::Refuse(status, code, retry_after)) = fault {
                    let mut reply = Mock::refusal(status, code);
                    reply.1 = retry_after;
                    return reply;
                }
                if headers.get("content-type").map(String::as_str)
                    != Some("application/octet-stream")
                {
                    self.bad_requests.push(format!("{path}: content-type"));
                    return Mock::refusal(415, "UNSUPPORTED_MEDIA_TYPE");
                }
                if self.state != "uploading" {
                    return Mock::refusal(409, "UPLOAD_NOT_ACCEPTING");
                }
                let (_, expected) = part_range(self.files[file as usize].size, self.chunk, n);
                let declared = headers
                    .get("content-length")
                    .and_then(|v| v.parse::<u64>().ok());
                if declared != Some(expected) || body.len() as u64 != expected {
                    self.bad_requests
                        .push(format!("{path}: length {declared:?}, wanted {expected}"));
                    return Mock::refusal(400, "PART_SIZE_MISMATCH");
                }
                let actual = hex(&Sha256::digest(body));
                match headers.get("x-chunk-sha256") {
                    Some(sent) if *sent == actual => {}
                    other => {
                        self.bad_requests
                            .push(format!("{path}: checksum header {other:?}"));
                        self.parts.remove(&(file, n));
                        return Mock::refusal(422, "CHECKSUM_MISMATCH");
                    }
                }
                self.parts.insert((file, n), body.to_vec());
                let reported = if matches!(fault, Some(Fault::WrongSha)) {
                    "0".repeat(64)
                } else {
                    actual
                };
                (
                    200,
                    None,
                    json!({ "datasetId": DS, "fileId": file, "partNumber": n, "size": expected, "sha256": reported,
                            "fileComplete": (1..=self.parts_of(file)).all(|p| self.parts.contains_key(&(file, p))) }),
                )
            }
            _ => Mock::refusal(404, "NOT_FOUND"),
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
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
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap_or("").split(' ');
        let method = first.next().unwrap_or("").to_string();
        let path = first.next().unwrap_or("").to_string();
        let headers: HashMap<String, String> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();

        // The real server answers a body without Content-Length with 411.
        if headers.contains_key("transfer-encoding") {
            mock.lock()
                .unwrap()
                .bad_requests
                .push(format!("{path}: chunked"));
            let body = r#"{"error":"length required","code":"LENGTH_REQUIRED"}"#;
            let _ = sock
                .write_all(
                    format!("HTTP/1.1 411 Length Required\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
                        .as_bytes(),
                )
                .await;
            return;
        }

        let length: usize = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        // Only a part big enough to still be on its way when the answer lands.
        let refuse_early = method == "PUT" && length >= 1024 * 1024 && {
            let mut m = mock.lock().unwrap();
            let early = m.early_busy > 0;
            if early {
                m.early_busy -= 1;
                m.log.push(format!("EARLY {path}"));
            }
            early
        };
        if refuse_early {
            // Like the real server: answer at once, keep the connection, and
            // let whatever body still arrives drain away.
            let body = r#"{"error":"busy","code":"UPLOAD_BUSY","retryAfterSeconds":2}"#;
            let _ = sock
                .write_all(
                    format!(
                        "HTTP/1.1 503 Service Unavailable\r\nretry-after: 2\r\ncontent-length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await;
            // Give the client time to read the answer, then drain: a client
            // that keeps sending pours the whole part in here.
            tokio::time::sleep(Duration::from_millis(250)).await;
            let mut poured = buf.len().saturating_sub(head_end + 4);
            let until = tokio::time::Instant::now() + Duration::from_millis(400);
            while poured < length {
                match tokio::time::timeout_at(until, sock.read(&mut tmp)).await {
                    Ok(Ok(n)) if n > 0 => poured += n,
                    _ => break,
                }
            }
            mock.lock().unwrap().poured_after_refusal.push(poured);
            if poured < length {
                return;
            }
            buf.clear();
            continue;
        }

        let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        let mut body = buf.split_off(head_end + 4);
        buf.clear();
        while body.len() < length {
            match sock.read(&mut tmp).await {
                Ok(n) if n > 0 => body.extend_from_slice(&tmp[..n]),
                _ => return,
            }
        }
        buf = body.split_off(length);
        // Hold the request open for a moment so parallel parts overlap.
        if method == "PUT" {
            tokio::time::sleep(Duration::from_millis(3)).await;
        }
        let (status, retry_after, reply) = {
            let mut m = mock.lock().unwrap();
            m.max_in_flight_seen = m.max_in_flight_seen.max(now);
            m.handle(&method, &path, &headers, &body)
        };
        in_flight.fetch_sub(1, Ordering::SeqCst);
        let text = reply.to_string();
        let retry = retry_after
            .map(|s| format!("retry-after: {s}\r\n"))
            .unwrap_or_default();
        let response = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n{retry}content-length: {}\r\n\r\n{text}",
            text.len()
        );
        if sock.write_all(response.as_bytes()).await.is_err() {
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
    // A small receive buffer, so a body the mock is not reading stays in the
    // client instead of vanishing into the kernel — "did the client keep
    // sending after it was refused" is then a question with one answer.
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(64 * 1024).unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let listener: TcpListener = socket.listen(64).unwrap();
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
    Server {
        mock,
        client,
        state: Scratch::new("notes"),
    }
}

fn ctx(yes: bool) -> Ctx {
    // --json doubles as the quiet mode, which keeps the test output readable.
    Ctx {
        json: true,
        yes,
        page: None,
        limit: None,
    }
}

fn options(dir: &Path, state: &Scratch) -> UploadOptions {
    let ms = Duration::from_millis;
    UploadOptions {
        dir: dir.to_path_buf(),
        name: None,
        description: None,
        robot_type: None,
        chunk_size: None,
        parallel: 3,
        retries: 4,
        resume: None,
        restart: false,
        no_checksum: false,
        no_wait: false,
        wait_timeout: Duration::from_secs(20),
        part_timeout: Duration::from_secs(20),
        state_dir: Some(state.0.clone()),
        tuning: Tuning {
            backoff_base: ms(1),
            max_pause: ms(5),
            poll_interval: ms(2),
        },
    }
}

/// Every file the mock holds is byte-identical to the one on disk.
fn assert_stored_matches_disk(mock: &Mock, dir: &Path) {
    assert!(mock.complete(), "the server holds every part");
    for (i, f) in mock.files.iter().enumerate() {
        let disk = std::fs::read(dir.join(&f.path)).unwrap();
        assert_eq!(mock.assembled(i as u64), disk, "{} differs", f.path);
    }
}

// ── The flow ────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn uploads_a_tree_in_parts_and_waits_for_ready() {
    let data = dataset("straight");
    let mut mock = Mock::new(16);
    mock.polls_before_verdict = 2;
    let srv = server(mock).await;
    let mut opts = options(&data.0, &srv.state);
    opts.name = Some("  g1 bottle sorting  ".into());
    opts.description = Some("sim data".into());
    opts.robot_type = Some("rt-7".into());
    opts.chunk_size = Some(8 * 1024 * 1024);

    let outcome = upload(&srv.client, &ctx(true), &opts).await.unwrap();

    assert_eq!(outcome.dataset_id, DS);
    assert_eq!(outcome.status, "ready");
    assert!(outcome.executed);
    let m = srv.mock.lock().unwrap();
    assert!(m.bad_requests.is_empty(), "{:?}", m.bad_requests);
    assert_stored_matches_disk(&m, &data.0);

    // the manifest: only the tree, with whole-file hashes and the flags
    let body = m.start_body.as_ref().unwrap();
    assert_eq!(body["name"], "g1 bottle sorting");
    assert_eq!(body["description"], "sim data");
    assert_eq!(body["robotTypeId"], "rt-7");
    assert_eq!(body["chunkSize"], 8 * 1024 * 1024);
    assert_eq!(body["info"]["robot_type"], "unitree_g1_dex3");
    let paths: Vec<&str> = body["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths.len(), 5);
    assert!(
        !paths
            .iter()
            .any(|p| p.contains("README") || p.contains(".git") || p.contains("notes"))
    );
    for f in &m.files {
        let disk = std::fs::read(data.0.join(&f.path)).unwrap();
        assert_eq!(
            f.sha256.as_deref(),
            Some(hex(&Sha256::digest(&disk)).as_str()),
            "{}",
            f.path
        );
    }

    // parts: 100 B → 7, 40 B → 3, 32 B → 2, info.json, and the empty file's
    // single empty part
    let info_parts = part_count(INFO.len() as u64, 16) as usize;
    assert_eq!(m.puts().len(), 7 + 3 + 2 + info_parts + 1);
    assert_eq!(outcome.parts_sent, m.puts().len());
    assert_eq!(outcome.bytes_sent, 100 + 40 + 32 + INFO.len() as u64);
    assert_eq!(
        m.count(&format!("POST /api/datasets/{DS}/upload/finish")),
        1
    );
    assert!(m.max_in_flight_seen > 1, "parts overlap");
    assert!(
        m.max_in_flight_seen <= 3,
        "never more than --parallel at once"
    );
    // finished: the note is gone, so a later run starts a new upload
    assert_eq!(std::fs::read_dir(&srv.state.0).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_yes_nothing_is_sent() {
    let data = dataset("gate");
    let srv = server(Mock::new(16)).await;
    let opts = options(&data.0, &srv.state);

    let outcome = upload(&srv.client, &ctx(false), &opts).await.unwrap();

    assert!(!outcome.executed);
    let m = srv.mock.lock().unwrap();
    assert!(m.log.is_empty(), "the gate sent {:?}", m.log);
    assert_eq!(std::fs::read_dir(&srv.state.0).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_again_sends_only_what_the_server_lacks() {
    let data = dataset("resume");
    let srv = server(Mock::new(16)).await;
    let opts = options(&data.0, &srv.state);

    // First run: one part at a time, and the sixth request is refused for
    // good — the run dies partway through with five parts stored.
    srv.mock.lock().unwrap().refuse_put_number = Some(6);
    let mut first = options(&data.0, &srv.state);
    first.parallel = 1;
    first.retries = 0;
    let err = upload(&srv.client, &ctx(true), &first).await.unwrap_err();
    assert!(err.to_string().contains("failed after 1 attempts"), "{err}");
    let stored_before = {
        let m = srv.mock.lock().unwrap();
        assert_eq!(m.parts.len(), 5);
        assert!(!m.complete());
        assert_eq!(m.state, "uploading", "a failed run leaves the upload open");
        m.parts.len()
    };
    // the note survived the failure
    assert_eq!(std::fs::read_dir(&srv.state.0).unwrap().count(), 1);

    let puts_before = srv.mock.lock().unwrap().puts().len();
    let outcome = upload(&srv.client, &ctx(true), &opts).await.unwrap();

    assert_eq!(outcome.status, "ready");
    let m = srv.mock.lock().unwrap();
    assert_eq!(
        m.count("POST /api/datasets/uploads"),
        1,
        "resume does not declare a second dataset"
    );
    let total_parts: usize = (0..m.files.len() as u64)
        .map(|f| m.parts_of(f) as usize)
        .sum();
    assert_eq!(
        m.puts().len() - puts_before,
        total_parts - stored_before,
        "only the missing parts were sent"
    );
    assert_eq!(outcome.parts_sent, total_parts - stored_before);
    assert_stored_matches_disk(&m, &data.0);
    assert!(m.bad_requests.is_empty(), "{:?}", m.bad_requests);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_by_id_works_without_a_note_and_refuses_a_changed_directory() {
    let data = dataset("byid");
    let srv = server(Mock::new(16)).await;

    // An upload started elsewhere: declared, with the first video part held.
    let scan = scan_dir(&data.0).unwrap();
    {
        let mut m = srv.mock.lock().unwrap();
        m.files = scan
            .files
            .iter()
            .map(|f| MockFile {
                path: f.rel.clone(),
                size: f.size,
                sha256: None,
            })
            .collect();
        m.state = "uploading".into();
        m.status = "uploading".into();
        let video = m
            .files
            .iter()
            .position(|f| f.path.ends_with(".mp4"))
            .unwrap() as u64;
        let bytes =
            std::fs::read(data.path("videos/observation.images.head/chunk-000/file-000.mp4"))
                .unwrap();
        m.parts.insert((video, 1), bytes[..16].to_vec());
    }

    // the directory changed since: refused before anything is sent
    let grown = dataset("byid-grown");
    grown.write("data/chunk-000/file-001.parquet", b"new episode table");
    let mut opts = options(&grown.0, &srv.state);
    opts.resume = Some(DS.into());
    let err = upload(&srv.client, &ctx(true), &opts)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("was not declared when the upload started"),
        "{err}"
    );
    assert!(srv.mock.lock().unwrap().puts().is_empty());

    let mut opts = options(&data.0, &srv.state);
    opts.resume = Some(DS.into());
    let outcome = upload(&srv.client, &ctx(true), &opts).await.unwrap();
    assert_eq!(outcome.status, "ready");
    {
        let m = srv.mock.lock().unwrap();
        assert_eq!(m.count("POST /api/datasets/uploads"), 0);
        let video = m
            .files
            .iter()
            .position(|f| f.path.ends_with(".mp4"))
            .unwrap();
        let held = format!("PUT /api/datasets/{DS}/upload/files/{video}/parts/1");
        assert_eq!(m.count(&held), 0, "the part already held is not sent again");
        assert_stored_matches_disk(&m, &data.0);
    }

    // an id the server does not know is an error, not a silent new upload
    let mut opts = options(&data.0, &srv.state);
    opts.resume = Some("00000000-0000-0000-0000-000000000000".into());
    let err = upload(&srv.client, &ctx(true), &opts)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("does not exist on this server"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_note_for_an_upload_the_server_dropped_starts_a_new_one() {
    let data = dataset("stale");
    let srv = server(Mock::new(16)).await;
    let opts = options(&data.0, &srv.state);
    let root = std::fs::canonicalize(&data.0).unwrap();
    let store = StateStore::new(opts.state_dir.clone(), srv.client.base_url(), &root);
    assert!(store.save(DS, "old"));

    // the mock holds no upload: GET answers 404, as after a reaped upload
    let outcome = upload(&srv.client, &ctx(true), &opts).await.unwrap();

    assert_eq!(outcome.status, "ready");
    let m = srv.mock.lock().unwrap();
    assert_eq!(m.log[0], format!("GET /api/datasets/{DS}/upload"));
    assert_eq!(m.count("POST /api/datasets/uploads"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_discards_the_earlier_upload_first() {
    let data = dataset("restart");
    let srv = server(Mock::new(16)).await;
    let mut opts = options(&data.0, &srv.state);
    let root = std::fs::canonicalize(&data.0).unwrap();
    {
        let mut m = srv.mock.lock().unwrap();
        m.files = vec![MockFile {
            path: "meta/info.json".into(),
            size: 3,
            sha256: None,
        }];
        m.state = "uploading".into();
        m.status = "uploading".into();
    }
    StateStore::new(opts.state_dir.clone(), srv.client.base_url(), &root).save(DS, "old");
    opts.restart = true;

    let outcome = upload(&srv.client, &ctx(true), &opts).await.unwrap();

    assert_eq!(outcome.status, "ready");
    let m = srv.mock.lock().unwrap();
    let cancel = m
        .log
        .iter()
        .position(|l| l.ends_with("/upload/cancel"))
        .expect("the old upload is cancelled");
    let start = m
        .log
        .iter()
        .position(|l| l == "POST /api/datasets/uploads")
        .unwrap();
    assert!(cancel < start);
    assert_stored_matches_disk(&m, &data.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transient_refusals_are_retried_and_a_busy_server_is_waited_out() {
    let data = dataset("retry");
    let mut mock = Mock::new(16);
    // More busy answers than --retries allows failures: busy must not count.
    for _ in 0..7 {
        mock.faults
            .push_back(Fault::Refuse(503, "UPLOAD_BUSY", Some(2)));
    }
    mock.faults
        .push_back(Fault::Refuse(502, "STORE_ERROR", None));
    mock.faults
        .push_back(Fault::Refuse(422, "CHECKSUM_MISMATCH", None));
    mock.faults
        .push_back(Fault::Refuse(400, "PART_INTERRUPTED", None));
    mock.faults.push_back(Fault::Refuse(429, "", Some(1)));
    mock.faults.push_back(Fault::WrongSha);
    let srv = server(mock).await;
    let mut opts = options(&data.0, &srv.state);
    // One part at a time, so every fault lands on the same part: five real
    // failures are within --retries 5, and the seven busy answers are extra.
    opts.parallel = 1;
    opts.retries = 5;

    let outcome = upload(&srv.client, &ctx(true), &opts).await.unwrap();

    assert_eq!(outcome.status, "ready");
    let m = srv.mock.lock().unwrap();
    assert!(m.faults.is_empty());
    assert_stored_matches_disk(&m, &data.0);
    let total_parts: usize = (0..m.files.len() as u64)
        .map(|f| m.parts_of(f) as usize)
        .sum();
    assert_eq!(
        m.puts().len(),
        total_parts + 12,
        "each fault cost exactly one extra request"
    );
    // progress counts a part once however often it was sent
    assert_eq!(
        outcome.bytes_sent,
        m.files.iter().map(|f| f.size).sum::<u64>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refusal_that_cannot_improve_stops_at_once_and_keeps_the_upload() {
    let data = dataset("fatal");
    let mut mock = Mock::new(16);
    mock.faults
        .push_back(Fault::Refuse(409, "UPLOAD_NOT_ACCEPTING", None));
    let srv = server(mock).await;
    let mut opts = options(&data.0, &srv.state);
    opts.parallel = 1;

    let err = upload(&srv.client, &ctx(true), &opts)
        .await
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("API error (409)") && err.contains("[UPLOAD_NOT_ACCEPTING]"),
        "{err}"
    );
    let m = srv.mock.lock().unwrap();
    assert_eq!(m.puts().len(), 1, "not retried");
    assert!(
        !m.log
            .iter()
            .any(|l| l.ends_with("/finish") || l.ends_with("/cancel"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finish_reporting_missing_parts_sends_them_again() {
    let data = dataset("incomplete");
    let mut mock = Mock::new(16);
    mock.lose_on_finish = vec![(0, 1), (0, 2)];
    let srv = server(mock).await;
    let opts = options(&data.0, &srv.state);

    let outcome = upload(&srv.client, &ctx(true), &opts).await.unwrap();

    assert_eq!(outcome.status, "ready");
    let m = srv.mock.lock().unwrap();
    assert_eq!(
        m.count(&format!("POST /api/datasets/{DS}/upload/finish")),
        2
    );
    assert_eq!(
        m.count(&format!("PUT /api/datasets/{DS}/upload/files/0/parts/1")),
        2
    );
    assert_eq!(
        m.count(&format!("PUT /api/datasets/{DS}/upload/files/0/parts/2")),
        2
    );
    assert_stored_matches_disk(&m, &data.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dataset_that_fails_validation_is_an_error() {
    let data = dataset("invalid");
    let mut mock = Mock::new(16);
    mock.verdict = "failed".into();
    let srv = server(mock).await;
    let opts = options(&data.0, &srv.state);

    let err = upload(&srv.client, &ctx(true), &opts)
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("ended 'failed'"), "{err}");
    assert!(err.contains(DS), "{err}");
    // a verdict is final: the next run must not try to resume into it
    assert_eq!(std::fs::read_dir(&srv.state.0).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_checksum_failure_while_assembling_is_reported_with_its_reason() {
    let data = dataset("assemble");
    let mut mock = Mock::new(16);
    mock.verdict = "failed".into();
    mock.import_error = Some(
        json!({ "phase": "storing", "error": "Checksum mismatch for data/chunk-000/file-000.parquet",
                                     "failedAt": "2026-11-13T10:00:00.000Z", "sink": "rustfs" }),
    );
    let srv = server(mock).await;
    let opts = options(&data.0, &srv.state);

    let err = upload(&srv.client, &ctx(true), &opts)
        .await
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("Checksum mismatch for data/chunk-000/file-000.parquet (while storing)"),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_wait_returns_after_finish_and_no_checksum_declares_no_hashes() {
    let data = dataset("nowait");
    let mut mock = Mock::new(16);
    mock.polls_before_verdict = 1_000_000;
    let srv = server(mock).await;
    let mut opts = options(&data.0, &srv.state);
    opts.no_wait = true;
    opts.no_checksum = true;

    let outcome = upload(&srv.client, &ctx(true), &opts).await.unwrap();

    assert_eq!(outcome.status, "uploading");
    let m = srv.mock.lock().unwrap();
    assert_eq!(m.state, "processing");
    assert!(m.files.iter().all(|f| f.sha256.is_none()));
    // parts are still verified one by one
    assert!(m.bad_requests.is_empty(), "{:?}", m.bad_requests);
    // still noted: running again picks the verdict up
    assert_eq!(std::fs::read_dir(&srv.state.0).unwrap().count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_again_after_finish_only_waits_for_the_verdict() {
    let data = dataset("verdict");
    let srv = server(Mock::new(16)).await;
    let mut opts = options(&data.0, &srv.state);
    opts.no_wait = true;
    upload(&srv.client, &ctx(true), &opts).await.unwrap();
    let before = srv.mock.lock().unwrap().log.len();

    opts.no_wait = false;
    // read-only from here on, so it runs even without --yes
    let outcome = upload(&srv.client, &ctx(false), &opts).await.unwrap();

    assert_eq!(outcome.status, "ready");
    let m = srv.mock.lock().unwrap();
    assert!(
        m.log[before..].iter().all(|l| l.starts_with("GET ")),
        "{:?}",
        &m.log[before..]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waiting_for_the_verdict_gives_up_after_the_timeout() {
    let data = dataset("timeout");
    let mut mock = Mock::new(16);
    mock.polls_before_verdict = 1_000_000;
    let srv = server(mock).await;
    let mut opts = options(&data.0, &srv.state);
    opts.wait_timeout = Duration::from_millis(30);

    let err = upload(&srv.client, &ctx(true), &opts)
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("still checking"), "{err}");
    assert!(err.contains("upload-status"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_that_shrank_after_it_was_declared_stops_the_upload() {
    let data = dataset("shrunk");
    let srv = server(Mock::new(16)).await;
    let scan = scan_dir(&std::fs::canonicalize(&data.0).unwrap()).unwrap();
    {
        let mut m = srv.mock.lock().unwrap();
        m.files = scan
            .files
            .iter()
            .map(|f| MockFile {
                path: f.rel.clone(),
                size: f.size,
                sha256: None,
            })
            .collect();
        m.state = "uploading".into();
        m.status = "uploading".into();
    }
    // The plan is built from the declared sizes; the file is cut afterwards.
    let plan = build_plan(
        &scan.files,
        &srv.mock
            .lock()
            .unwrap()
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| remote(i as u64, &f.path, f.size, false, &[]))
            .collect::<Vec<_>>(),
        &HashSet::new(),
        16,
    )
    .unwrap();
    data.write(
        "videos/observation.images.head/chunk-000/file-000.mp4",
        &pattern(20, 3),
    );
    let opts = options(&data.0, &srv.state);
    let progress = Arc::new(Progress::new(&plan));

    let err = send_missing(&srv.client, DS, &plan, &opts, &progress)
        .await
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("shorter than when the upload was declared"),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_refusals_show_the_code_and_create_no_note() {
    let data = dataset("refused");
    let srv = server(Mock::new(16)).await;
    // No mock route answers when the id space is empty and the path is wrong:
    // point the client at a base where start is a 404.
    let client = ApiClient::new(&RmsConfig {
        server_url: format!("{}/nope", srv.client.base_url()),
        token: None,
    });
    let opts = options(&data.0, &srv.state);

    let err = upload(&client, &ctx(true), &opts)
        .await
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("API error (404)") && err.contains("[NOT_FOUND]"),
        "{err}"
    );
    assert_eq!(std::fs::read_dir(&srv.state.0).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_is_sent_again_only_after_a_refusal_the_server_wrote() {
    // The server's own 503 says nothing was created: wait and start again.
    let data = dataset("start-retry");
    let mut mock = Mock::new(16);
    mock.start_refusals.push_back((503, "STORAGE_UNAVAILABLE"));
    let srv = server(mock).await;
    let opts = options(&data.0, &srv.state);

    let outcome = upload(&srv.client, &ctx(true), &opts).await.unwrap();

    assert_eq!(outcome.status, "ready");
    assert_eq!(
        srv.mock.lock().unwrap().count("POST /api/datasets/uploads"),
        2
    );

    // A proxy's 504 does not: the server may have created the dataset, and a
    // second start would create another one.
    let data = dataset("start-once");
    let mut mock = Mock::new(16);
    mock.start_refusals.push_back((504, ""));
    let srv = server(mock).await;
    let opts = options(&data.0, &srv.state);

    let err = upload(&srv.client, &ctx(true), &opts)
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("API error (504)"), "{err}");
    assert!(err.contains("may still have started the upload"), "{err}");
    let m = srv.mock.lock().unwrap();
    assert_eq!(m.count("POST /api/datasets/uploads"), 1);
    assert!(m.puts().is_empty());
    assert_eq!(std::fs::read_dir(&srv.state.0).unwrap().count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_small_chunk_size_is_refused_before_any_request() {
    let data = dataset("smallchunk");
    let srv = server(Mock::new(16)).await;
    let mut opts = options(&data.0, &srv.state);
    opts.chunk_size = Some(1024 * 1024);

    let err = upload(&srv.client, &ctx(true), &opts)
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("at least 5 MiB"), "{err}");
    assert!(srv.mock.lock().unwrap().log.is_empty());
}

// ── cancel / status ─────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_is_gated_then_removes_the_upload_and_its_note() {
    let srv = server(Mock::new(16)).await;
    {
        let mut m = srv.mock.lock().unwrap();
        m.files = vec![MockFile {
            path: "meta/info.json".into(),
            size: 3,
            sha256: None,
        }];
        m.state = "uploading".into();
        m.status = "uploading".into();
    }
    let store = StateStore::new(
        Some(srv.state.0.clone()),
        srv.client.base_url(),
        Path::new("/data/set"),
    );
    store.save(DS, "set");

    // without --yes: described, nothing changed
    cancel(&srv.client, &ctx(false), DS, Some(srv.state.0.clone()))
        .await
        .unwrap();
    assert_eq!(srv.mock.lock().unwrap().state, "uploading");
    assert!(store.load().is_some());

    cancel(&srv.client, &ctx(true), DS, Some(srv.state.0.clone()))
        .await
        .unwrap();
    assert_eq!(srv.mock.lock().unwrap().state, "cancelled");
    assert!(store.load().is_none());

    // cancelled means gone
    let err = cancel(&srv.client, &ctx(true), DS, Some(srv.state.0.clone()))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("API error (404)"), "{err}");
    let err = status(&srv.client, &ctx(true), DS)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("API error (404)"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_refuses_an_upload_that_already_became_a_dataset() {
    let srv = server(Mock::new(16)).await;
    {
        let mut m = srv.mock.lock().unwrap();
        m.files = vec![MockFile {
            path: "meta/info.json".into(),
            size: 3,
            sha256: None,
        }];
        m.state = "completed".into();
        m.status = "ready".into();
    }
    let err = cancel(&srv.client, &ctx(true), DS, Some(srv.state.0.clone()))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("is complete"), "{err}");
    assert!(
        !srv.mock
            .lock()
            .unwrap()
            .log
            .iter()
            .any(|l| l.ends_with("/cancel"))
    );

    srv.mock.lock().unwrap().state = "processing".into();
    srv.mock.lock().unwrap().polls_before_verdict = 1_000_000;
    let err = cancel(&srv.client, &ctx(true), DS, Some(srv.state.0.clone()))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("cannot be cancelled now"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_reads_an_open_upload() {
    let srv = server(Mock::new(16)).await;
    {
        let mut m = srv.mock.lock().unwrap();
        m.files = vec![MockFile {
            path: "videos/a.mp4".into(),
            size: 40,
            sha256: None,
        }];
        m.state = "uploading".into();
        m.status = "uploading".into();
        m.parts.insert((0, 2), vec![0; 16]);
    }
    colored::control::set_override(false);
    // both renderings run against the same body without error
    status(&srv.client, &ctx(true), DS).await.unwrap();
    let text = Ctx {
        json: false,
        yes: false,
        page: None,
        limit: None,
    };
    status(&srv.client, &text, DS).await.unwrap();
    assert_eq!(
        srv.mock
            .lock()
            .unwrap()
            .count(&format!("GET /api/datasets/{DS}/upload")),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_deletes_an_uploaded_dataset_that_failed_validation() {
    let srv = server(Mock::new(16)).await;
    {
        let mut m = srv.mock.lock().unwrap();
        m.files = vec![MockFile {
            path: "meta/info.json".into(),
            size: 3,
            sha256: None,
        }];
        m.state = "completed".into();
        m.status = "failed".into();
    }
    let notes = Some(srv.state.0.clone());

    // gated like every other removal
    cancel(&srv.client, &ctx(false), DS, notes.clone())
        .await
        .unwrap();
    assert_eq!(srv.mock.lock().unwrap().state, "completed");

    cancel(&srv.client, &ctx(true), DS, notes).await.unwrap();
    let m = srv.mock.lock().unwrap();
    assert_eq!(m.state, "cancelled");
    // the server answers 409 UPLOAD_COMPLETED to a cancel here; the dataset
    // route is the one that removes it
    assert_eq!(m.count(&format!("DELETE /api/datasets/{DS}")), 1);
    assert!(!m.log.iter().any(|l| l.ends_with("/upload/cancel")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refusal_before_the_body_is_read_stops_the_stream_and_the_count() {
    const MIB: usize = 1024 * 1024;
    let data = dataset("early");
    data.write("videos/cam/chunk-000/file-001.mp4", &pattern(32 * MIB, 5));
    let mut mock = Mock::new(16 * MIB as u64);
    mock.early_busy = 3;
    let srv = server(mock).await;
    let mut opts = options(&data.0, &srv.state);
    opts.parallel = 2;
    opts.no_checksum = true;

    let outcome = upload(&srv.client, &ctx(true), &opts).await.unwrap();

    assert_eq!(outcome.status, "ready");
    // The refused connections are still being drained by the mock; wait for
    // all three to report what arrived on them.
    for _ in 0..500 {
        if srv.mock.lock().unwrap().poured_after_refusal.len() == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let m = srv.mock.lock().unwrap();
    assert_eq!(m.early_busy, 0);
    assert_stored_matches_disk(&m, &data.0);
    // refused attempts are taken back out of the count: no 120 %
    let total: u64 = m.files.iter().map(|f| f.size).sum();
    assert_eq!(outcome.bytes_sent, total);
    // and the client did not go on pushing the refused 16 MiB part: what
    // still arrived is what was already in the socket buffers
    assert_eq!(m.poured_after_refusal.len(), 3);
    for poured in &m.poured_after_refusal {
        assert!(*poured < 8 * MIB, "{poured} bytes sent after the refusal");
    }
}
