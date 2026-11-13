//! `rmsctl datasets upload | upload-status | upload-cancel`
//!
//! Sends a LeRobot dataset directory straight into an RMS instance — no
//! Hugging Face repository in between. The contract is the server's
//! `docs/dataset-upload.md`:
//!
//! ```text
//! POST /datasets/uploads                             declare the files
//! PUT  /datasets/:id/upload/files/:fileId/parts/:n   one part, raw body
//! GET  /datasets/:id/upload                          what the server holds
//! POST /datasets/:id/upload/finish                   every part is sent
//! POST /datasets/:id/upload/cancel                   stop and remove everything
//! ```
//!
//! Three properties shape the code below:
//!
//! - **Bounded memory.** A part is never held in memory. It is read from disk
//!   once to hash it (`X-Chunk-Sha256`) and streamed from disk a second time
//!   as the request body, so resident memory does not grow with the part
//!   size, the file size or `--parallel`.
//! - **The server is the source of truth for resume.** The only thing kept
//!   locally is which dataset id belongs to which directory. What is already
//!   stored is always asked of the server (`receivedParts`), which is correct
//!   after either side restarted.
//! - **Every request is small and quick.** One part per request, finish
//!   returns at once, and the verdict is polled — the instance sits behind a
//!   proxy that caps a body at 100 MB and a request at about 100 s.

use crate::client::{ApiClient, RawResponse};
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::output;
use colored::*;
use futures_util::{StreamExt, TryStreamExt, stream};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{IsTerminal, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

/// S3 refuses a multipart part under 5 MiB (except the last), so the server
/// refuses a smaller `chunkSize`. Checked here too so the mistake costs no
/// round trip.
pub const MIN_CHUNK_BYTES: u64 = 5 * 1024 * 1024;

/// How often the server may answer `UPLOAD_BUSY` for one part before giving
/// up. Busy is not a failure — another upload holds the slots — so it has its
/// own, much larger budget than `--retries`.
const MAX_BUSY_WAITS: u32 = 300;

/// Rounds of "send what is missing, then finish". A second round only happens
/// when finish answers `UPLOAD_INCOMPLETE`, i.e. the store lost a part the
/// server had acknowledged.
const MAX_FINISH_ROUNDS: usize = 3;

// ── Options ─────────────────────────────────────────────────────────────

pub struct UploadOptions {
    pub dir: PathBuf,
    pub name: Option<String>,
    pub description: Option<String>,
    pub robot_type: Option<String>,
    pub chunk_size: Option<u64>,
    pub parallel: usize,
    pub retries: u32,
    pub resume: Option<String>,
    pub restart: bool,
    pub no_checksum: bool,
    pub no_wait: bool,
    pub wait_timeout: Duration,
    pub part_timeout: Duration,
    /// Which files of the directory go.
    pub rules: ScanRules,
    /// Where the "which upload belongs to this directory" notes live. `None`
    /// resolves the platform state directory.
    pub state_dir: Option<PathBuf>,
    pub tuning: Tuning,
}

/// Pauses, separated out so the tests do not sleep for real.
#[derive(Clone, Copy)]
pub struct Tuning {
    pub backoff_base: Duration,
    pub max_pause: Duration,
    pub poll_interval: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            backoff_base: Duration::from_secs(1),
            max_pause: Duration::from_secs(30),
            poll_interval: Duration::from_secs(2),
        }
    }
}

/// What an upload run ended with. The command itself only needs "did it
/// succeed"; the fields are what the tests assert on.
#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct UploadOutcome {
    pub dataset_id: String,
    /// The dataset row's status when the command stopped watching.
    pub status: String,
    pub parts_sent: usize,
    pub bytes_sent: u64,
    /// False when the command described the action and stopped at the gate.
    pub executed: bool,
}

/// Parse `--chunk-size`: plain bytes, or a number with K/M/G (binary units —
/// `32M` is 32 MiB, which is how the server counts a part).
pub fn parse_size(s: &str) -> Result<u64, String> {
    let t = s.trim().to_ascii_lowercase();
    let t = t
        .strip_suffix("ib")
        .or_else(|| t.strip_suffix('b'))
        .unwrap_or(&t);
    let (num, mult) = match t.chars().last() {
        Some('k') => (&t[..t.len() - 1], 1024u64),
        Some('m') => (&t[..t.len() - 1], 1024 * 1024),
        Some('g') => (&t[..t.len() - 1], 1024 * 1024 * 1024),
        _ => (t, 1),
    };
    let n: f64 = num
        .trim()
        .parse()
        .map_err(|_| format!("'{s}' is not a size — use bytes or e.g. 16M"))?;
    if !n.is_finite() || n <= 0.0 {
        return Err(format!("'{s}' is not a positive size"));
    }
    Ok((n * mult as f64).round() as u64)
}

// ── Which files of the directory go ─────────────────────────────────────

/// The files the server *reads* — the rule its Hugging Face importer selects
/// by. Everything else in the directory is uploaded as well (a server before
/// RMS TASK-500 refused to store it, see `start`); this is what
/// `--lerobot-only` narrows the upload to.
pub fn is_dataset_file(rel: &str) -> bool {
    if rel.starts_with("meta/") {
        return true;
    }
    if rel.starts_with("data/") {
        return rel.ends_with(".parquet");
    }
    if rel.starts_with("videos/") {
        return rel.ends_with(".mp4");
    }
    rel.starts_with("images/")
}

/// Why an entry called `name` is never dataset content, or `None`.
///
/// Version-control data, OS and interpreter droppings, and this tool's own
/// unfinished downloads — at any depth. `.cache` only at the top: that is
/// where `huggingface-cli download --local-dir` keeps its bookkeeping.
/// `--no-default-excludes` switches these off, except the last: `datasets
/// download` refuses to write a name ending in `.rmsctl-part`, so one that
/// was uploaded could never come back.
pub(super) fn junk_reason(name: &str, top_level: bool, defaults: bool) -> Option<&'static str> {
    if name.ends_with(super::dataset_download::PART_SUFFIX) {
        return Some("an unfinished rmsctl download");
    }
    if !defaults {
        return None;
    }
    match name {
        ".git" => Some("version-control data"),
        ".DS_Store" => Some("macOS Finder metadata"),
        "__pycache__" => Some("Python bytecode cache"),
        ".cache" if top_level => Some("Hugging Face download cache"),
        _ => None,
    }
}

/// Why the server would refuse `rel` as a manifest path, or `None`. Checked
/// here so one odd file name is named before anything is hashed or sent,
/// instead of failing the whole start request.
pub(super) fn unstorable_reason(rel: &str) -> Option<&'static str> {
    if rel.len() > 1024 {
        return Some("its path is longer than 1024 bytes");
    }
    if rel.chars().any(|c| c.is_control()) {
        return Some("its name contains a control character");
    }
    if rel.contains('\\') {
        return Some("its name contains a backslash");
    }
    let b = rel.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return Some("its path starts like a drive letter (X:)");
    }
    None
}

/// What decides which files of a directory are uploaded (and, in `datasets
/// verify`, compared).
#[derive(Debug, Clone, Default)]
pub struct ScanRules {
    /// `--exclude` patterns, in `models upload`'s syntax.
    pub exclude: Vec<String>,
    /// Only the LeRobot tree: `meta/**`, `data/**/*.parquet`,
    /// `videos/**/*.mp4`, `images/**`.
    pub lerobot_only: bool,
    /// Do not skip `.git`, `.DS_Store`, `__pycache__` and a top-level `.cache`.
    pub no_default_excludes: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LocalFile {
    /// Path relative to the upload root, forward slashes.
    pub(super) rel: String,
    pub(super) abs: PathBuf,
    pub(super) size: u64,
}

/// One entry of the directory that is not uploaded, and why. Nothing is left
/// out without an entry here.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Skipped {
    /// Relative path; a directory ends in `/` and stands for all it holds.
    pub(super) rel: String,
    pub(super) why: String,
}

#[derive(Debug, Default)]
pub(super) struct Scan {
    pub(super) files: Vec<LocalFile>,
    pub(super) skipped: Vec<Skipped>,
}

impl Scan {
    pub(super) fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

pub(super) fn io_err(what: &str, path: &Path, e: std::io::Error) -> RmsError {
    RmsError::Other(format!("{what} {}: {e}", path.display()))
}

/// Every regular file under `root` that `rules` lets through, sorted by path,
/// plus the list of what was left out.
pub(super) fn scan_dir(root: &Path, rules: &ScanRules) -> RmsResult<Scan> {
    let mut scan = Scan::default();
    walk(root, "", 0, rules, &mut scan)?;
    scan.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    scan.skipped.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(scan)
}

/// Walks `dir`; returns how many entries it holds, so the caller can tell an
/// empty directory (which no file-by-file upload can carry) from a full one.
fn walk(
    dir: &Path,
    rel: &str,
    depth: usize,
    rules: &ScanRules,
    scan: &mut Scan,
) -> RmsResult<usize> {
    // The cap only exists to end a symlink loop.
    if depth > 32 {
        return Err(RmsError::InvalidInput(format!(
            "{} is nested more than 32 levels deep — is there a symlink loop?",
            dir.display()
        )));
    }
    let mut entries = 0;
    for entry in std::fs::read_dir(dir).map_err(|e| io_err("cannot read", dir, e))? {
        let entry = entry.map_err(|e| io_err("cannot read", dir, e))?;
        entries += 1;
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(RmsError::InvalidInput(format!(
                "{} has a name that is not valid UTF-8 and cannot be uploaded",
                path.display()
            )));
        };
        let child = if rel.is_empty() {
            name.to_string()
        } else {
            format!("{rel}/{name}")
        };
        // `metadata` follows symlinks on purpose: a Hub cache snapshot is a
        // tree of links into `blobs/`. A linked file is uploaded as the bytes
        // it points to.
        let meta = std::fs::metadata(&path);
        let shown = match &meta {
            Ok(m) if m.is_dir() => format!("{child}/"),
            _ => child.clone(),
        };
        // Junk and --exclude come before the stat is looked at, so a dangling
        // link inside `.git`, or one the user excluded, is not an error.
        if let Some(why) = junk_reason(name, rel.is_empty(), !rules.no_default_excludes) {
            scan.skipped.push(Skipped {
                rel: shown,
                why: why.to_string(),
            });
            continue;
        }
        if let Some(pattern) = rules
            .exclude
            .iter()
            .find(|p| super::model_upload::excluded(p, &child))
        {
            scan.skipped.push(Skipped {
                rel: shown,
                why: format!("--exclude {pattern}"),
            });
            continue;
        }
        let meta = meta.map_err(|e| {
            let link = std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
            if link {
                RmsError::InvalidInput(format!(
                    "{} is a symbolic link whose target cannot be read ({e}).\n  \
                     Fix or remove the link, or leave it out with --exclude '{child}'.",
                    path.display()
                ))
            } else {
                io_err("cannot stat", &path, e)
            }
        })?;
        if meta.is_dir() {
            if walk(&path, &child, depth + 1, rules, scan)? == 0 {
                scan.skipped.push(Skipped {
                    rel: shown,
                    why: "an empty directory — only files can be stored".into(),
                });
            }
        } else if !meta.is_file() {
            scan.skipped.push(Skipped {
                rel: shown,
                why: "not a regular file (socket, pipe or device)".into(),
            });
        } else if rules.lerobot_only && !is_dataset_file(&child) {
            scan.skipped.push(Skipped {
                rel: shown,
                why: "not part of the LeRobot tree (--lerobot-only)".into(),
            });
        } else if let Some(why) = unstorable_reason(&child) {
            return Err(RmsError::InvalidInput(format!(
                "{} cannot be stored: {why}.\n  \
                 Rename it, or leave it out with --exclude.",
                path.display()
            )));
        } else {
            scan.files.push(LocalFile {
                rel: child,
                abs: path,
                size: meta.len(),
            });
        }
    }
    Ok(entries)
}

/// The left-out entries as lines for the terminal: all of them, or the first
/// `limit` and a count of the rest.
pub(super) fn skipped_lines(skipped: &[Skipped], limit: usize) -> Vec<String> {
    let mut lines: Vec<String> = skipped
        .iter()
        .take(limit)
        .map(|s| format!("{}  ({})", s.rel, s.why))
        .collect();
    if skipped.len() > limit {
        lines.push(format!("… and {} more", skipped.len() - limit));
    }
    lines
}

fn read_info(root: &Path) -> RmsResult<Value> {
    let path = root.join("meta").join("info.json");
    let text = std::fs::read_to_string(&path).map_err(|e| {
        RmsError::InvalidInput(format!(
            "{} is not a LeRobot dataset root: cannot read meta/info.json ({e}). \
             Pass the directory that holds meta/ and data/.",
            root.display()
        ))
    })?;
    let info: Value = serde_json::from_str(&text).map_err(|e| {
        RmsError::InvalidInput(format!("{} is not valid JSON: {e}", path.display()))
    })?;
    if !info.is_object() {
        return Err(RmsError::InvalidInput(format!(
            "{} does not hold a JSON object",
            path.display()
        )));
    }
    Ok(info)
}

// ── Part arithmetic ─────────────────────────────────────────────────────

/// `max(1, ceil(size / chunk))` — an empty file still has one (empty) part.
pub fn part_count(size: u64, chunk: u64) -> u32 {
    if chunk == 0 {
        return 1;
    }
    size.div_ceil(chunk).max(1) as u32
}

/// Offset and length of part `n` (numbered from 1).
pub fn part_range(size: u64, chunk: u64, n: u32) -> (u64, u64) {
    let offset = (n as u64 - 1).saturating_mul(chunk).min(size);
    (offset, chunk.min(size - offset))
}

/// SHA-256 of `len` bytes starting at `offset`, read through a fixed buffer.
/// A file that ends early is an error: it changed after it was declared.
fn sha256_range(path: &Path, offset: u64, len: u64) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut left = len;
    while left > 0 {
        let want = left.min(buf.len() as u64) as usize;
        let got = file.read(&mut buf[..want])?;
        if got == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "the file is shorter than when the upload was declared",
            ));
        }
        hasher.update(&buf[..got]);
        left -= got as u64;
    }
    Ok(hex(&hasher.finalize()))
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(super) async fn sha256_range_async(path: PathBuf, offset: u64, len: u64) -> RmsResult<String> {
    let shown = path.clone();
    tokio::task::spawn_blocking(move || sha256_range(&path, offset, len))
        .await
        .map_err(|e| RmsError::Other(format!("hashing task failed: {e}")))?
        .map_err(|e| io_err("cannot read", &shown, e))
}

// ── Server shapes ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RemoteFile {
    pub(super) id: u64,
    pub(super) path: String,
    #[serde(default)]
    pub(super) size: u64,
    #[serde(default)]
    pub(super) part_count: Option<u32>,
    #[serde(default)]
    pub(super) complete: bool,
    #[serde(default)]
    pub(super) received_parts: Vec<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartResponse {
    dataset_id: String,
    chunk_size: u64,
    #[serde(default)]
    storage: Option<String>,
    #[serde(default)]
    files: Vec<RemoteFile>,
    #[serde(default)]
    ignored: Vec<IgnoredFile>,
}

#[derive(Debug, Deserialize)]
struct IgnoredFile {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadStatus {
    #[serde(default)]
    state: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    storage: Option<String>,
    #[serde(default)]
    chunk_size: u64,
    #[serde(default)]
    total_bytes: u64,
    #[serde(default)]
    received_bytes: u64,
    #[serde(default)]
    total_files: u64,
    #[serde(default)]
    completed_files: u64,
    #[serde(default)]
    ready_to_finish: bool,
    #[serde(default)]
    files: Vec<RemoteFile>,
    #[serde(default)]
    last_activity_at: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    import_error: Option<Value>,
}

impl UploadStatus {
    /// The reason the server recorded for a failure, if any.
    fn failure(&self) -> Option<String> {
        let e = self.import_error.as_ref()?;
        match e {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => {
                let msg = o.get("error").and_then(|m| m.as_str())?;
                Some(match o.get("phase").and_then(|p| p.as_str()) {
                    Some(phase) => format!("{msg} (while {phase})"),
                    None => msg.to_string(),
                })
            }
            _ => None,
        }
    }
}

// ── Errors and retries ──────────────────────────────────────────────────

#[derive(Debug, PartialEq, Clone, Copy)]
pub(super) enum Retry {
    /// Other uploads hold the part slots: wait, does not count as a failure.
    Busy,
    /// Worth sending again after a pause.
    Transient,
    /// Sending it again cannot help.
    Fatal,
}

/// Which refusals are worth another attempt. Branches on the status and the
/// server's stable `code`, never on the sentence.
pub(super) fn classify(status: u16, code: Option<&str>) -> Retry {
    match (status, code) {
        (503, Some("UPLOAD_BUSY")) => Retry::Busy,
        (400, Some("PART_INTERRUPTED")) | (422, Some("CHECKSUM_MISMATCH")) => Retry::Transient,
        (408 | 425 | 429, _) => Retry::Transient,
        // 507 is "the store is full": no pause fixes that.
        (501 | 505 | 507, _) => Retry::Fatal,
        (500..=599, _) => Retry::Transient,
        _ => Retry::Fatal,
    }
}

/// Exponential pause with a little jitter, never shorter than the server's
/// `Retry-After`, never longer than `max_pause`.
pub(super) fn backoff(tuning: &Tuning, attempt: u32, retry_after: Option<u64>) -> Duration {
    let exp = tuning
        .backoff_base
        .saturating_mul(1u32 << attempt.saturating_sub(1).min(8));
    let asked = retry_after.map(Duration::from_secs).unwrap_or_default();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let jitter = exp.mul_f64((nanos % 1000) as f64 / 4000.0);
    (exp + jitter).max(asked).min(tuning.max_pause)
}

/// Turn a refusal into the error the user reads: the server's sentence, its
/// stable code, and the context fields that say what to change.
pub(super) fn api_error(r: &RawResponse) -> RmsError {
    let body = r.json();
    let sentence = body
        .get("error")
        .or_else(|| body.get("message"))
        .and_then(|m| m.as_str())
        .map(String::from)
        .unwrap_or_else(|| {
            if r.body.trim().is_empty() {
                "request failed".to_string()
            } else {
                output::clip(r.body.trim(), 200)
            }
        });
    let mut message = sentence;
    if let Some(code) = body.get("code").and_then(|c| c.as_str()) {
        message.push_str(&format!(" [{code}]"));
    }
    if let Some(obj) = body.as_object() {
        let context: serde_json::Map<String, Value> = obj
            .iter()
            .filter(|(k, _)| !matches!(k.as_str(), "error" | "code" | "message"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if !context.is_empty() {
            let text = Value::Object(context).to_string();
            message.push_str(&format!("\n  {}", output::clip(&text, 600)));
        }
    }
    RmsError::Api {
        status: r.status,
        message,
    }
}

/// A JSON control call (start, status, finish, cancel) that waits out rate
/// limits and gateway hiccups.
///
/// `retry_network` is false for start: a request that died on the wire may
/// have created the dataset, and sending it again would create a second one.
pub(super) async fn control(
    client: &ApiClient,
    method: Method,
    path: &str,
    body: Option<&Value>,
    retry_network: bool,
    tuning: &Tuning,
) -> RmsResult<RawResponse> {
    const ATTEMPTS: u32 = 6;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let mut req = client
            .request(method.clone(), path)
            .timeout(Duration::from_secs(90));
        if let Some(b) = body {
            req = req.json(b);
        }
        match client.send_raw(req).await {
            Ok(r) if r.is_success() => return Ok(r),
            Ok(r) => {
                let code = r.code();
                // On a control call a 5xx with a code is the server's own
                // verdict (STORAGE_UNAVAILABLE aside), not a hiccup.
                //
                // A 502/503/504 without a code comes from a proxy, and says
                // nothing about whether the server acted. A call that must not
                // run twice (start) is therefore only re-sent after a refusal
                // the server itself wrote — that one created nothing.
                let from_server = code.as_deref().is_some_and(|c| !c.is_empty());
                let again = match r.status {
                    429 => true,
                    502..=504 => {
                        classify(r.status, code.as_deref()) != Retry::Fatal
                            && (retry_network || from_server)
                    }
                    _ => false,
                };
                if !again || attempt >= ATTEMPTS {
                    return Ok(r);
                }
                tokio::time::sleep(backoff(tuning, attempt, r.retry_after)).await;
            }
            Err(e) => {
                if !retry_network || attempt >= ATTEMPTS {
                    return Err(e);
                }
                tokio::time::sleep(backoff(tuning, attempt, None)).await;
            }
        }
    }
}

pub(super) fn parse<T: serde::de::DeserializeOwned>(r: &RawResponse) -> RmsResult<T> {
    serde_json::from_str(&r.body).map_err(|e| {
        RmsError::Parse(format!(
            "{e}\n  The server sent a shape this command does not understand.\n  Body: {}",
            output::clip(&r.body, 600)
        ))
    })
}

/// The upload the server holds for a dataset. `None` when the dataset is gone
/// (cancelled, expired, deleted, or another organization's).
async fn fetch_status(
    client: &ApiClient,
    id: &str,
    tuning: &Tuning,
) -> RmsResult<Option<(UploadStatus, Value)>> {
    let r = control(
        client,
        Method::GET,
        &format!("/datasets/{id}/upload"),
        None,
        true,
        tuning,
    )
    .await?;
    if r.status == 404 && r.code().as_deref() != Some("NO_UPLOAD") {
        return Ok(None);
    }
    if !r.is_success() {
        return Err(api_error(&r));
    }
    Ok(Some((parse(&r)?, r.json())))
}

// ── The note that makes "run it again" resume ───────────────────────────

/// Which dataset an upload of a directory to a server was started as. Kept
/// outside the dataset directory — that is often read-only and never ours to
/// write into. It holds no progress: what is stored is asked of the server.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(super) struct SavedUpload {
    server: String,
    dir: String,
    /// The id the server gave the upload (a dataset id, or a model upload id).
    pub(super) dataset_id: String,
    pub(super) name: String,
}

pub(super) struct StateStore {
    dir: Option<PathBuf>,
    server: String,
    source: String,
}

fn default_state_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RMSCTL_STATE_DIR")
        && !p.trim().is_empty()
    {
        return Some(PathBuf::from(p));
    }
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|d| d.join("rmsctl").join("uploads"))
}

impl StateStore {
    pub(super) fn new(dir: Option<PathBuf>, server: &str, source: &Path) -> Self {
        Self {
            dir: dir.or_else(default_state_dir),
            server: server.to_string(),
            source: source.display().to_string(),
        }
    }

    fn path(&self) -> Option<PathBuf> {
        let mut h = Sha256::new();
        h.update(self.server.as_bytes());
        h.update(b"\n");
        h.update(self.source.as_bytes());
        let key = hex(&h.finalize());
        Some(self.dir.as_ref()?.join(format!("{}.json", &key[..24])))
    }

    pub(super) fn load(&self) -> Option<SavedUpload> {
        let text = std::fs::read_to_string(self.path()?).ok()?;
        let saved: SavedUpload = serde_json::from_str(&text).ok()?;
        (saved.server == self.server && saved.dir == self.source).then_some(saved)
    }

    /// Best effort: an upload that cannot be noted still uploads; it just has
    /// to be resumed with `--resume <id>`. Returns whether the note was kept.
    pub(super) fn save(&self, dataset_id: &str, name: &str) -> bool {
        let Some(path) = self.path() else {
            return false;
        };
        let saved = SavedUpload {
            server: self.server.clone(),
            dir: self.source.clone(),
            dataset_id: dataset_id.to_string(),
            name: name.to_string(),
        };
        let write = || -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, serde_json::to_vec_pretty(&saved)?)
        };
        write().is_ok()
    }

    pub(super) fn remove(&self) {
        if let Some(path) = self.path() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Drop every note that points at `dataset_id` on `server` — after a cancel,
/// which is addressed by id and does not know the directory.
pub(super) fn forget_dataset(dir: Option<PathBuf>, server: &str, dataset_id: &str) {
    let Some(dir) = dir.or_else(default_state_dir) else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(saved) = serde_json::from_str::<SavedUpload>(&text)
            && saved.server == server
            && saved.dataset_id == dataset_id
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

// ── Plan: which parts still have to go ──────────────────────────────────

#[derive(Debug)]
pub(super) struct PlanFile {
    id: u64,
    rel: String,
    abs: PathBuf,
    size: u64,
    missing: Vec<u32>,
}

#[derive(Debug)]
pub(super) struct Plan {
    chunk_size: u64,
    files: Vec<PlanFile>,
    pub(super) total_bytes: u64,
}

impl Plan {
    pub(super) fn missing_parts(&self) -> usize {
        self.files.iter().map(|f| f.missing.len()).sum()
    }

    pub(super) fn missing_bytes(&self) -> u64 {
        self.files
            .iter()
            .map(|f| {
                f.missing
                    .iter()
                    .map(|n| part_range(f.size, self.chunk_size, *n).1)
                    .sum::<u64>()
            })
            .sum()
    }
}

/// Match what the server expects against what is on disk and list the parts
/// it does not hold yet. Refuses when the two disagree: a directory that
/// changed since the upload was declared cannot be resumed into it.
pub(super) fn build_plan(
    local: &[LocalFile],
    remote: &[RemoteFile],
    ignored: &HashSet<String>,
    chunk_size: u64,
) -> RmsResult<Plan> {
    if chunk_size == 0 {
        return Err(RmsError::Parse(
            "the server reported a chunkSize of 0".into(),
        ));
    }
    let by_path: HashMap<&str, &LocalFile> = local.iter().map(|f| (f.rel.as_str(), f)).collect();
    let mut files = Vec::with_capacity(remote.len());
    let mut total_bytes = 0;
    for r in remote {
        let Some(l) = by_path.get(r.path.as_str()) else {
            return Err(changed(format!(
                "{} is part of the upload but no longer in the directory",
                r.path
            )));
        };
        if l.size != r.size {
            return Err(changed(format!(
                "{} is {} bytes on disk but was declared as {}",
                r.path, l.size, r.size
            )));
        }
        let parts = part_count(r.size, chunk_size);
        let missing: Vec<u32> = if r.complete {
            Vec::new()
        } else {
            let held: HashSet<u32> = r.received_parts.iter().copied().collect();
            (1..=parts).filter(|n| !held.contains(n)).collect()
        };
        total_bytes += r.size;
        files.push(PlanFile {
            id: r.id,
            rel: r.path.clone(),
            abs: l.abs.clone(),
            size: r.size,
            missing,
        });
    }
    let declared: HashSet<&str> = remote.iter().map(|r| r.path.as_str()).collect();
    if let Some(extra) = local
        .iter()
        .find(|l| !declared.contains(l.rel.as_str()) && !ignored.contains(&l.rel))
    {
        return Err(changed(format!(
            "{} is in the directory but was not declared when the upload started",
            extra.rel
        )));
    }
    Ok(Plan {
        chunk_size,
        files,
        total_bytes,
    })
}

fn changed(what: String) -> RmsError {
    RmsError::InvalidInput(format!(
        "the directory no longer matches the upload: {what}.\n  \
         Start over with --restart (cancels the stored parts), or restore the file."
    ))
}

// ── Progress ────────────────────────────────────────────────────────────

pub(super) struct Progress {
    total: u64,
    /// Bytes the server already held when this run began.
    base: u64,
    /// Bytes handed to the network in this run (rolled back when a part fails).
    pub(super) sent: AtomicU64,
    files_total: usize,
    files_done: AtomicUsize,
    /// Parts each file still needs, by index into the plan.
    remaining: Vec<AtomicUsize>,
    pub(super) parts_sent: AtomicUsize,
    pub(super) started: Instant,
}

impl Progress {
    pub(super) fn new(plan: &Plan) -> Self {
        let missing = plan.missing_bytes();
        Self {
            total: plan.total_bytes,
            base: plan.total_bytes.saturating_sub(missing),
            sent: AtomicU64::new(0),
            files_total: plan.files.len(),
            files_done: AtomicUsize::new(
                plan.files.iter().filter(|f| f.missing.is_empty()).count(),
            ),
            remaining: plan
                .files
                .iter()
                .map(|f| AtomicUsize::new(f.missing.len()))
                .collect(),
            parts_sent: AtomicUsize::new(0),
            started: Instant::now(),
        }
    }

    pub(super) fn line(&self) -> String {
        let sent = self.sent.load(Ordering::Relaxed);
        let done = (self.base + sent).min(self.total);
        let secs = self.started.elapsed().as_secs_f64();
        let rate = if secs > 0.5 { sent as f64 / secs } else { 0.0 };
        progress_line(
            done,
            self.total,
            self.files_done.load(Ordering::Relaxed),
            self.files_total,
            rate,
        )
    }
}

/// `1.2 GB`-style size, decimal units like the dataset pages show.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit < UNITS.len() - 1 {
        v /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

/// A part size, which is a binary quantity: `32 MiB`, not `33.6 MB`.
pub(super) fn human_chunk(n: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if n >= MIB && n.is_multiple_of(MIB) {
        format!("{} MiB", n / MIB)
    } else {
        human_bytes(n)
    }
}

pub(super) fn human_duration(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "-".into();
    }
    let s = secs.round() as u64;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m{:02}s", s / 60, s % 60),
        _ => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
    }
}

/// One progress line: bar, percentage, bytes, files, rate and ETA.
fn progress_line(
    done: u64,
    total: u64,
    files_done: usize,
    files_total: usize,
    rate: f64,
) -> String {
    transfer_line("Uploading", done, total, files_done, files_total, rate)
}

/// The progress line of a transfer in either direction; `label` says which.
pub(super) fn transfer_line(
    label: &str,
    done: u64,
    total: u64,
    files_done: usize,
    files_total: usize,
    rate: f64,
) -> String {
    let pct = if total > 0 {
        (done as f64 / total as f64).min(1.0)
    } else {
        1.0
    };
    let filled = ((pct * 20.0) as usize).min(20);
    let eta = if done >= total {
        "done".to_string()
    } else if rate > 0.0 {
        format!("ETA {}", human_duration((total - done) as f64 / rate))
    } else {
        "ETA -".to_string()
    };
    let speed = if rate > 0.0 {
        format!("{}/s", human_bytes(rate as u64))
    } else {
        "- /s".to_string()
    };
    format!(
        "    {} {} {}{} {:>5.1}%  {} / {} · {}/{} files · {} · {}",
        "│".bright_black(),
        format!("{label:<14}").bright_black(),
        "█".repeat(filled).truecolor(255, 103, 0),
        "░".repeat(20 - filled).bright_black(),
        pct * 100.0,
        human_bytes(done),
        human_bytes(total),
        files_done,
        files_total,
        speed,
        eta
    )
}

/// What the command prints. `--json` keeps stdout for the final payload, so
/// everything here goes quiet.
pub(super) struct Ui {
    pub(super) quiet: bool,
    pub(super) tty: bool,
}

impl Ui {
    pub(super) fn new(quiet: bool) -> Self {
        Self {
            quiet,
            tty: std::io::stdout().is_terminal(),
        }
    }

    pub(super) fn info(&self, msg: &str) {
        if !self.quiet {
            output::info(msg);
        }
    }

    pub(super) fn success(&self, msg: &str) {
        if !self.quiet {
            output::success(msg);
        }
    }

    pub(super) fn warn(&self, msg: &str) {
        if !self.quiet {
            output::warn(msg);
        }
    }

    pub(super) fn field(&self, label: &str, value: &str) {
        if !self.quiet {
            output::field(label, value);
        }
    }

    /// Redraw the progress line in place on a terminal; in a log file, write a
    /// new line each time instead.
    pub(super) fn progress(&self, line: &str) {
        if self.quiet {
            return;
        }
        if self.tty {
            print!("\r\x1b[2K{line}");
            let _ = std::io::stdout().flush();
        } else {
            println!("{line}");
        }
    }

    pub(super) fn progress_done(&self, line: &str) {
        if self.quiet {
            return;
        }
        if self.tty {
            println!("\r\x1b[2K{line}");
        } else {
            println!("{line}");
        }
    }

    pub(super) async fn tick(&self, progress: &Progress) {
        // A terminal gets a smooth bar; a log gets a line every few seconds.
        let every = if self.tty {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(5)
        };
        loop {
            tokio::time::sleep(every).await;
            self.progress(&progress.line());
        }
    }
}

// ── Sending parts ───────────────────────────────────────────────────────

struct PartFailure {
    retry: Retry,
    retry_after: Option<u64>,
    error: RmsError,
}

/// One attempt's share of the progress counter.
#[derive(Default)]
struct Attempt {
    counted: u64,
    over: bool,
}

struct PartJob<'a> {
    client: &'a ApiClient,
    /// The upload's own path: parts live at `<base>/files/:fileId/parts/:n`.
    base: &'a str,
    file: &'a PlanFile,
    chunk_size: u64,
    part: u32,
    timeout: Duration,
}

/// One attempt at one part: hash it, then stream the same byte range from
/// disk as the body. Nothing larger than a read buffer is held in memory.
async fn try_part(job: &PartJob<'_>, progress: &Arc<Progress>) -> Result<(), PartFailure> {
    let fatal = |error: RmsError| PartFailure {
        retry: Retry::Fatal,
        retry_after: None,
        error,
    };
    let (offset, len) = part_range(job.file.size, job.chunk_size, job.part);
    let sha = sha256_range_async(job.file.abs.clone(), offset, len)
        .await
        .map_err(fatal)?;

    // Counted as the bytes leave for the socket, and taken back if the
    // attempt fails, so the bar never claims a part the server did not keep.
    //
    // `over` matters because the server answers most refusals (UPLOAD_BUSY
    // above all) before it reads the body, and the HTTP client would go on
    // feeding the rest of the part to a server that is not listening. Once
    // the answer is in, the stream ends itself: nothing more is read from
    // disk, counted, or pushed up a slow link.
    let attempt = Arc::new(std::sync::Mutex::new(Attempt::default()));
    let body = if len == 0 {
        reqwest::Body::from(Vec::new())
    } else {
        let mut file = tokio::fs::File::open(&job.file.abs)
            .await
            .map_err(|e| fatal(io_err("cannot open", &job.file.abs, e)))?;
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| fatal(io_err("cannot seek in", &job.file.abs, e)))?;
        let attempt = attempt.clone();
        let progress = progress.clone();
        let stream = ReaderStream::with_capacity(file.take(len), 256 * 1024).map(move |chunk| {
            let bytes = chunk?;
            let mut a = attempt.lock().unwrap_or_else(|e| e.into_inner());
            if a.over {
                return Err(std::io::Error::other("the server already answered"));
            }
            a.counted += bytes.len() as u64;
            progress
                .sent
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            Ok(bytes)
        });
        reqwest::Body::wrap_stream(stream)
    };

    let path = format!("{}/files/{}/parts/{}", job.base, job.file.id, job.part);
    // Content-Length is set by hand: a streamed body would otherwise go out
    // chunked, which the server answers with 411.
    let req = job
        .client
        .request(Method::PUT, &path)
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .header(reqwest::header::CONTENT_LENGTH, len)
        .header("X-Chunk-Sha256", &sha)
        .timeout(job.timeout)
        .body(body);

    let answer = job.client.send_raw(req).await;
    // Close the attempt: what it counted is final from here on.
    let counted = {
        let mut a = attempt.lock().unwrap_or_else(|e| e.into_inner());
        a.over = true;
        a.counted
    };
    let rollback = || {
        progress.sent.fetch_sub(counted, Ordering::Relaxed);
    };

    match answer {
        Ok(r) if r.status == 200 => {
            // The server answers with the hash of what it stored. It checked
            // it against the header already; comparing again costs nothing
            // and catches a proxy that answered for it.
            match r.json().get("sha256").and_then(|s| s.as_str()) {
                Some(stored) if !stored.eq_ignore_ascii_case(&sha) => {
                    rollback();
                    Err(PartFailure {
                        retry: Retry::Transient,
                        retry_after: None,
                        error: RmsError::Other(format!(
                            "the server stored {stored} but the part hashes to {sha}"
                        )),
                    })
                }
                // Cannot happen with this server (it reads the whole part
                // before it answers 200), so something in between answered.
                _ if counted != len => {
                    rollback();
                    Err(PartFailure {
                        retry: Retry::Transient,
                        retry_after: None,
                        error: RmsError::Other(format!(
                            "got a 200 after only {counted} of {len} bytes were sent"
                        )),
                    })
                }
                _ => Ok(()),
            }
        }
        Ok(r) => {
            rollback();
            Err(PartFailure {
                retry: classify(r.status, r.code().as_deref()),
                retry_after: r.retry_after,
                error: api_error(&r),
            })
        }
        Err(error) => {
            rollback();
            Err(PartFailure {
                retry: Retry::Transient,
                retry_after: None,
                error,
            })
        }
    }
}

async fn send_part(
    job: PartJob<'_>,
    progress: &Arc<Progress>,
    retries: u32,
    tuning: &Tuning,
) -> RmsResult<()> {
    let mut failures = 0u32;
    let mut busy = 0u32;
    loop {
        let failure = match try_part(&job, progress).await {
            Ok(()) => return Ok(()),
            Err(f) => f,
        };
        let give_up = |why: &str| {
            RmsError::Other(format!(
                "part {} of {} {why}: {}",
                job.part, job.file.rel, failure.error
            ))
        };
        match failure.retry {
            Retry::Fatal => return Err(failure.error),
            Retry::Busy => {
                busy += 1;
                if busy > MAX_BUSY_WAITS {
                    return Err(give_up("was refused as busy too many times"));
                }
                let pause = Duration::from_secs(failure.retry_after.unwrap_or(2));
                tokio::time::sleep(pause.min(tuning.max_pause)).await;
            }
            Retry::Transient => {
                failures += 1;
                if failures > retries {
                    let mut error = give_up(&format!("failed after {failures} attempts"));
                    // The one cause the user can fix from here: a part that
                    // cannot cross a slow uplink inside the proxy's timeout.
                    if let RmsError::Other(text) = &mut error {
                        text.push_str(
                            "\n  If parts time out on a slow uplink, start over with smaller ones: --restart --chunk-size 8M",
                        );
                    }
                    return Err(error);
                }
                tokio::time::sleep(backoff(tuning, failures, failure.retry_after)).await;
            }
        }
    }
}

/// What sending parts needs to know, whatever is being uploaded.
#[derive(Clone, Copy)]
pub(super) struct SendOptions {
    pub(super) parallel: usize,
    pub(super) retries: u32,
    pub(super) part_timeout: Duration,
    pub(super) tuning: Tuning,
}

impl UploadOptions {
    fn send(&self) -> SendOptions {
        SendOptions {
            parallel: self.parallel,
            retries: self.retries,
            part_timeout: self.part_timeout,
            tuning: self.tuning,
        }
    }
}

/// The path every call of one dataset upload hangs off.
fn upload_base(dataset_id: &str) -> String {
    format!("/datasets/{dataset_id}/upload")
}

/// Send every missing part, `parallel` at a time. The first part that cannot
/// be sent ends the run; everything stored so far stays on the server.
///
/// `base` is the upload's own path (`/datasets/:id/upload`,
/// `/models/uploads/:id`); a part goes to `<base>/files/:fileId/parts/:n`.
pub(super) async fn send_missing(
    client: &ApiClient,
    base: &str,
    plan: &Plan,
    opts: &SendOptions,
    progress: &Arc<Progress>,
) -> RmsResult<()> {
    let jobs: Vec<(usize, u32)> = plan
        .files
        .iter()
        .enumerate()
        .flat_map(|(i, f)| f.missing.iter().map(move |n| (i, *n)))
        .collect();

    stream::iter(jobs.into_iter().map(Ok::<_, RmsError>))
        .try_for_each_concurrent(opts.parallel.max(1), |(index, part)| async move {
            let job = PartJob {
                client,
                base,
                file: &plan.files[index],
                chunk_size: plan.chunk_size,
                part,
                timeout: opts.part_timeout,
            };
            send_part(job, progress, opts.retries, &opts.tuning).await?;
            progress.parts_sent.fetch_add(1, Ordering::Relaxed);
            if progress.remaining[index].fetch_sub(1, Ordering::Relaxed) == 1 {
                progress.files_done.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        })
        .await
}

// ── upload ──────────────────────────────────────────────────────────────

/// The id of the upload in flight, for the Ctrl-C message.
static ACTIVE_UPLOAD: OnceLock<String> = OnceLock::new();

/// Entry point for `datasets upload`: runs the upload and turns Ctrl-C into
/// an exit that says how to carry on.
pub async fn run_upload(client: &ApiClient, ctx: &Ctx, opts: UploadOptions) -> RmsResult<()> {
    tokio::select! {
        outcome = upload(client, ctx, &opts) => outcome.map(|_| ()),
        _ = tokio::signal::ctrl_c() => {
            if !ctx.json {
                println!();
                println!();
                match ACTIVE_UPLOAD.get() {
                    Some(id) => {
                        output::warn("Interrupted. The parts already sent are kept on the server.");
                        output::info("Resume:  run the same command again");
                        output::info(&format!("Discard: rmsctl datasets upload-cancel {id} --yes"));
                    }
                    None => output::warn("Interrupted before anything was sent."),
                }
                println!();
            }
            std::process::exit(130);
        }
    }
}

pub async fn upload(
    client: &ApiClient,
    ctx: &Ctx,
    opts: &UploadOptions,
) -> RmsResult<UploadOutcome> {
    let ui = Ui::new(ctx.json);
    let tuning = &opts.tuning;

    if let Some(c) = opts.chunk_size
        && c < MIN_CHUNK_BYTES
    {
        return Err(RmsError::InvalidInput(format!(
            "--chunk-size must be at least 5 MiB ({MIN_CHUNK_BYTES} bytes); the object store refuses smaller parts"
        )));
    }

    let root = std::fs::canonicalize(&opts.dir)
        .map_err(|e| RmsError::InvalidInput(format!("cannot open {}: {e}", opts.dir.display())))?;
    if !root.is_dir() {
        return Err(RmsError::InvalidInput(format!(
            "{} is not a directory — pass the dataset root (the directory holding meta/ and data/)",
            root.display()
        )));
    }
    let info = read_info(&root)?;
    let scan = scan_dir(&root, &opts.rules)?;
    if scan.files.is_empty() {
        return Err(RmsError::InvalidInput(format!(
            "{} holds no files to upload",
            root.display()
        )));
    }

    let store = StateStore::new(opts.state_dir.clone(), client.base_url(), &root);
    let saved = store.load();

    // Without --name: the name the upload was started with when there is one
    // to resume, else the directory's.
    let name = opts
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(String::from)
        .or_else(|| {
            saved
                .as_ref()
                .filter(|_| !opts.restart && opts.resume.is_none())
                .map(|s| s.name.clone())
        })
        .or_else(|| root.file_name().and_then(|n| n.to_str()).map(String::from))
        .ok_or_else(|| {
            RmsError::InvalidInput("pass --name: the directory has no usable name".into())
        })?;
    if name.chars().count() > 200 {
        return Err(RmsError::InvalidInput(
            "--name may be at most 200 characters".into(),
        ));
    }

    if !ui.quiet {
        output::section("◈", &format!("Upload: {}", name.bold()));
        output::field("Source", &root.display().to_string());
        output::field("Server", client.base_url());
        output::field("Format", &describe_info(&info));
        let beside = scan
            .files
            .iter()
            .filter(|f| !is_dataset_file(&f.rel))
            .count();
        output::field(
            "Files",
            &format!(
                "{} ({}){}",
                scan.files.len(),
                human_bytes(scan.total_bytes()),
                match (opts.rules.lerobot_only, beside) {
                    (true, _) => " — the LeRobot tree only (--lerobot-only)".to_string(),
                    (false, 0) => " — everything in the directory".to_string(),
                    (false, n) =>
                        format!(" — everything in the directory, {n} beside the LeRobot tree"),
                }
            ),
        );
        // Nothing is dropped silently: the dry run names every entry that
        // stays behind; a run that sends shows the first few and the count.
        if !scan.skipped.is_empty() {
            let limit = if ctx.yes { 12 } else { usize::MAX };
            output::field(
                "Left out",
                &format!(
                    "{} entr{} — NOT uploaded:",
                    scan.skipped.len(),
                    if scan.skipped.len() == 1 { "y" } else { "ies" }
                ),
            );
            for line in skipped_lines(&scan.skipped, limit) {
                println!("      {} {line}", "-".bright_black());
            }
            if ctx.yes && scan.skipped.len() > limit {
                output::info("Run without --yes to list every left-out entry.");
            }
        }
    }

    // ── Is there an upload to carry on with? ────────────────────────────
    let explicit = opts.resume.is_some();
    let candidate = opts.resume.clone().or_else(|| {
        (!opts.restart)
            .then(|| saved.as_ref().map(|s| s.dataset_id.clone()))
            .flatten()
    });
    let mut existing: Option<(String, UploadStatus)> = None;
    if let Some(id) = candidate {
        match fetch_status(client, &id, tuning).await? {
            Some((status, _)) => existing = Some((id, status)),
            None if explicit => {
                return Err(RmsError::Api {
                    status: 404,
                    message: format!(
                        "upload {id} does not exist on this server (cancelled, expired, or another organization's)"
                    ),
                });
            }
            None => {
                ui.info(&format!(
                    "The upload started earlier ({id}) no longer exists on the server — starting a new one."
                ));
                store.remove();
            }
        }
    }

    let stopped = |dataset_id: String, status: &str| UploadOutcome {
        dataset_id,
        status: status.to_string(),
        parts_sent: 0,
        bytes_sent: 0,
        executed: false,
    };

    let (dataset_id, mut plan) = match existing {
        Some((id, status)) => match status.state.as_str() {
            "uploading" => {
                let plan = build_plan(
                    &scan.files,
                    &status.files,
                    &HashSet::new(),
                    status.chunk_size,
                )?;
                ui.field("Resuming", &id);
                if opts.chunk_size.is_some_and(|c| c != status.chunk_size) {
                    ui.warn(&format!(
                        "--chunk-size is fixed when an upload starts; this one keeps {} parts (--restart to change it)",
                        human_chunk(status.chunk_size)
                    ));
                }
                ui.field(
                    "Stored",
                    &format!(
                        "{} of {} — {} part(s) still to send",
                        human_bytes(plan.total_bytes - plan.missing_bytes()),
                        human_bytes(plan.total_bytes),
                        plan.missing_parts()
                    ),
                );
                if !output::confirm(
                    &format!(
                        "This will resume upload {id} on {}: {} in {} part(s) still to send",
                        client.base_url(),
                        human_bytes(plan.missing_bytes()),
                        plan.missing_parts()
                    ),
                    ctx.yes,
                ) {
                    return Ok(stopped(id, &status.status));
                }
                store.save(&id, &name);
                (id, plan)
            }
            // Finish was already accepted: nothing left to send, only the
            // verdict to wait for. Watching is read-only, so it is not gated.
            "processing" | "completed" => {
                ui.field("Resuming", &id);
                ui.info("Every part is already stored; waiting for the server's verdict.");
                let _ = ACTIVE_UPLOAD.set(id.clone());
                return conclude(client, ctx, &ui, opts, &store, &id, 0, 0).await;
            }
            "failed" => {
                return Err(RmsError::Other(format!(
                    "upload {id} failed on the server: {}.\n  \
                     Remove it with: rmsctl datasets upload-cancel {id} --yes — then run this command again.",
                    status
                        .failure()
                        .unwrap_or_else(|| "no reason recorded".into())
                )));
            }
            other => {
                return Err(RmsError::Other(format!(
                    "upload {id} is '{other}' on the server and cannot be resumed right now"
                )));
            }
        },
        None => {
            let replaces = saved.as_ref().filter(|_| opts.restart);
            let mut action = format!(
                "This will upload {} file(s), {}, from {} to {} as dataset \"{name}\"",
                scan.files.len(),
                human_bytes(scan.total_bytes()),
                root.display(),
                client.base_url()
            );
            if let Some(old) = replaces {
                action.push_str(&format!(
                    " — and first discard the upload started earlier ({})",
                    old.dataset_id
                ));
            }
            if !output::confirm(&action, ctx.yes) {
                return Ok(stopped(String::new(), "not started"));
            }
            if let Some(old) = replaces {
                discard(client, &ui, &old.dataset_id, tuning).await?;
                store.remove();
            }
            start(client, &ui, opts, &store, &scan, &info, &name).await?
        }
    };
    let _ = ACTIVE_UPLOAD.set(dataset_id.clone());

    // ── Send, finish, and send again if the store lost something ────────
    let mut parts_sent = 0;
    let mut bytes_sent = 0;
    for round in 1..=MAX_FINISH_ROUNDS {
        let progress = Arc::new(Progress::new(&plan));
        if plan.missing_parts() > 0 {
            ui.progress(&progress.line());
            let (base, send) = (upload_base(&dataset_id), opts.send());
            let sent = tokio::select! {
                r = send_missing(client, &base, &plan, &send, &progress) => r,
                _ = ui.tick(&progress) => unreachable!("the progress ticker never returns"),
            };
            ui.progress_done(&progress.line());
            parts_sent += progress.parts_sent.load(Ordering::Relaxed);
            bytes_sent += progress.sent.load(Ordering::Relaxed);
            if let Err(e) = sent {
                // A 404/409 means the upload itself is gone or closed
                // (cancelled elsewhere); there is nothing to resume then.
                let gone = matches!(
                    e,
                    RmsError::Api {
                        status: 404 | 409,
                        ..
                    }
                );
                if !gone {
                    ui.warn(&format!(
                        "Stopped. What was sent is kept: run the same command again to resume, or discard it with\n      rmsctl datasets upload-cancel {dataset_id} --yes"
                    ));
                }
                return Err(e);
            }
            let secs = progress.started.elapsed().as_secs_f64();
            let sent_now = progress.sent.load(Ordering::Relaxed);
            ui.success(&format!(
                "Sent {} in {} ({}/s)",
                human_bytes(sent_now),
                human_duration(secs),
                human_bytes((sent_now as f64 / secs.max(0.001)) as u64)
            ));
        }

        let r = control(
            client,
            Method::POST,
            &format!("/datasets/{dataset_id}/upload/finish"),
            None,
            true,
            tuning,
        )
        .await?;
        if r.is_success() {
            break;
        }
        if r.status == 409
            && r.code().as_deref() == Some("UPLOAD_INCOMPLETE")
            && round < MAX_FINISH_ROUNDS
        {
            ui.warn("The server is missing parts it had acknowledged — sending them again.");
            let Some((status, _)) = fetch_status(client, &dataset_id, tuning).await? else {
                return Err(api_error(&r));
            };
            plan = build_plan(
                &scan.files,
                &status.files,
                &HashSet::new(),
                status.chunk_size,
            )?;
            continue;
        }
        return Err(api_error(&r));
    }

    if opts.no_wait {
        if ctx.json {
            if let Some((_, raw)) = fetch_status(client, &dataset_id, tuning).await? {
                output::json_out(&raw)?;
            }
        } else {
            output::success(&format!(
                "Upload {dataset_id} handed to the server for checking."
            ));
            output::info(&format!(
                "Follow it with: rmsctl datasets upload-status {dataset_id}"
            ));
            println!();
        }
        return Ok(UploadOutcome {
            dataset_id,
            status: "uploading".into(),
            parts_sent,
            bytes_sent,
            executed: true,
        });
    }

    conclude(
        client,
        ctx,
        &ui,
        opts,
        &store,
        &dataset_id,
        parts_sent,
        bytes_sent,
    )
    .await
}

fn describe_info(info: &Value) -> String {
    let text = |key: &str| {
        info.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string()
    };
    let mut out = format!(
        "LeRobot {} · {}",
        text("codebase_version"),
        text("robot_type")
    );
    if let Some(fps) = info.get("fps").and_then(|f| f.as_f64()) {
        out.push_str(&format!(" · {fps} fps"));
    }
    if let Some(n) = info.get("total_episodes").and_then(|f| f.as_u64()) {
        out.push_str(&format!(" · {n} episodes"));
    }
    out
}

/// Declare the files and create the dataset row.
async fn start(
    client: &ApiClient,
    ui: &Ui,
    opts: &UploadOptions,
    store: &StateStore,
    scan: &Scan,
    info: &Value,
    name: &str,
) -> RmsResult<(String, Plan)> {
    // A whole-file hash lets the server prove, after assembling, that what it
    // stored is what was on this disk. It costs one extra read of the tree.
    let hashes: Vec<Option<String>> =
        if opts.no_checksum {
            vec![None; scan.files.len()]
        } else {
            ui.info(&format!(
                "Hashing {} file(s) so the server can verify what it stored …",
                scan.files.len()
            ));
            stream::iter(scan.files.iter().map(|f| async move {
                sha256_range_async(f.abs.clone(), 0, f.size).await.map(Some)
            }))
            .buffered(opts.parallel.max(1))
            .try_collect()
            .await?
        };

    let files: Vec<Value> = scan
        .files
        .iter()
        .zip(&hashes)
        .map(|(f, sha)| match sha {
            Some(sha) => json!({ "path": f.rel, "size": f.size, "sha256": sha }),
            None => json!({ "path": f.rel, "size": f.size }),
        })
        .collect();
    let mut body = json!({ "name": name, "info": info, "files": files });
    if let Some(d) = opts.description.as_deref().filter(|d| !d.trim().is_empty()) {
        body["description"] = json!(d);
    }
    if let Some(r) = opts.robot_type.as_deref().filter(|r| !r.trim().is_empty()) {
        body["robotTypeId"] = json!(r.trim());
    }
    if let Some(c) = opts.chunk_size {
        body["chunkSize"] = json!(c);
    }

    let r = control(
        client,
        Method::POST,
        "/datasets/uploads",
        Some(&body),
        false,
        &opts.tuning,
    )
    .await?;
    if !r.is_success() {
        let mut error = api_error(&r);
        // A proxy's 5xx (no `code`) does not say whether the request reached
        // the server. It is not sent again — that could create a second
        // dataset — so say where a first one would be found.
        if r.status >= 500
            && r.code().is_none_or(|c| c.is_empty())
            && let RmsError::Api { message, .. } = &mut error
        {
            message.push_str(
                "\n  This answer came from a proxy, so the server may still have started the upload.\n  \
                 Before trying again, look for a dataset in status 'uploading' (rmsctl datasets list)\n  \
                 and discard it with: rmsctl datasets upload-cancel <id> --yes",
            );
        }
        return Err(error);
    }
    let started: StartResponse = parse(&r)?;

    // A server before RMS TASK-500 stores only the LeRobot tree and answers
    // with the declared files it will leave out. Carrying on would end in a
    // "ready" dataset that silently lacks them — the one thing this command
    // must not do — so the upload it just opened is cancelled again.
    if !started.ignored.is_empty() {
        let cancelled = control(
            client,
            Method::POST,
            &format!("/datasets/{}/upload/cancel", started.dataset_id),
            None,
            true,
            &opts.tuning,
        )
        .await
        .is_ok_and(|r| r.is_success() || r.status == 404);
        let mut paths: Vec<&str> = started.ignored.iter().map(|i| i.path.as_str()).collect();
        paths.sort();
        let shown = paths.iter().take(8).copied().collect::<Vec<_>>().join(", ");
        let more = match paths.len().saturating_sub(8) {
            0 => String::new(),
            n => format!(" and {n} more"),
        };
        return Err(RmsError::Other(format!(
            "this server stores only the LeRobot tree (meta/, data/**/*.parquet, videos/**/*.mp4, images/) \
             and would leave out {} file(s): {shown}{more}.\n  \
             It is older than the release that stores every file of a dataset directory. {}\n  \
             Either update the server, or upload only the LeRobot tree with --lerobot-only — \
             the files named above are then NOT on the instance, so keep them before deleting the directory.",
            paths.len(),
            if cancelled {
                "Nothing was uploaded; the upload that was opened has been cancelled.".to_string()
            } else {
                format!(
                    "Nothing was uploaded, but the opened upload could not be cancelled — remove it with: \
                     rmsctl datasets upload-cancel {} --yes",
                    started.dataset_id
                )
            }
        )));
    }

    // Noted before the first byte goes out, so an upload killed a second
    // later is still found again.
    let noted = store.save(&started.dataset_id, name);
    let ignored: HashSet<String> = HashSet::new();
    let plan = build_plan(&scan.files, &started.files, &ignored, started.chunk_size)?;

    ui.success(&format!(
        "Upload started: dataset {} ({} store, {} parts, {} part(s))",
        started.dataset_id,
        started.storage.as_deref().unwrap_or("unknown"),
        human_chunk(started.chunk_size),
        plan.missing_parts()
    ));
    if !noted {
        ui.warn(&format!(
            "Could not note this upload locally; to resume it pass --resume {}",
            started.dataset_id
        ));
    }
    Ok((started.dataset_id, plan))
}

/// `--restart`: remove the earlier upload before starting over.
async fn discard(client: &ApiClient, ui: &Ui, id: &str, tuning: &Tuning) -> RmsResult<()> {
    let r = control(
        client,
        Method::POST,
        &format!("/datasets/{id}/upload/cancel"),
        None,
        true,
        tuning,
    )
    .await?;
    match (r.status, r.code().as_deref()) {
        (200..=299, _) => ui.info(&format!("Discarded the earlier upload {id}.")),
        (404, _) => {}
        // Already a finished dataset: not ours to delete on a restart.
        (409, Some("UPLOAD_COMPLETED" | "UPLOAD_PROCESSING")) => ui.warn(&format!(
            "The earlier upload {id} already finished and is kept as a dataset."
        )),
        _ => return Err(api_error(&r)),
    }
    Ok(())
}

/// Wait for the verdict and print the dataset. `ready` is success; anything
/// else is an error so the exit code says so.
#[allow(clippy::too_many_arguments)]
async fn conclude(
    client: &ApiClient,
    ctx: &Ctx,
    ui: &Ui,
    opts: &UploadOptions,
    store: &StateStore,
    dataset_id: &str,
    parts_sent: usize,
    bytes_sent: u64,
) -> RmsResult<UploadOutcome> {
    let tuning = &opts.tuning;
    let began = Instant::now();
    let mut announced = String::new();
    let verdict = loop {
        let Some((status, _)) = fetch_status(client, dataset_id, tuning).await? else {
            store.remove();
            return Err(RmsError::Other(format!(
                "dataset {dataset_id} disappeared while it was being checked — was it cancelled or deleted?"
            )));
        };
        if status.status == "ready" || status.status == "failed" || status.state == "failed" {
            break status;
        }
        let phase = match (status.state.as_str(), status.status.as_str()) {
            ("processing", _) => "The server is assembling the files and verifying checksums …",
            (_, "validating") => "The server is validating the dataset …",
            _ => "Waiting for the server …",
        };
        if phase != announced {
            ui.info(phase);
            announced = phase.to_string();
        }
        if began.elapsed() >= opts.wait_timeout {
            return Err(RmsError::Other(format!(
                "the server is still checking dataset {dataset_id} after {}.\n  \
                 Nothing is lost — look again with: rmsctl datasets upload-status {dataset_id}",
                human_duration(opts.wait_timeout.as_secs_f64())
            )));
        }
        tokio::time::sleep(tuning.poll_interval).await;
    };

    // The verdict is in either way; the note has done its job.
    store.remove();

    let dataset = client.get_value(&format!("/datasets/{dataset_id}")).await;
    let ready = verdict.status == "ready";

    if ctx.json {
        match &dataset {
            Ok(v) => output::json_out(v)?,
            Err(_) => println!(
                "{}",
                json!({ "datasetId": dataset_id, "status": verdict.status })
            ),
        }
    } else {
        if ready {
            output::success("Dataset ready");
        } else {
            output::warn("The upload was stored but the dataset is not usable");
        }
        match &dataset {
            Ok(v) => {
                // The verdict stands even if the row has a shape this build
                // cannot draw.
                if let Err(e) = super::datasets::render_result(v) {
                    output::field("ID", dataset_id);
                    output::info(&format!("Dataset details unavailable: {e}"));
                }
            }
            Err(e) => output::info(&format!("Dataset details unavailable: {e}")),
        }
        if let Some(reason) = verdict.failure() {
            output::field_colored("Reason", output::truncate(&reason, 84).red());
        }
        println!();
    }

    if !ready {
        let reason = verdict
            .failure()
            .unwrap_or_else(|| "validation rejected it — see the errors above".into());
        return Err(RmsError::Other(format!(
            "dataset {dataset_id} ended 'failed': {reason}.\n  \
             Its files are kept until it is removed: rmsctl datasets upload-cancel {dataset_id} --yes"
        )));
    }
    Ok(UploadOutcome {
        dataset_id: dataset_id.to_string(),
        status: verdict.status,
        parts_sent,
        bytes_sent,
        executed: true,
    })
}

// ── upload-status ───────────────────────────────────────────────────────

pub async fn status(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let tuning = Tuning::default();
    let r = control(
        client,
        Method::GET,
        &format!("/datasets/{id}/upload"),
        None,
        true,
        &tuning,
    )
    .await?;
    if !r.is_success() {
        return Err(api_error(&r));
    }
    if ctx.json {
        return output::json_out(&r.json());
    }
    let s: UploadStatus = parse(&r)?;

    output::section("◈", &format!("Upload: {}", id.bold()));
    output::field("State", &upload_state(&s.state));
    output::field("Dataset", &upload_state(&s.status));
    if let Some(storage) = &s.storage {
        output::field("Store", storage);
    }
    output::field("Part size", &human_chunk(s.chunk_size));
    output::field(
        "Received",
        &format!(
            "{} of {}",
            human_bytes(s.received_bytes),
            human_bytes(s.total_bytes)
        ),
    );
    output::meter(
        "Progress",
        s.received_bytes as f64,
        s.total_bytes.max(1) as f64,
        "orange",
    );
    output::field(
        "Files",
        &format!("{} of {} complete", s.completed_files, s.total_files),
    );
    if s.state == "uploading" {
        let missing: usize = s
            .files
            .iter()
            .filter(|f| !f.complete)
            .map(|f| {
                let parts = f
                    .part_count
                    .unwrap_or_else(|| part_count(f.size, s.chunk_size));
                (parts as usize).saturating_sub(f.received_parts.len())
            })
            .sum();
        output::field(
            "Missing parts",
            &if s.ready_to_finish {
                "none — ready to finish".to_string()
            } else {
                missing.to_string()
            },
        );
    }
    if let Some(ts) = &s.last_activity_at {
        output::field("Last activity", ts);
    }
    if let Some(ts) = &s.expires_at {
        output::field("Expires", &format!("{ts} if no further part arrives"));
    }
    if let Some(reason) = s.failure() {
        output::field_colored("Error", output::truncate(&reason, 84).red());
    }

    // A multi-part file reads `complete: false` until finish assembles it,
    // even with every part held — what matters here is parts still to send.
    let unfinished: Vec<&RemoteFile> = s
        .files
        .iter()
        .filter(|f| {
            let parts = f
                .part_count
                .unwrap_or_else(|| part_count(f.size, s.chunk_size));
            !f.complete && f.received_parts.len() < parts as usize
        })
        .collect();
    if s.state == "uploading" && !unfinished.is_empty() {
        output::subsection(&format!("Unfinished files ({})", unfinished.len()));
        output::table_header(&[("PATH", 56), ("SIZE", 10), ("PARTS", 9)]);
        for f in unfinished.iter().take(20) {
            let parts = f
                .part_count
                .unwrap_or_else(|| part_count(f.size, s.chunk_size));
            println!(
                "    {} {} {}",
                output::pad(&output::truncate(&f.path, 56), 56),
                output::pad(&human_bytes(f.size), 10),
                format_args!("{}/{}", f.received_parts.len().min(parts as usize), parts)
            );
        }
        if unfinished.len() > 20 {
            output::info(&format!(
                "… and {} more (see --json)",
                unfinished.len() - 20
            ));
        }
    }

    println!();
    match s.state.as_str() {
        "uploading" => {
            output::info(
                "Resume:  rmsctl datasets upload <dir> --yes   (on another machine add --resume <id>)",
            );
            output::info(&format!(
                "Discard: rmsctl datasets upload-cancel {id} --yes"
            ));
        }
        "processing" => {
            output::info("Finish was accepted; the server is assembling and verifying.")
        }
        _ => {}
    }
    println!();
    Ok(())
}

pub(super) fn upload_state(state: &str) -> String {
    match state {
        "ready" => format!("{} {}", "●".bright_black(), state.green().bold()),
        "uploading" | "processing" | "validating" | "cancelling" => {
            format!(
                "{} {}",
                "◐".bright_black(),
                state.truecolor(255, 103, 0).bold()
            )
        }
        other => output::status_indicator(other),
    }
}

// ── upload-cancel ───────────────────────────────────────────────────────

pub async fn cancel(
    client: &ApiClient,
    ctx: &Ctx,
    id: &str,
    state_dir: Option<PathBuf>,
) -> RmsResult<()> {
    let tuning = Tuning::default();
    // Read it first so the gate can say what would be thrown away, and a
    // mistyped id fails before anything is asked.
    let Some((s, _)) = fetch_status(client, id, &tuning).await? else {
        return Err(RmsError::Api {
            status: 404,
            message: format!(
                "no upload {id} on this server (already cancelled, expired, or not yours)"
            ),
        });
    };
    let rejected = match s.state.as_str() {
        "processing" => {
            return Err(RmsError::InvalidInput(format!(
                "upload {id} is being assembled and verified and cannot be cancelled now — wait for the verdict (rmsctl datasets upload-status {id})"
            )));
        }
        // The upload ran to the end and validation rejected the result. The
        // server no longer calls that an upload to cancel (409
        // UPLOAD_COMPLETED) — but it is still this upload's debris, and it
        // holds its full size in the store until the dataset is deleted.
        "completed" if s.status == "failed" => true,
        "completed" => {
            return Err(RmsError::InvalidInput(format!(
                "upload {id} is complete — it is a dataset now ('{}'), not an upload to cancel",
                s.status
            )));
        }
        _ => false,
    };

    let action = if rejected {
        format!(
            "This will delete dataset {id}, which was uploaded and then failed validation, together with the {} it stored",
            human_bytes(s.total_bytes)
        )
    } else {
        format!(
            "This will cancel upload {id} and delete the {} of {} already stored, together with its dataset row",
            human_bytes(s.received_bytes),
            human_bytes(s.total_bytes)
        )
    };
    if !output::confirm(&action, ctx.yes) {
        return Ok(());
    }

    let (method, path) = if rejected {
        (Method::DELETE, format!("/datasets/{id}"))
    } else {
        (Method::POST, format!("/datasets/{id}/upload/cancel"))
    };
    let r = control(client, method, &path, None, true, &tuning).await?;
    if !r.is_success() {
        return Err(api_error(&r));
    }
    forget_dataset(state_dir, client.base_url(), id);

    if ctx.json {
        return output::json_out(&r.json());
    }
    if rejected {
        output::success(&format!(
            "Dataset {id} deleted; its stored files are removed."
        ));
    } else {
        output::success(&format!(
            "Upload {id} cancelled; its stored parts are removed."
        ));
    }
    println!();
    Ok(())
}

#[cfg(test)]
#[path = "dataset_upload_tests.rs"]
mod tests;
