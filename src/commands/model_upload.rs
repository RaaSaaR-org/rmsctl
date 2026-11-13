//! `rmsctl models upload | upload-status | upload-cancel | files`
//!
//! Sends a trained checkpoint directory straight into an RMS instance's model
//! store and registers it — no Hugging Face repository, no external URL. The
//! way back, `models download`, is the dataset download's engine
//! (`dataset_download.rs`, `Kind::Model`): the server hands both out the same way.
//! The contract is the server's `docs/model-upload.md`:
//!
//! ```text
//! POST /models/uploads                              declare the model and its files
//! PUT  /models/uploads/:id/files/:fileId/parts/:n   one part, raw body
//! GET  /models/uploads/:id                          what the server holds
//! POST /models/uploads/:id/finish                   every part is sent
//! POST /models/uploads/:id/cancel                   stop and remove everything
//! GET  /models/uploads                              uploads that are not models yet
//! GET  /models/versions/:id/files                   the stored files of a model
//! GET  /models/versions/:id/files/content?path=     one of them, with Range
//! ```
//!
//! The part protocol is the dataset upload's, so sending parts, retrying,
//! planning what is missing and the note that makes "run it again" resume all
//! come from `dataset_upload.rs`. What is here is what differs: every file of
//! the directory is a candidate (a checkpoint has no fixed layout), the start
//! request carries what the registry will say about the model, and the verdict
//! is a registered model version rather than a validated dataset.

use super::dataset_download::{Kind, fetch_manifest};
use super::dataset_upload::{
    LocalFile, MIN_CHUNK_BYTES, Plan, Progress, RemoteFile, SendOptions, StateStore, Tuning, Ui,
    api_error, build_plan, control, forget_dataset, human_bytes, human_chunk, human_duration,
    io_err, parse, part_count, send_missing, sha256_range_async, upload_state,
};
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::output;
use colored::*;
use futures_util::{StreamExt, TryStreamExt, stream};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Rounds of "send what is missing, then finish"; a second round only happens
/// when finish answers `UPLOAD_INCOMPLETE`.
const MAX_FINISH_ROUNDS: usize = 3;

// ── Options ─────────────────────────────────────────────────────────────

pub struct ModelUploadOptions {
    pub dir: PathBuf,
    pub name: Option<String>,
    pub version: String,
    pub model_type: Option<String>,
    pub base_model: Option<String>,
    pub dataset: Option<String>,
    /// `k=v` pairs or paths of JSON files; become `trainingMetrics`.
    pub metrics: Vec<String>,
    /// The same for `validationMetrics`.
    pub eval_metrics: Vec<String>,
    pub notes: Option<String>,
    pub exclude: Vec<String>,
    pub chunk_size: Option<u64>,
    pub parallel: usize,
    pub retries: u32,
    pub resume: Option<String>,
    pub restart: bool,
    pub no_checksum: bool,
    pub no_wait: bool,
    pub wait_timeout: Duration,
    pub part_timeout: Duration,
    pub state_dir: Option<PathBuf>,
    pub tuning: Tuning,
}

impl ModelUploadOptions {
    fn send(&self) -> SendOptions {
        SendOptions {
            parallel: self.parallel,
            retries: self.retries,
            part_timeout: self.part_timeout,
            tuning: self.tuning,
        }
    }
}

/// What an upload run ended with; the fields are what the tests assert on.
#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct ModelUploadOutcome {
    pub upload_id: String,
    /// The upload's state when the command stopped watching.
    pub state: String,
    pub model_version_id: Option<String>,
    pub parts_sent: usize,
    pub bytes_sent: u64,
    /// False when the command described the action and stopped at the gate.
    pub executed: bool,
}

// ── Which files of the directory go ─────────────────────────────────────

/// Match one path segment against a pattern segment: `*` is any run of
/// characters, `?` exactly one.
fn segment_match(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => (0..=text.len()).any(|i| segment_match(rest, &text[i..])),
        Some((b'?', rest)) => !text.is_empty() && segment_match(rest, &text[1..]),
        Some((c, rest)) => text.first() == Some(c) && segment_match(rest, &text[1..]),
    }
}

fn segments_match(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|i| segments_match(rest, &path[i..])),
        Some((p, rest)) => path.split_first().is_some_and(|(s, tail)| {
            segment_match(p.as_bytes(), s.as_bytes()) && segments_match(rest, tail)
        }),
    }
}

/// Whether `--exclude <pattern>` leaves `rel` (a path relative to the model
/// directory, forward slashes) out.
///
/// The rules are the ones people expect from `.gitignore`: a pattern without
/// a slash matches a file or directory of that name at any depth
/// (`*.log`, `wandb`); one with a slash is anchored at the directory
/// (`checkpoint-*/optimizer.pt`); `**` crosses directories; and a pattern
/// that matches a directory leaves out everything under it.
pub fn excluded(pattern: &str, rel: &str) -> bool {
    let pattern = pattern
        .trim()
        .trim_start_matches("./")
        .trim_end_matches('/');
    if pattern.is_empty() {
        return false;
    }
    let path: Vec<&str> = rel.split('/').collect();
    if !pattern.contains('/') {
        return path
            .iter()
            .any(|s| segment_match(pattern.as_bytes(), s.as_bytes()));
    }
    let pat: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
    // The path itself, or any directory above it.
    (1..=path.len()).any(|n| segments_match(&pat, &path[..n]))
}

#[derive(Debug, Default)]
struct Scan {
    files: Vec<LocalFile>,
    /// Files an `--exclude` pattern left out.
    excluded: usize,
}

impl Scan {
    fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

fn scan_dir(root: &Path, exclude: &[String]) -> RmsResult<Scan> {
    let mut scan = Scan::default();
    walk(root, "", 0, exclude, &mut scan)?;
    scan.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(scan)
}

fn walk(dir: &Path, rel: &str, depth: usize, exclude: &[String], scan: &mut Scan) -> RmsResult<()> {
    // The cap only exists to end a symlink loop.
    if depth > 32 {
        return Err(RmsError::InvalidInput(format!(
            "{} is nested more than 32 levels deep — is there a symlink loop?",
            dir.display()
        )));
    }
    for entry in std::fs::read_dir(dir).map_err(|e| io_err("cannot read", dir, e))? {
        let entry = entry.map_err(|e| io_err("cannot read", dir, e))?;
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
        // tree of links into `blobs/`.
        let meta = std::fs::metadata(&path).map_err(|e| io_err("cannot stat", &path, e))?;
        if meta.is_dir() {
            walk(&path, &child, depth + 1, exclude, scan)?;
        } else if meta.is_file() {
            if exclude.iter().any(|p| excluded(p, &child)) {
                scan.excluded += 1;
            } else {
                scan.files.push(LocalFile {
                    rel: child,
                    abs: path,
                    size: meta.len(),
                });
            }
        }
    }
    Ok(())
}

// ── Metrics ─────────────────────────────────────────────────────────────

/// Read `--metrics` / `--eval-metrics` values into one JSON object.
///
/// Each value is either `key=value` — the value read as JSON when it is a
/// number, `true`, `false` or `null`, as text otherwise — or the path of a
/// file holding a JSON object. Later values win over earlier ones.
pub fn parse_metrics(flag: &str, values: &[String]) -> RmsResult<Map<String, Value>> {
    let mut out = Map::new();
    for raw in values {
        let as_file = Path::new(raw);
        if as_file.is_file() {
            let text =
                std::fs::read_to_string(as_file).map_err(|e| io_err("cannot read", as_file, e))?;
            let parsed: Value = serde_json::from_str(&text).map_err(|e| {
                RmsError::InvalidInput(format!("{flag} {raw}: not valid JSON ({e})"))
            })?;
            let Value::Object(map) = parsed else {
                return Err(RmsError::InvalidInput(format!(
                    "{flag} {raw}: the file must hold a JSON object, e.g. {{\"final_loss\": 0.026}}"
                )));
            };
            out.extend(map);
            continue;
        }
        let Some((key, value)) = raw.split_once('=') else {
            return Err(RmsError::InvalidInput(format!(
                "{flag} {raw}: expected key=value or the path of a JSON file"
            )));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(RmsError::InvalidInput(format!(
                "{flag} {raw}: the key is empty"
            )));
        }
        let value = value.trim();
        let parsed = match serde_json::from_str::<Value>(value) {
            Ok(v @ (Value::Number(_) | Value::Bool(_) | Value::Null)) => v,
            _ => Value::String(value.to_string()),
        };
        out.insert(key.to_string(), parsed);
    }
    Ok(out)
}

// ── Server shapes ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartResponse {
    upload_id: String,
    chunk_size: u64,
    #[serde(default)]
    storage: Option<String>,
    #[serde(default)]
    files: Vec<RemoteFile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadStatus {
    #[serde(default)]
    upload_id: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    base_model: Option<String>,
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
    model_version_id: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    last_activity_at: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

impl UploadStatus {
    fn label(&self) -> String {
        match (&self.name, &self.version) {
            (Some(n), Some(v)) => format!("{n} · {v}"),
            (Some(n), None) => n.clone(),
            _ => self.upload_id.clone(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct UploadsResponse {
    #[serde(default)]
    uploads: Vec<UploadStatus>,
}

fn base(id: &str) -> String {
    format!("/models/uploads/{id}")
}

/// The upload the server holds. `None` when it is gone (cancelled, expired,
/// or another organization's).
async fn fetch_status(
    client: &ApiClient,
    id: &str,
    tuning: &Tuning,
) -> RmsResult<Option<(UploadStatus, Value)>> {
    let r = control(client, Method::GET, &base(id), None, true, tuning).await?;
    if r.status == 404 {
        return Ok(None);
    }
    if !r.is_success() {
        return Err(api_error(&r));
    }
    Ok(Some((parse(&r)?, r.json())))
}

/// The note key: the same directory may be uploaded as a dataset and as a
/// model to one server, and the two must not find each other's upload.
fn note_server(client: &ApiClient) -> String {
    format!("{}#models", client.base_url())
}

// ── upload ──────────────────────────────────────────────────────────────

/// The id of the upload in flight, for the Ctrl-C message.
static ACTIVE_UPLOAD: OnceLock<String> = OnceLock::new();

/// Entry point for `models upload`: runs the upload and turns Ctrl-C into an
/// exit that says how to carry on.
pub async fn run_upload(client: &ApiClient, ctx: &Ctx, opts: ModelUploadOptions) -> RmsResult<()> {
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
                        output::info(&format!("Discard: rmsctl models upload-cancel {id} --yes"));
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
    opts: &ModelUploadOptions,
) -> RmsResult<ModelUploadOutcome> {
    let ui = Ui::new(ctx.json);
    let tuning = &opts.tuning;

    if let Some(c) = opts.chunk_size
        && c < MIN_CHUNK_BYTES
    {
        return Err(RmsError::InvalidInput(format!(
            "--chunk-size must be at least 5 MiB ({MIN_CHUNK_BYTES} bytes); the object store refuses smaller parts"
        )));
    }
    let version = opts.version.trim().to_string();
    if version.is_empty() || version.chars().count() > 100 {
        return Err(RmsError::InvalidInput(
            "--version is required (1 to 100 characters), e.g. 1.0.0 or step-5000".into(),
        ));
    }
    let training_metrics = parse_metrics("--metrics", &opts.metrics)?;
    let validation_metrics = parse_metrics("--eval-metrics", &opts.eval_metrics)?;

    let root = std::fs::canonicalize(&opts.dir)
        .map_err(|e| RmsError::InvalidInput(format!("cannot open {}: {e}", opts.dir.display())))?;
    if !root.is_dir() {
        return Err(RmsError::InvalidInput(format!(
            "{} is not a directory — pass the checkpoint directory (the one holding config.json and the weights)",
            root.display()
        )));
    }
    let scan = scan_dir(&root, &opts.exclude)?;
    if scan.files.is_empty() {
        return Err(RmsError::InvalidInput(format!(
            "{} holds no files to upload{}",
            root.display(),
            if scan.excluded > 0 {
                " — every file matched an --exclude pattern"
            } else {
                ""
            }
        )));
    }

    let store = StateStore::new(opts.state_dir.clone(), &note_server(client), &root);
    let saved = store.load();

    let name = opts
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(String::from)
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
        output::section("◈", &format!("Upload model: {} {}", name.bold(), version));
        output::field("Source", &root.display().to_string());
        output::field("Server", client.base_url());
        if let Some(b) = &opts.base_model {
            output::field("Base model", b);
        }
        if let Some(d) = &opts.dataset {
            output::field("Trained on", d);
        }
        let left_out = match scan.excluded {
            0 => String::new(),
            n => format!(" — {n} excluded"),
        };
        output::field(
            "Files",
            &format!(
                "{} ({}){left_out}",
                scan.files.len(),
                human_bytes(scan.total_bytes())
            ),
        );
        // The few files that make up nearly all of it: what the user came to send.
        let mut largest: Vec<&LocalFile> = scan.files.iter().collect();
        largest.sort_by_key(|f| std::cmp::Reverse(f.size));
        for f in largest.iter().take(3).filter(|f| f.size >= 1024 * 1024) {
            output::field("", &format!("{}  {}", human_bytes(f.size), f.rel));
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

    // The upload remembered for this directory is past sending and is another
    // model than the one asked for now (a run with --no-wait, or one stopped
    // while the server was still verifying, leaves its note behind). Reporting
    // it would say "registered" about the wrong version and send nothing.
    let other_model = existing.as_ref().is_some_and(|(_, status)| {
        matches!(status.state.as_str(), "processing" | "completed")
            && (status.name.as_deref() != Some(name.as_str())
                || status.version.as_deref() != Some(version.as_str()))
    });
    if other_model
        && !explicit
        && let Some((id, status)) = existing.take()
    {
        ui.info(&format!(
            "The upload remembered for this directory ({id}) is \"{}\" and needs nothing more sent — starting a new one for \"{name}\" {version}.",
            status.label()
        ));
        store.remove();
    }

    let stopped = |upload_id: String, state: &str| ModelUploadOutcome {
        upload_id,
        state: state.to_string(),
        model_version_id: None,
        parts_sent: 0,
        bytes_sent: 0,
        executed: false,
    };

    let (upload_id, mut plan) = match existing {
        Some((id, status)) => match status.state.as_str() {
            "uploading" => {
                let plan = build_plan(
                    &scan.files,
                    &status.files,
                    &HashSet::new(),
                    status.chunk_size,
                )?;
                let renamed = status.name.as_deref() != Some(name.as_str())
                    || status.version.as_deref() != Some(version.as_str());
                // Carrying on would register the directory under the name and
                // version of the earlier run, not the ones asked for now — and
                // with --yes nobody reads a warning. Only an upload named with
                // --resume is continued whatever it is called.
                if renamed && !explicit {
                    return Err(RmsError::InvalidInput(format!(
                        "an upload of this directory is already open as \"{}\" ({id}), and this run asks for \"{name} · {version}\".\n  \
                         Name, version and the other details are fixed when an upload starts. Either\n  \
                         - run again with that name and version to resume it,\n  \
                         - add --restart to discard it and upload as \"{name} · {version}\", or\n  \
                         - add --resume {id} to finish it as it was started.",
                        status.label()
                    )));
                }
                ui.field("Resuming", &format!("{id} ({})", status.label()));
                ui.info(
                    "Base model, dataset, metrics and notes stay as they were given when this upload started (--restart to change them).",
                );
                if renamed {
                    ui.warn(&format!(
                        "Name, version and the other details are fixed when an upload starts; this one stays \"{}\" (--restart to change them)",
                        status.label()
                    ));
                }
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
                        "This will resume model upload {id} on {}: {} in {} part(s) still to send",
                        client.base_url(),
                        human_bytes(plan.missing_bytes()),
                        plan.missing_parts()
                    ),
                    ctx.yes,
                ) {
                    return Ok(stopped(id, &status.state));
                }
                store.save(&id, &name);
                (id, plan)
            }
            // Finish was already accepted: nothing left to send, only the
            // registration to wait for. Watching is read-only, so not gated.
            "processing" | "completed" => {
                ui.field("Resuming", &id);
                ui.info("Every part is already stored; waiting for the model to be registered.");
                let _ = ACTIVE_UPLOAD.set(id.clone());
                return conclude(client, ctx, &ui, opts, &store, &id, 0, 0).await;
            }
            "failed" => {
                return Err(RmsError::Other(format!(
                    "upload {id} failed on the server: {}.\n  \
                     Remove it with: rmsctl models upload-cancel {id} --yes — then run this command again.",
                    status.error.unwrap_or_else(|| "no reason recorded".into())
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
                "This will upload {} file(s), {}, from {} to {} and register them as model \"{name}\" version \"{version}\"",
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
            let metadata = Metadata {
                name: &name,
                version: &version,
                training_metrics,
                validation_metrics,
            };
            start(client, &ui, opts, &store, &scan, metadata).await?
        }
    };
    let _ = ACTIVE_UPLOAD.set(upload_id.clone());

    // ── Send, finish, and send again if the store lost something ────────
    let upload_base = base(&upload_id);
    let send = opts.send();
    let mut parts_sent = 0;
    let mut bytes_sent = 0;
    for round in 1..=MAX_FINISH_ROUNDS {
        let progress = Arc::new(Progress::new(&plan));
        if plan.missing_parts() > 0 {
            ui.progress(&progress.line());
            let sent = tokio::select! {
                r = send_missing(client, &upload_base, &plan, &send, &progress) => r,
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
                        "Stopped. What was sent is kept: run the same command again to resume, or discard it with\n      rmsctl models upload-cancel {upload_id} --yes"
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
            &format!("{upload_base}/finish"),
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
            let Some((status, _)) = fetch_status(client, &upload_id, tuning).await? else {
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
            if let Some((_, raw)) = fetch_status(client, &upload_id, tuning).await? {
                output::json_out(&raw)?;
            }
        } else {
            output::success(&format!(
                "Upload {upload_id} handed to the server to assemble, verify and register."
            ));
            output::info(&format!(
                "Follow it with: rmsctl models upload-status {upload_id}"
            ));
            println!();
        }
        return Ok(ModelUploadOutcome {
            upload_id,
            state: "processing".into(),
            model_version_id: None,
            parts_sent,
            bytes_sent,
            executed: true,
        });
    }

    conclude(
        client, ctx, &ui, opts, &store, &upload_id, parts_sent, bytes_sent,
    )
    .await
}

struct Metadata<'a> {
    name: &'a str,
    version: &'a str,
    training_metrics: Map<String, Value>,
    validation_metrics: Map<String, Value>,
}

/// Declare the model and its files, and open the upload.
async fn start(
    client: &ApiClient,
    ui: &Ui,
    opts: &ModelUploadOptions,
    store: &StateStore,
    scan: &Scan,
    metadata: Metadata<'_>,
) -> RmsResult<(String, Plan)> {
    // A whole-file hash lets the server prove, after assembling, that what it
    // stored is what was on this disk — and lets a later download prove the
    // same. It costs one extra read of the directory.
    let hashes: Vec<Option<String>> =
        if opts.no_checksum {
            vec![None; scan.files.len()]
        } else {
            ui.info(&format!(
                "Hashing {} file(s), {}, so the server can verify what it stored …",
                scan.files.len(),
                human_bytes(scan.total_bytes())
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
    let mut body = json!({ "name": metadata.name, "version": metadata.version, "files": files });
    let text = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    if let Some(t) = text(&opts.model_type) {
        body["modelType"] = json!(t);
    }
    if let Some(b) = text(&opts.base_model) {
        body["baseModel"] = json!(b);
    }
    if let Some(d) = text(&opts.dataset) {
        body["datasetId"] = json!(d);
    }
    if let Some(n) = text(&opts.notes) {
        body["notes"] = json!(n);
    }
    if !metadata.training_metrics.is_empty() {
        body["trainingMetrics"] = Value::Object(metadata.training_metrics);
    }
    if !metadata.validation_metrics.is_empty() {
        body["validationMetrics"] = Value::Object(metadata.validation_metrics);
    }
    if let Some(c) = opts.chunk_size {
        body["chunkSize"] = json!(c);
    }

    // Never re-sent after a network error: it may have opened the upload.
    let r = control(
        client,
        Method::POST,
        "/models/uploads",
        Some(&body),
        false,
        &opts.tuning,
    )
    .await?;
    if !r.is_success() {
        let mut error = api_error(&r);
        if r.status >= 500
            && r.code().is_none_or(|c| c.is_empty())
            && let RmsError::Api { message, .. } = &mut error
        {
            message.push_str(
                "\n  This answer came from a proxy, so the server may still have started the upload.\n  \
                 Before trying again, look for it (rmsctl models upload-status) and discard it with:\n  \
                 rmsctl models upload-cancel <id> --yes",
            );
        }
        return Err(error);
    }
    let started: StartResponse = parse(&r)?;

    // Noted before the first byte goes out, so an upload killed a second
    // later is still found again.
    let noted = store.save(&started.upload_id, metadata.name);
    let plan = build_plan(
        &scan.files,
        &started.files,
        &HashSet::new(),
        started.chunk_size,
    )?;

    ui.success(&format!(
        "Upload started: {} ({} store, {} parts, {} part(s))",
        started.upload_id,
        started.storage.as_deref().unwrap_or("unknown"),
        human_chunk(started.chunk_size),
        plan.missing_parts()
    ));
    if !noted {
        ui.warn(&format!(
            "Could not note this upload locally; to resume it pass --resume {}",
            started.upload_id
        ));
    }
    Ok((started.upload_id, plan))
}

/// `--restart`: remove the earlier upload before starting over.
async fn discard(client: &ApiClient, ui: &Ui, id: &str, tuning: &Tuning) -> RmsResult<()> {
    let r = control(
        client,
        Method::POST,
        &format!("{}/cancel", base(id)),
        None,
        true,
        tuning,
    )
    .await?;
    match (r.status, r.code().as_deref()) {
        (200..=299, _) => ui.info(&format!("Discarded the earlier upload {id}.")),
        (404, _) => {}
        // Already a registered model: not ours to delete on a restart.
        (409, Some("UPLOAD_COMPLETED" | "UPLOAD_PROCESSING")) => ui.warn(&format!(
            "The earlier upload {id} already finished and is kept as a model."
        )),
        _ => return Err(api_error(&r)),
    }
    Ok(())
}

/// Wait until the server has registered the model, and print it. `completed`
/// is success; anything else is an error so the exit code says so.
#[allow(clippy::too_many_arguments)]
async fn conclude(
    client: &ApiClient,
    ctx: &Ctx,
    ui: &Ui,
    opts: &ModelUploadOptions,
    store: &StateStore,
    upload_id: &str,
    parts_sent: usize,
    bytes_sent: u64,
) -> RmsResult<ModelUploadOutcome> {
    let tuning = &opts.tuning;
    let began = Instant::now();
    let mut announced = false;
    let verdict = loop {
        let Some((status, _)) = fetch_status(client, upload_id, tuning).await? else {
            store.remove();
            return Err(RmsError::Other(format!(
                "upload {upload_id} disappeared while it was being finished — was it cancelled?"
            )));
        };
        if status.state == "completed" || status.state == "failed" {
            break status;
        }
        if !announced {
            ui.info("The server is assembling the files and verifying checksums …");
            announced = true;
        }
        if began.elapsed() >= opts.wait_timeout {
            return Err(RmsError::Other(format!(
                "the server is still finishing upload {upload_id} after {}.\n  \
                 Nothing is lost — look again with: rmsctl models upload-status {upload_id}",
                human_duration(opts.wait_timeout.as_secs_f64())
            )));
        }
        tokio::time::sleep(tuning.poll_interval).await;
    };

    if verdict.state != "completed" {
        let reason = verdict
            .error
            .clone()
            .unwrap_or_else(|| "no reason recorded".into());
        return Err(RmsError::Other(format!(
            "upload {upload_id} failed on the server and no model was registered: {reason}.\n  \
             Its parts are kept until it is removed: rmsctl models upload-cancel {upload_id} --yes"
        )));
    }

    // Registered; the note has done its job.
    store.remove();
    let model_id = verdict
        .model_version_id
        .clone()
        .unwrap_or_else(|| upload_id.to_string());
    let model = client
        .get_value(&format!("/models/versions/{model_id}"))
        .await;

    if ctx.json {
        match &model {
            Ok(v) => output::json_out(v)?,
            Err(_) => println!(
                "{}",
                json!({ "uploadId": upload_id, "modelVersionId": model_id, "state": "completed" })
            ),
        }
    } else {
        output::success("Model registered");
        match &model {
            Ok(v) => {
                // The verdict stands even if the row has a shape this build cannot draw.
                if let Err(e) = super::models::render_result(v) {
                    output::field("ID", &model_id);
                    output::info(&format!("Model details unavailable: {e}"));
                }
            }
            Err(e) => {
                output::field("ID", &model_id);
                output::info(&format!("Model details unavailable: {e}"));
            }
        }
        println!();
        output::info(&format!("Files:    rmsctl models files {model_id}"));
        output::info(&format!(
            "Download: rmsctl models download {model_id} --out <dir>"
        ));
        println!();
    }

    Ok(ModelUploadOutcome {
        upload_id: upload_id.to_string(),
        state: verdict.state,
        model_version_id: Some(model_id),
        parts_sent,
        bytes_sent,
        executed: true,
    })
}

// ── upload-status ───────────────────────────────────────────────────────

/// `models upload-status [id]`: one upload in detail, or — without an id —
/// every upload that is not a registered model yet.
pub async fn status(client: &ApiClient, ctx: &Ctx, id: Option<&str>) -> RmsResult<()> {
    let tuning = Tuning::default();
    let Some(id) = id else {
        return list_open(client, ctx, &tuning).await;
    };
    let r = control(client, Method::GET, &base(id), None, true, &tuning).await?;
    if !r.is_success() {
        return Err(api_error(&r));
    }
    if ctx.json {
        return output::json_out(&r.json());
    }
    let s: UploadStatus = parse(&r)?;

    output::section("◈", &format!("Model upload: {}", s.label().bold()));
    output::field("ID", id);
    output::field("State", &upload_state(&s.state));
    if let Some(b) = &s.base_model {
        output::field("Base model", b);
    }
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
    if let Some(model) = &s.model_version_id {
        output::field("Model version", model);
    }
    if let Some(ts) = &s.last_activity_at {
        output::field("Last activity", ts);
    }
    if let Some(ts) = &s.expires_at {
        output::field("Expires", &format!("{ts} if nothing further happens"));
    }
    if let Some(reason) = &s.error {
        output::field_colored("Error", output::truncate(reason, 84).red());
    }

    // A multi-part file reads `complete: false` until finish assembles it,
    // even with every part held — what matters here is parts still to send.
    let parts_of = |f: &RemoteFile| {
        f.part_count
            .unwrap_or_else(|| part_count(f.size, s.chunk_size))
    };
    let unfinished: Vec<&RemoteFile> = s
        .files
        .iter()
        .filter(|f| !f.complete && f.received_parts.len() < parts_of(f) as usize)
        .collect();
    if s.state == "uploading" {
        let missing: usize = unfinished
            .iter()
            .map(|f| (parts_of(f) as usize).saturating_sub(f.received_parts.len()))
            .sum();
        output::field(
            "Missing parts",
            &if s.ready_to_finish {
                "none — ready to finish".to_string()
            } else {
                missing.to_string()
            },
        );
        if !unfinished.is_empty() {
            output::subsection(&format!("Unfinished files ({})", unfinished.len()));
            output::table_header(&[("PATH", 56), ("SIZE", 10), ("PARTS", 9)]);
            for f in unfinished.iter().take(20) {
                let parts = parts_of(f);
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
    }

    println!();
    match s.state.as_str() {
        "uploading" => {
            output::info(
                "Resume:  rmsctl models upload <dir> --version <v> --yes   (on another machine add --resume <id>)",
            );
            output::info(&format!("Discard: rmsctl models upload-cancel {id} --yes"));
        }
        "processing" => {
            output::info("Finish was accepted; the server is assembling and verifying.")
        }
        "completed" => output::info(&format!(
            "Registered: rmsctl models show {}",
            s.model_version_id.as_deref().unwrap_or(id)
        )),
        "failed" => output::info(&format!("Discard: rmsctl models upload-cancel {id} --yes")),
        _ => {}
    }
    println!();
    Ok(())
}

async fn list_open(client: &ApiClient, ctx: &Ctx, tuning: &Tuning) -> RmsResult<()> {
    let r = control(client, Method::GET, "/models/uploads", None, true, tuning).await?;
    if !r.is_success() {
        return Err(api_error(&r));
    }
    if ctx.json {
        return output::json_out(&r.json());
    }
    let resp: UploadsResponse = parse(&r)?;
    output::section("◈", "Model uploads in progress");
    if resp.uploads.is_empty() {
        output::info("No model upload is open. Finished uploads are models: rmsctl models list");
        println!();
        return Ok(());
    }
    output::table_header(&[
        ("ID", output::ID_W),
        ("NAME", 28),
        ("VERSION", 16),
        ("SIZE", 10),
        ("STATE", 14),
    ]);
    for u in &resp.uploads {
        println!(
            "    {} {} {} {} {}",
            output::pad(&u.upload_id, output::ID_W),
            output::pad(&output::truncate(u.name.as_deref().unwrap_or("-"), 28), 28),
            output::pad(
                &output::truncate(u.version.as_deref().unwrap_or("-"), 16),
                16
            ),
            output::pad(&human_bytes(u.total_bytes), 10),
            upload_state(&u.state),
        );
    }
    println!();
    output::info("Details: rmsctl models upload-status <id>");
    if let Some(at) = resp
        .uploads
        .iter()
        .filter_map(|u| u.created_at.as_deref())
        .min()
    {
        output::info(&format!("Oldest started {at}"));
    }
    println!();
    Ok(())
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
                "no model upload {id} on this server (already cancelled, expired, or not yours)"
            ),
        });
    };
    match s.state.as_str() {
        "processing" => {
            return Err(RmsError::InvalidInput(format!(
                "upload {id} is being assembled and verified and cannot be cancelled now — wait for it (rmsctl models upload-status {id})"
            )));
        }
        "completed" => {
            return Err(RmsError::InvalidInput(format!(
                "upload {id} is complete — it is a registered model now, not an upload to cancel.\n  \
                 To remove the model and its stored files: rmsctl models delete {} --yes",
                s.model_version_id.as_deref().unwrap_or(id)
            )));
        }
        _ => {}
    }

    if !output::confirm(
        &format!(
            "This will cancel model upload {id} (\"{}\") and delete the {} of {} already stored",
            s.label(),
            human_bytes(s.received_bytes),
            human_bytes(s.total_bytes)
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let r = control(
        client,
        Method::POST,
        &format!("{}/cancel", base(id)),
        None,
        true,
        &tuning,
    )
    .await?;
    if !r.is_success() {
        return Err(api_error(&r));
    }
    forget_dataset(state_dir, &note_server(client), id);

    if ctx.json {
        return output::json_out(&r.json());
    }
    output::success(&format!(
        "Upload {id} cancelled; its stored parts are removed."
    ));
    println!();
    Ok(())
}

// ── files ───────────────────────────────────────────────────────────────

/// `models files <id>`: what the instance stores for a model.
pub async fn files(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let tuning = Tuning::default();
    if ctx.json {
        // The server's own body: the first page, which is all of a model of up to 1000 files.
        let r = control(
            client,
            Method::GET,
            &format!("/models/versions/{id}/files"),
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
    let manifest = fetch_manifest(client, Kind::Model, id, &tuning).await?;
    let total: u64 = manifest.files.iter().map(|f| f.size).sum();

    output::section("◈", &format!("Model files: {}", id.bold()));
    if let Some(s) = &manifest.storage {
        output::field("Store", s);
    }
    output::field(
        "Total",
        &format!("{} in {} file(s)", human_bytes(total), manifest.files.len()),
    );
    println!();
    output::table_header(&[("PATH", 56), ("SIZE", 10), ("SHA-256", 14)]);
    for f in &manifest.files {
        println!(
            "    {} {} {}",
            output::pad(&output::truncate(&f.path, 56), 56),
            output::pad(&human_bytes(f.size), 10),
            f.sha256
                .as_deref()
                .map(|s| &s[..s.len().min(12)])
                .unwrap_or("-")
        );
    }
    println!();
    output::info(&format!(
        "Download: rmsctl models download {id} --out <dir>"
    ));
    println!();
    Ok(())
}

#[cfg(test)]
#[path = "model_upload_tests.rs"]
mod tests;
