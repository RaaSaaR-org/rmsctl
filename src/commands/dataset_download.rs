//! `datasets download` — take a dataset back out of RMS as the LeRobot
//! directory it was stored as.
//!
//! The counterpart of `datasets upload`: a dataset that was pushed to an
//! instance to free local disk comes back with the same relative paths
//! (`meta/`, `data/`, `videos/` …) and the same bytes, without Hugging Face.
//! The contract is the server's `docs/dataset-download.md`:
//!
//! ```text
//! GET /datasets/:id/files?limit=&after=      one page of { path, size, etag?, sha256? }
//! GET /datasets/:id/files/content?path=      the raw bytes; Range: bytes=N- resumes
//! ```
//!
//! What must stay true:
//!
//! - **Never hold a file in memory.** A response body is written to disk chunk
//!   by chunk; memory does not grow with file size or `--parallel`.
//! - **A file exists under its real name only when it is whole.** Bytes land in
//!   `<name>.rmsctl-part` and are renamed once the size — and the SHA-256, when
//!   the manifest carries one — match. An interrupted run leaves `.rmsctl-part`
//!   files, never a short file that looks finished.
//! - **The directory is the only state.** `--resume` skips files that are there
//!   with the manifest's size and continues a `.rmsctl-part` with a `Range`
//!   request from its length.
//! - **Never write outside `--out`.** The manifest is the server's; every path
//!   in it is checked before it is joined to the output directory.
//! - **Any failure is a non-zero exit.** A run that did not reproduce every
//!   file says which one and why.

use super::dataset_upload::{
    Retry, Tuning, Ui, api_error, backoff, classify, control, human_bytes, io_err, parse,
    sha256_range_async, transfer_line,
};
use crate::client::{ApiClient, RawResponse};
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::output;
use colored::Colorize;
use futures_util::{StreamExt, stream};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

/// Appended to a file's name while it is incomplete.
pub const PART_SUFFIX: &str = ".rmsctl-part";

/// Files asked for per manifest page (the server's maximum).
const MANIFEST_PAGE: u32 = 1000;

/// How often one file may be picked up again after a connection that died
/// mid-body. Each of those made progress, so they are not counted as failed
/// attempts — but a link that delivers a few bytes per connection forever is
/// not a download either.
const MAX_RESUMPTIONS: u32 = 1000;

// ── What is downloaded ──────────────────────────────────────────────────

/// A dataset, or a model that was uploaded to the instance (`models
/// download`). The server hands both out the same way — a paged file list and
/// `files/content?path=` with `Range` (docs/dataset-download.md,
/// docs/model-upload.md) — so they differ only in where the routes hang and
/// in what the thing is called.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Dataset,
    Model,
}

impl Kind {
    /// The word for it in a sentence.
    fn noun(self) -> &'static str {
        match self {
            Kind::Dataset => "dataset",
            Kind::Model => "model",
        }
    }

    /// The command group, for the hints that name a command.
    fn group(self) -> &'static str {
        match self {
            Kind::Dataset => "datasets",
            Kind::Model => "models",
        }
    }

    /// The word for it at the start of a heading.
    fn title(self) -> &'static str {
        match self {
            Kind::Dataset => "Dataset",
            Kind::Model => "Model",
        }
    }

    /// The path of the thing itself; its files are at `<base>/files`.
    fn base(self, id: &str) -> String {
        match self {
            Kind::Dataset => format!("/datasets/{id}"),
            Kind::Model => format!("/models/versions/{id}"),
        }
    }

    /// The key the detail route wraps its row in.
    fn envelope(self) -> &'static str {
        match self {
            Kind::Dataset => "dataset",
            Kind::Model => "modelVersion",
        }
    }

    /// The key of the id in the `--json` summary.
    fn id_key(self) -> &'static str {
        match self {
            Kind::Dataset => "datasetId",
            Kind::Model => "modelVersionId",
        }
    }
}

// ── Options ─────────────────────────────────────────────────────────────

pub struct DownloadOptions {
    pub kind: Kind,
    pub id: String,
    /// Where the dataset directory goes. `None`: `./<dataset name>`.
    pub out: Option<PathBuf>,
    /// Fetch only the listed files matching one of these (`--include`); empty: all.
    pub include: Vec<String>,
    pub parallel: usize,
    pub retries: u32,
    /// Continue in a directory that already holds part of the dataset.
    pub resume: bool,
    /// Write into a non-empty directory and fetch every file again.
    pub force: bool,
    pub no_checksum: bool,
    /// How long a request may deliver nothing before it is tried again.
    pub stall_timeout: Duration,
    pub tuning: Tuning,
}

/// What a download run ended with; the fields are what the tests assert on
/// and what `--json` prints.
#[derive(Debug)]
pub struct DownloadOutcome {
    /// The id of what was downloaded — a dataset's, or a model version's.
    pub dataset_id: String,
    pub out: PathBuf,
    pub files: usize,
    pub bytes: u64,
    /// Files that were already complete in the directory (`--resume`).
    pub skipped_files: usize,
    /// Bytes that came over the network in this run.
    pub fetched_bytes: u64,
    /// Files whose SHA-256 was compared with the manifest's.
    pub verified_sha256: usize,
}

// ── Server shapes ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub(super) struct RemoteEntry {
    pub(super) path: String,
    #[serde(default)]
    pub(super) size: u64,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    pub(super) sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilesPage {
    #[serde(default)]
    files: Vec<RemoteEntry>,
    #[serde(default)]
    next_cursor: Option<String>,
    #[serde(default)]
    storage: Option<String>,
}

// ── Paths ───────────────────────────────────────────────────────────────

/// The manifest path as a path under the output directory, or why it may not
/// be written. A relative path of plain segments and nothing else — the
/// server promises that, and this does not take its word for it.
pub fn safe_rel(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("empty path".into());
    }
    if path.starts_with('/') {
        return Err("absolute path".into());
    }
    if path.contains('\\') {
        return Err("backslash in path".into());
    }
    if path.chars().any(|c| c.is_control()) {
        return Err("control character in path".into());
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err("drive letter in path".into());
    }
    let mut out = PathBuf::new();
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err("empty, \".\" or \"..\" segment".into());
        }
        out.push(segment);
    }
    if path.ends_with(PART_SUFFIX) {
        return Err(format!("name ends in {PART_SUFFIX}"));
    }
    Ok(out)
}

/// `<file>.rmsctl-part` beside the file.
fn part_path(file: &Path) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(PART_SUFFIX);
    file.with_file_name(name)
}

/// A dataset name as one directory name: nothing that is a separator or that
/// a shell or a filesystem trips over.
pub fn dir_name_for(name: &str, id: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '-' || c == '.').to_string();
    if cleaned.is_empty() {
        id.to_string()
    } else {
        cleaned
    }
}

/// `bytes 100-199/200` → (100, 199, 200).
pub fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (window, total) = rest.split_once('/')?;
    let (start, end) = window.split_once('-')?;
    Some((
        start.trim().parse().ok()?,
        end.trim().parse().ok()?,
        total.trim().parse().ok()?,
    ))
}

fn unquote(etag: &str) -> &str {
    etag.trim().trim_start_matches("W/").trim_matches('"')
}

/// Whether `dir` holds anything. A directory that does not exist is empty.
fn dir_is_empty(dir: &Path) -> RmsResult<bool> {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => Ok(entries.next().is_none()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(io_err("cannot read", dir, e)),
    }
}

fn file_len(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
}

// ── Manifest ────────────────────────────────────────────────────────────

pub(super) struct Manifest {
    pub(super) files: Vec<RemoteEntry>,
    pub(super) storage: Option<String>,
}

pub(super) async fn fetch_manifest(
    client: &ApiClient,
    kind: Kind,
    id: &str,
    tuning: &Tuning,
) -> RmsResult<Manifest> {
    let base = kind.base(id);
    let mut files: Vec<RemoteEntry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut storage = None;
    let mut after: Option<String> = None;
    loop {
        let mut path = format!("{base}/files?limit={MANIFEST_PAGE}");
        if let Some(cursor) = &after {
            path.push_str("&after=");
            path.push_str(&encode_query(cursor));
        }
        let r = control(client, Method::GET, &path, None, true, tuning).await?;
        if r.status == 404 && r.code().is_none() {
            return Err(RmsError::Api {
                status: 404,
                message: format!(
                    "this server has no {} download API (GET {}/files) — \
                     it is older than the release that added it",
                    kind.noun(),
                    kind.base(":id")
                ),
            });
        }
        if !r.is_success() {
            return Err(api_error(&r));
        }
        let page: FilesPage = parse(&r)?;
        storage = page.storage.or(storage);
        for entry in page.files {
            if !seen.insert(entry.path.clone()) {
                return Err(RmsError::Parse(format!(
                    "the server listed {} twice",
                    entry.path
                )));
            }
            files.push(entry);
        }
        match page.next_cursor {
            Some(next) if !next.is_empty() => {
                // A cursor that does not move would page forever.
                if after.as_deref() == Some(next.as_str()) {
                    return Err(RmsError::Parse(
                        "the server repeated a manifest cursor".into(),
                    ));
                }
                after = Some(next);
            }
            _ => break,
        }
    }
    Ok(Manifest { files, storage })
}

// ── files ───────────────────────────────────────────────────────────────

/// `datasets files <id>` and `models files <id>`: what the instance stores,
/// one row per file (path, size, the start of the SHA-256 where listed),
/// read from every page of the manifest.
pub async fn files(client: &ApiClient, ctx: &Ctx, kind: Kind, id: &str) -> RmsResult<()> {
    let tuning = Tuning::default();
    if ctx.json {
        // The server's own body: the first page, which is all of anything up to 1000 files.
        let r = control(
            client,
            Method::GET,
            &format!("{}/files", kind.base(id)),
            None,
            true,
            &tuning,
        )
        .await?;
        if !r.is_success() {
            return Err(api_error(&r));
        }
        return output::json_out(&r.json());
    }
    let manifest = fetch_manifest(client, kind, id, &tuning).await?;
    let total: u64 = manifest.files.iter().map(|f| f.size).sum();

    output::section("◈", &format!("{} files: {}", kind.title(), id.bold()));
    if let Some(s) = &manifest.storage {
        output::field("Store", s);
    }
    output::field(
        "Total",
        &format!("{} in {} file(s)", human_bytes(total), manifest.files.len()),
    );
    println!();
    let path_w = files_path_width(
        output::term_width(),
        output::longest(manifest.files.iter().map(|f| f.path.as_str())),
    );
    output::table_header(&[("PATH", path_w), ("SIZE", 10), ("SHA-256", 14)]);
    for f in &manifest.files {
        println!(
            "    {} {} {}",
            output::pad(&output::truncate(&f.path, path_w), path_w),
            output::pad(&human_bytes(f.size), 10),
            f.sha256
                .as_deref()
                .map(|s| &s[..s.len().min(12)])
                .unwrap_or("-")
        );
    }
    println!();
    output::info(&format!(
        "Download: rmsctl {} download {id} --out <dir>",
        kind.group()
    ));
    println!();
    Ok(())
}

/// PATH of the files table: 56 without a terminal (what it always was), else
/// as wide as the longest path the terminal leaves room for, at least 24.
pub(super) fn files_path_width(term: Option<usize>, longest: usize) -> usize {
    match term {
        None => 56,
        Some(_) => output::flex_width(24, output::fixed_width(&[10, 14]), longest, term),
    }
}

/// The manifest entries `--include` keeps: those matching any pattern, by the
/// gitignore-style rules of `--exclude` (`model_upload::excluded`). No
/// pattern keeps everything.
pub(super) fn included(files: Vec<RemoteEntry>, include: &[String]) -> Vec<RemoteEntry> {
    if include.is_empty() {
        return files;
    }
    files
        .into_iter()
        .filter(|f| {
            include
                .iter()
                .any(|p| super::model_upload::excluded(p, &f.path))
        })
        .collect()
}

/// Percent-encode a query value: everything but unreserved characters.
fn encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ── Progress ────────────────────────────────────────────────────────────

struct Progress {
    total: u64,
    /// Bytes that are on disk for this dataset: skipped files, parts found, and what arrived.
    on_disk: AtomicU64,
    /// Bytes that came over the network in this run.
    fetched: AtomicU64,
    files_total: usize,
    files_done: AtomicUsize,
    verified: AtomicUsize,
    started: Instant,
}

impl Progress {
    fn line(&self) -> String {
        let done = self.on_disk.load(Ordering::Relaxed).min(self.total);
        let secs = self.started.elapsed().as_secs_f64();
        let fetched = self.fetched.load(Ordering::Relaxed);
        let rate = if secs > 0.5 {
            fetched as f64 / secs
        } else {
            0.0
        };
        transfer_line(
            "Downloading",
            done,
            self.total,
            self.files_done.load(Ordering::Relaxed),
            self.files_total,
            rate,
        )
    }
}

async fn tick(ui: &Ui, progress: &Progress) {
    // A terminal gets a smooth bar; a log gets a line every few seconds.
    let every = if ui.tty {
        Duration::from_millis(200)
    } else {
        Duration::from_secs(5)
    };
    loop {
        tokio::time::sleep(every).await;
        ui.progress(&progress.line());
    }
}

// ── One file ────────────────────────────────────────────────────────────

struct Job<'a> {
    client: &'a ApiClient,
    id: &'a str,
    entry: &'a RemoteEntry,
    dest: PathBuf,
    opts: &'a DownloadOptions,
    progress: &'a Progress,
}

struct Failure {
    retry: Retry,
    retry_after: Option<u64>,
    /// Bytes arrived before it failed: the next attempt continues, and this one is not counted.
    progressed: bool,
    error: RmsError,
}

impl Failure {
    fn fatal(message: String) -> Self {
        Failure {
            retry: Retry::Fatal,
            retry_after: None,
            progressed: false,
            error: RmsError::Other(message),
        }
    }

    fn transient(error: RmsError, progressed: bool) -> Self {
        Failure {
            retry: Retry::Transient,
            retry_after: None,
            progressed,
            error,
        }
    }
}

fn changed(path: &str, what: String) -> Failure {
    Failure::fatal(format!(
        "{path}: {what} — it changed on the server since the file list was read. Run the download again."
    ))
}

/// One request for the rest of a file, from `offset`. `Ok` means the part
/// file now holds all `size` bytes.
async fn attempt(job: &Job<'_>, part: &Path, mut offset: u64) -> Result<(), Failure> {
    let entry = job.entry;
    let size = entry.size;
    let stall = job.opts.stall_timeout;
    let mut req = job
        .client
        .request(
            Method::GET,
            &format!("{}/files/content", job.opts.kind.base(job.id)),
        )
        .query(&[("path", entry.path.as_str())]);
    if offset > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    let stalled = |progressed: bool| {
        Failure::transient(
            RmsError::Network(format!(
                "{}: the server sent nothing for {}s",
                entry.path,
                stall.as_secs().max(1)
            )),
            progressed,
        )
    };
    let mut resp = match tokio::time::timeout(stall, req.send()).await {
        Err(_) => return Err(stalled(false)),
        Ok(Err(e)) => return Err(Failure::transient(e.into(), false)),
        Ok(Ok(resp)) => resp,
    };

    let status = resp.status().as_u16();
    let header = |name: reqwest::header::HeaderName| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
    };
    match status {
        200 => {
            if let Some(len) = resp.content_length()
                && len != size
            {
                return Err(changed(
                    &entry.path,
                    format!("the file is {len} bytes, its listing said {size}"),
                ));
            }
            if offset > 0 {
                // The Range was ignored: this is the whole file, so start it over.
                tokio::fs::remove_file(part)
                    .await
                    .map_err(|e| Failure::fatal(io_err("cannot remove", part, e).to_string()))?;
                job.progress.on_disk.fetch_sub(offset, Ordering::Relaxed);
                offset = 0;
            }
        }
        206 => {
            let range = header(reqwest::header::CONTENT_RANGE);
            match range.as_deref().and_then(parse_content_range) {
                Some((start, _, total)) if start == offset && total == size => {}
                Some((_, _, total)) if total != size => {
                    return Err(changed(
                        &entry.path,
                        format!("the file is {total} bytes, its listing said {size}"),
                    ));
                }
                _ => {
                    return Err(Failure::fatal(format!(
                        "{}: asked for bytes from {offset}, the server answered Content-Range {:?}",
                        entry.path, range
                    )));
                }
            }
        }
        _ => {
            let retry_after =
                header(reqwest::header::RETRY_AFTER).and_then(|v| v.trim().parse::<u64>().ok());
            let body = resp.text().await.unwrap_or_default();
            let raw = RawResponse {
                status,
                retry_after,
                body,
            };
            if status == 416 {
                // What is on disk is not a prefix of this file any more: start it over.
                let _ = tokio::fs::remove_file(part).await;
                job.progress.on_disk.fetch_sub(offset, Ordering::Relaxed);
                return Err(Failure::transient(api_error(&raw), false));
            }
            return Err(Failure {
                retry: classify(status, raw.code().as_deref()),
                retry_after,
                progressed: false,
                error: api_error(&raw),
            });
        }
    }
    if let (Some(listed), Some(served)) = (entry.etag.as_deref(), header(reqwest::header::ETAG))
        && unquote(listed) != unquote(&served)
    {
        return Err(changed(&entry.path, "its ETag differs".into()));
    }

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(part)
        .await
        .map_err(|e| Failure::fatal(io_err("cannot write", part, e).to_string()))?;
    let mut received: u64 = 0;
    let outcome = loop {
        let chunk = match tokio::time::timeout(stall, resp.chunk()).await {
            Err(_) => break Err(stalled(received > 0)),
            Ok(Err(e)) => break Err(Failure::transient(e.into(), received > 0)),
            Ok(Ok(None)) => break Ok(()),
            Ok(Ok(Some(chunk))) => chunk,
        };
        let len = chunk.len() as u64;
        if offset + received + len > size {
            break Err(changed(
                &entry.path,
                format!("the server sent more than the {size} bytes it listed"),
            ));
        }
        if let Err(e) = file.write_all(&chunk).await {
            break Err(Failure::fatal(io_err("cannot write", part, e).to_string()));
        }
        received += len;
        job.progress.on_disk.fetch_add(len, Ordering::Relaxed);
        job.progress.fetched.fetch_add(len, Ordering::Relaxed);
    };
    if let Err(e) = file.flush().await {
        return Err(Failure::fatal(io_err("cannot write", part, e).to_string()));
    }
    drop(file);
    outcome?;
    if offset + received < size {
        return Err(Failure::transient(
            RmsError::Network(format!(
                "{}: the connection ended {} short",
                entry.path,
                human_bytes(size - offset - received)
            )),
            received > 0,
        ));
    }
    Ok(())
}

/// Bring one file to its final path: fetch what is missing into the part
/// file, verify it, rename it.
async fn fetch_file(job: &Job<'_>) -> RmsResult<()> {
    let entry = job.entry;
    let size = entry.size;
    let dest = &job.dest;
    let part = part_path(dest);
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| io_err("cannot create", parent, e))?;
    }

    let mut failures: u32 = 0;
    let mut resumptions: u32 = 0;
    loop {
        let mut offset = file_len(&part).unwrap_or(0);
        if offset > size {
            // Longer than the file: not a prefix of it.
            tokio::fs::remove_file(&part)
                .await
                .map_err(|e| io_err("cannot remove", &part, e))?;
            job.progress.on_disk.fetch_sub(offset, Ordering::Relaxed);
            offset = 0;
        }

        if offset < size {
            let Err(failure) = attempt(job, &part, offset).await else {
                continue;
            };
            if failure.retry == Retry::Fatal {
                return Err(failure.error);
            }
            if failure.progressed {
                resumptions += 1;
            } else {
                failures += 1;
            }
            if failures > job.opts.retries || resumptions > MAX_RESUMPTIONS {
                return Err(RmsError::Other(format!(
                    "{}: gave up after {} failed attempt(s): {}",
                    entry.path,
                    failures.max(1),
                    failure.error
                )));
            }
            tokio::time::sleep(backoff(
                &job.opts.tuning,
                failures.max(1),
                failure.retry_after,
            ))
            .await;
            continue;
        }

        // Every byte is there (an empty file has none to fetch).
        if size == 0 {
            tokio::fs::File::create(&part)
                .await
                .map_err(|e| io_err("cannot write", &part, e))?;
        }
        if let Some(expected) = entry.sha256.as_deref()
            && !job.opts.no_checksum
        {
            let actual = sha256_range_async(part.clone(), 0, size).await?;
            if !actual.eq_ignore_ascii_case(expected) {
                // Nothing of it can be trusted: fetch the file again from its first byte.
                tokio::fs::remove_file(&part)
                    .await
                    .map_err(|e| io_err("cannot remove", &part, e))?;
                job.progress.on_disk.fetch_sub(size, Ordering::Relaxed);
                failures += 1;
                if failures > job.opts.retries {
                    return Err(RmsError::Other(format!(
                        "{}: SHA-256 mismatch after {failures} attempt(s) — expected {expected}, got {actual}",
                        entry.path
                    )));
                }
                tokio::time::sleep(backoff(&job.opts.tuning, failures, None)).await;
                continue;
            }
            job.progress.verified.fetch_add(1, Ordering::Relaxed);
        }
        tokio::fs::rename(&part, dest)
            .await
            .map_err(|e| io_err("cannot move into place", dest, e))?;
        job.progress.files_done.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }
}

// ── download ────────────────────────────────────────────────────────────

/// The directory of the download in flight, for the Ctrl-C message.
static ACTIVE_DOWNLOAD: OnceLock<(Kind, String, PathBuf)> = OnceLock::new();

/// Entry point for `datasets download`: runs it and turns Ctrl-C into an exit
/// that says how to carry on.
pub async fn run_download(client: &ApiClient, ctx: &Ctx, opts: DownloadOptions) -> RmsResult<()> {
    tokio::select! {
        outcome = download(client, ctx, &opts) => outcome.map(|_| ()),
        _ = tokio::signal::ctrl_c() => {
            if !ctx.json {
                println!();
                println!();
                match ACTIVE_DOWNLOAD.get() {
                    Some((kind, id, out)) => {
                        output::warn("Interrupted. What arrived is kept; unfinished files end in .rmsctl-part.");
                        output::info(&format!(
                            "Resume: rmsctl {} download {id} --out {} --resume",
                            kind.group(),
                            out.display()
                        ));
                    }
                    None => output::warn("Interrupted before anything was written."),
                }
                println!();
            }
            std::process::exit(130);
        }
    }
}

pub async fn download(
    client: &ApiClient,
    ctx: &Ctx,
    opts: &DownloadOptions,
) -> RmsResult<DownloadOutcome> {
    let ui = Ui::new(ctx.json);
    let tuning = &opts.tuning;
    let id = opts.id.as_str();
    let kind = opts.kind;
    let noun = kind.noun();

    // The dataset (or model) itself: its name is the default directory, and a
    // wrong id should read "not found", not "no files".
    let r = control(client, Method::GET, &kind.base(id), None, true, tuning).await?;
    if !r.is_success() {
        return Err(api_error(&r));
    }
    // `{ dataset: {…} }` / `{ modelVersion: {…} }`; a bare object is read too.
    let body = r.json();
    let dataset = body.get(kind.envelope()).unwrap_or(&body);
    let text = |key: &str| dataset.get(key).and_then(Value::as_str);
    let name = match (kind, text("name"), text("version")) {
        // A model is a name and a version; two versions must not share a directory.
        (Kind::Model, Some(name), Some(version)) => format!("{name} {version}"),
        (Kind::Model, None, Some(version)) => version.to_string(),
        (_, Some(name), _) => name.to_string(),
        _ => id.to_string(),
    };
    let status = text(match kind {
        Kind::Dataset => "status",
        Kind::Model => "deploymentStatus",
    })
    .unwrap_or("-")
    .to_string();

    let out = opts
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(dir_name_for(&name, id)));
    if out.exists() && !out.is_dir() {
        return Err(RmsError::InvalidInput(format!(
            "{} exists and is not a directory",
            out.display()
        )));
    }
    let fresh = dir_is_empty(&out)?;
    if !fresh && !opts.resume && !opts.force {
        return Err(RmsError::InvalidInput(format!(
            "{} is not empty.\n  Continue an earlier download of this {noun} with --resume \
             (keeps complete files, continues partial ones),\n  fetch everything again over it \
             with --force, or pick another directory with --out.",
            out.display()
        )));
    }

    let mut manifest = fetch_manifest(client, kind, id, tuning).await?;
    if manifest.files.is_empty() {
        return Err(RmsError::Other(format!(
            "the server lists no files for {noun} {id}"
        )));
    }
    // `--include`: from here on the manifest is the files asked for, so the
    // fetch, the checks and the final "every listed file is on disk" cover
    // exactly those.
    let listed = manifest.files.len();
    manifest.files = included(manifest.files, &opts.include);
    if manifest.files.is_empty() {
        return Err(RmsError::InvalidInput(format!(
            "no file of the {listed} the server lists for {noun} {id} matches --include {} — \
             see 'rmsctl {} files {id}'",
            opts.include.join(" --include "),
            kind.group()
        )));
    }
    // Check every path before the first byte is written.
    let mut dests: Vec<PathBuf> = Vec::with_capacity(manifest.files.len());
    for entry in &manifest.files {
        let rel = safe_rel(&entry.path).map_err(|why| {
            RmsError::Other(format!(
                "the server listed a path this command will not write ({why}): {:?}",
                entry.path
            ))
        })?;
        dests.push(out.join(rel));
    }
    let total: u64 = manifest.files.iter().map(|f| f.size).sum();

    // What is already there.
    let mut todo: Vec<usize> = Vec::new();
    let mut skipped_files = 0usize;
    let mut on_disk = 0u64;
    for (i, entry) in manifest.files.iter().enumerate() {
        let dest = &dests[i];
        let part = part_path(dest);
        if opts.force {
            if part.exists() {
                std::fs::remove_file(&part).map_err(|e| io_err("cannot remove", &part, e))?;
            }
        } else if opts.resume {
            if file_len(dest) == Some(entry.size) {
                skipped_files += 1;
                on_disk += entry.size;
                continue;
            }
            on_disk += file_len(&part).unwrap_or(0);
        }
        todo.push(i);
    }
    let with_sha = manifest.files.iter().filter(|f| f.sha256.is_some()).count();

    if !ui.quiet {
        println!();
        output::section("◈", &format!("Download: {name}"));
        println!();
    }
    ui.field(
        match kind {
            Kind::Dataset => "Dataset",
            Kind::Model => "Model version",
        },
        id,
    );
    ui.field("Status", &status);
    ui.field("Into", &out.display().to_string());
    ui.field(
        "Files",
        &format!(
            "{} · {}{}",
            manifest.files.len(),
            human_bytes(total),
            manifest
                .storage
                .as_deref()
                .map(|s| format!(" · from the {s} store"))
                .unwrap_or_default()
        ),
    );
    if !opts.include.is_empty() {
        ui.field(
            "Included",
            &format!(
                "{} of {listed} files, by --include {}",
                manifest.files.len(),
                opts.include.join(" --include ")
            ),
        );
    }
    ui.field(
        "Verified by",
        &if opts.no_checksum || with_sha == 0 {
            "size".to_string()
        } else if with_sha == manifest.files.len() {
            "size and SHA-256".to_string()
        } else {
            format!(
                "size; SHA-256 for {with_sha} of {} files",
                manifest.files.len()
            )
        },
    );
    if opts.resume && !fresh {
        ui.field(
            "Already there",
            &format!(
                "{skipped_files} complete file(s) · {} of {}",
                human_bytes(on_disk.min(total)),
                human_bytes(total)
            ),
        );
    }
    // A model's status is its place on the deployment ladder, not its readiness.
    if kind == Kind::Dataset && status != "ready" {
        ui.warn(&format!(
            "The dataset is {status}, not ready — downloading what is stored."
        ));
    }
    if !ui.quiet {
        println!();
    }

    std::fs::create_dir_all(&out).map_err(|e| io_err("cannot create", &out, e))?;
    let _ = ACTIVE_DOWNLOAD.set((kind, id.to_string(), out.clone()));

    let progress = Progress {
        total,
        on_disk: AtomicU64::new(on_disk),
        fetched: AtomicU64::new(0),
        files_total: manifest.files.len(),
        files_done: AtomicUsize::new(skipped_files),
        verified: AtomicUsize::new(0),
        started: Instant::now(),
    };

    let work = async {
        let mut jobs = stream::iter(todo.iter().map(|&i| {
            let job = Job {
                client,
                id,
                entry: &manifest.files[i],
                dest: dests[i].clone(),
                opts,
                progress: &progress,
            };
            async move { fetch_file(&job).await }
        }))
        .buffer_unordered(opts.parallel.max(1));
        // The first file that cannot be fetched ends the run; what the others
        // have written stays for --resume.
        while let Some(result) = jobs.next().await {
            result?;
        }
        Ok::<(), RmsError>(())
    };
    let result = tokio::select! {
        r = work => r,
        _ = tick(&ui, &progress) => unreachable!("the progress ticker never returns"),
    };
    ui.progress_done(&progress.line());
    if !ui.quiet {
        println!();
    }

    let fetched_bytes = progress.fetched.load(Ordering::Relaxed);
    if let Err(e) = result {
        ui.warn(&format!(
            "Download incomplete: {} of {} files are in {}.",
            progress.files_done.load(Ordering::Relaxed),
            manifest.files.len(),
            out.display()
        ));
        ui.info(&format!(
            "Continue: rmsctl {} download {id} --out {} --resume",
            kind.group(),
            out.display()
        ));
        if !ui.quiet {
            println!();
        }
        return Err(e);
    }

    // The last word: every listed file is there under its name with its size.
    for (i, entry) in manifest.files.iter().enumerate() {
        match file_len(&dests[i]) {
            Some(len) if len == entry.size => {}
            got => {
                return Err(RmsError::Other(format!(
                    "{}: {} on disk, the server listed {} bytes",
                    entry.path,
                    got.map(|n| format!("{n} bytes"))
                        .unwrap_or_else(|| "missing".into()),
                    entry.size
                )));
            }
        }
    }

    let outcome = DownloadOutcome {
        dataset_id: id.to_string(),
        out: out.clone(),
        files: manifest.files.len(),
        bytes: total,
        skipped_files,
        fetched_bytes,
        verified_sha256: progress.verified.load(Ordering::Relaxed),
    };
    if ctx.json {
        output::json_out(&json!({
            kind.id_key(): outcome.dataset_id,
            "name": name,
            "out": outcome.out.display().to_string(),
            "files": outcome.files,
            "bytes": outcome.bytes,
            "skippedFiles": outcome.skipped_files,
            "fetchedBytes": outcome.fetched_bytes,
            "verifiedSha256": outcome.verified_sha256,
        }))?;
    } else {
        output::success(&format!(
            "Downloaded {} file(s), {} into {}",
            outcome.files,
            human_bytes(total),
            out.display()
        ));
        output::field(
            "Fetched",
            &format!(
                "{} in {}{}",
                human_bytes(fetched_bytes),
                super::dataset_upload::human_duration(progress.started.elapsed().as_secs_f64()),
                if skipped_files > 0 {
                    format!(" · {skipped_files} file(s) were already complete")
                } else {
                    String::new()
                }
            ),
        );
        output::field(
            "Verified",
            &if outcome.verified_sha256 > 0 {
                format!(
                    "size of every file · SHA-256 of {}",
                    outcome.verified_sha256
                )
            } else {
                "size of every file".to_string()
            },
        );
        println!();
    }
    Ok(outcome)
}

#[cfg(test)]
#[path = "dataset_download_tests.rs"]
pub(super) mod tests;
