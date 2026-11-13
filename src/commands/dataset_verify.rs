//! `datasets verify` — is everything in this directory on the instance?
//!
//! The check to run before a local dataset is deleted. It compares the
//! directory with the instance's file list (`GET /datasets/:id/files`, the
//! manifest `datasets download` works from) and downloads nothing else:
//!
//! - every local file must be listed, under the same relative path,
//! - with the same size,
//! - and the same SHA-256 wherever the instance has one (a dataset that was
//!   uploaded with `rmsctl datasets upload`; the server compared that hash
//!   with what it stored when the upload finished).
//!
//! The directory is read with the rules `datasets upload` reads it with, so
//! "verify says yes" means "what upload would send is what is stored". What
//! those rules leave out (`.git`, an `--exclude`) is listed, never passed over.
//!
//! Any difference is a non-zero exit, and so is a file the instance holds
//! that the directory does not: after this command succeeds, download followed
//! by `diff -r` is empty.

use super::dataset_download::{Kind, fetch_manifest};
use super::dataset_upload::{
    ScanRules, Tuning, Ui, api_error, control, human_bytes, scan_dir, sha256_range_async,
    skipped_lines,
};
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::output;
use colored::*;
use futures_util::{StreamExt, TryStreamExt, stream};
use reqwest::Method;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub struct VerifyOptions {
    pub id: String,
    pub dir: PathBuf,
    /// The same rules `datasets upload` takes.
    pub rules: ScanRules,
    /// Compare sizes only, even where the instance lists a SHA-256.
    pub no_checksum: bool,
    /// Count a file the instance has no SHA-256 for as a difference.
    pub require_checksum: bool,
    /// Files hashed at once.
    pub parallel: usize,
    pub tuning: Tuning,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DiffKind {
    /// In the directory, not on the instance.
    MissingOnInstance,
    /// On the instance, not in the directory.
    OnlyOnInstance,
    Size,
    Sha256,
    /// `--require-checksum`, and the instance lists none for this file.
    NoChecksum,
}

impl DiffKind {
    fn label(self) -> &'static str {
        match self {
            DiffKind::MissingOnInstance => "missing on the instance",
            DiffKind::OnlyOnInstance => "only on the instance",
            DiffKind::Size => "size differs",
            DiffKind::Sha256 => "content differs",
            DiffKind::NoChecksum => "no checksum",
        }
    }

    fn key(self) -> &'static str {
        match self {
            DiffKind::MissingOnInstance => "missingOnInstance",
            DiffKind::OnlyOnInstance => "onlyOnInstance",
            DiffKind::Size => "size",
            DiffKind::Sha256 => "sha256",
            DiffKind::NoChecksum => "noChecksum",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Difference {
    pub path: String,
    pub kind: DiffKind,
    pub detail: String,
}

/// What the comparison found; the fields are what the tests assert on and
/// what `--json` prints.
#[derive(Debug)]
pub struct VerifyOutcome {
    pub local_files: usize,
    pub local_bytes: u64,
    pub remote_files: usize,
    /// Local files whose SHA-256 was compared with the instance's.
    pub sha256_compared: usize,
    /// Local files that match by size and could not (or were told not to) be
    /// compared by hash.
    pub size_only: usize,
    pub differences: Vec<Difference>,
    /// Entries of the directory the scan rules left out: `(path, why)`.
    pub left_out: Vec<(String, String)>,
}

impl VerifyOutcome {
    pub fn matches(&self) -> bool {
        self.differences.is_empty()
    }
}

/// Entry point for `datasets verify`: differences are an error, so the exit
/// code can gate an `rm -rf`.
pub async fn run_verify(client: &ApiClient, ctx: &Ctx, opts: VerifyOptions) -> RmsResult<()> {
    let outcome = verify(client, ctx, &opts).await?;
    if outcome.matches() {
        return Ok(());
    }
    Err(RmsError::Other(format!(
        "{} does not match dataset {}: {} difference(s) — do not delete the local copy",
        opts.dir.display(),
        opts.id,
        outcome.differences.len()
    )))
}

pub async fn verify(
    client: &ApiClient,
    ctx: &Ctx,
    opts: &VerifyOptions,
) -> RmsResult<VerifyOutcome> {
    let ui = Ui::new(ctx.json);
    let tuning = &opts.tuning;
    let id = opts.id.as_str();

    let root = std::fs::canonicalize(&opts.dir)
        .map_err(|e| RmsError::InvalidInput(format!("cannot open {}: {e}", opts.dir.display())))?;
    if !root.is_dir() {
        return Err(RmsError::InvalidInput(format!(
            "{} is not a directory — pass the dataset root",
            root.display()
        )));
    }
    let scan = scan_dir(&root, &opts.rules)?;

    // The dataset itself first, so a wrong id reads "not found".
    let r = control(
        client,
        Method::GET,
        &format!("/datasets/{id}"),
        None,
        true,
        tuning,
    )
    .await?;
    if !r.is_success() {
        return Err(api_error(&r));
    }
    let body = r.json();
    let dataset = body.get("dataset").unwrap_or(&body);
    let text = |key: &str| dataset.get(key).and_then(Value::as_str);
    let name = text("name").unwrap_or(id).to_string();
    let status = text("status").unwrap_or("-").to_string();

    let manifest = fetch_manifest(client, Kind::Dataset, id, tuning).await?;
    let remote: HashMap<&str, _> = manifest
        .files
        .iter()
        .map(|f| (f.path.as_str(), f))
        .collect();
    let remote_bytes: u64 = manifest.files.iter().map(|f| f.size).sum();

    let mut differences: Vec<Difference> = Vec::new();
    let mut to_hash = Vec::new();
    let mut size_only = 0usize;
    for file in &scan.files {
        let Some(entry) = remote.get(file.rel.as_str()) else {
            differences.push(Difference {
                path: file.rel.clone(),
                kind: DiffKind::MissingOnInstance,
                detail: format!("{} locally", human_bytes(file.size)),
            });
            continue;
        };
        if entry.size != file.size {
            differences.push(Difference {
                path: file.rel.clone(),
                kind: DiffKind::Size,
                detail: format!(
                    "{} bytes locally, {} on the instance",
                    file.size, entry.size
                ),
            });
            continue;
        }
        match entry.sha256.as_deref() {
            Some(sha) if !opts.no_checksum => to_hash.push((file, sha.to_ascii_lowercase())),
            Some(_) => size_only += 1,
            None if opts.require_checksum => differences.push(Difference {
                path: file.rel.clone(),
                kind: DiffKind::NoChecksum,
                detail: "the instance lists no SHA-256 for it (--require-checksum)".into(),
            }),
            None => size_only += 1,
        }
    }

    let local_paths: HashSet<&str> = scan.files.iter().map(|f| f.rel.as_str()).collect();
    for entry in &manifest.files {
        if local_paths.contains(entry.path.as_str()) {
            continue;
        }
        // Say so when the file is there but the scan rules passed it over.
        let left_out = scan.skipped.iter().find(|s| {
            s.rel == entry.path || (s.rel.ends_with('/') && entry.path.starts_with(s.rel.as_str()))
        });
        differences.push(Difference {
            path: entry.path.clone(),
            kind: DiffKind::OnlyOnInstance,
            detail: match left_out {
                Some(s) => format!(
                    "{} — left out of the local side: {}",
                    human_bytes(entry.size),
                    s.why
                ),
                None => format!("{} — not in the directory", human_bytes(entry.size)),
            },
        });
    }

    if !ui.quiet {
        println!();
        output::section("◈", &format!("Verify: {name}"));
        println!();
    }
    ui.field("Dataset", id);
    ui.field("Status", &status);
    ui.field(
        "Local",
        &format!(
            "{} — {} file(s), {}",
            root.display(),
            scan.files.len(),
            human_bytes(scan.total_bytes())
        ),
    );
    ui.field(
        "Instance",
        &format!(
            "{} file(s), {}{}",
            manifest.files.len(),
            human_bytes(remote_bytes),
            manifest
                .storage
                .as_deref()
                .map(|s| format!(" · {s} store"))
                .unwrap_or_default()
        ),
    );

    // Hashing reads every byte of the local side once; nothing is fetched.
    let sha256_compared = to_hash.len();
    if !to_hash.is_empty() {
        let bytes: u64 = to_hash.iter().map(|(f, _)| f.size).sum();
        ui.info(&format!(
            "Hashing {} local file(s), {} …",
            to_hash.len(),
            human_bytes(bytes)
        ));
        let hashed: Vec<Option<Difference>> =
            stream::iter(to_hash.iter().map(|(file, expected)| async move {
                let actual = sha256_range_async(file.abs.clone(), 0, file.size).await?;
                Ok::<_, RmsError>((actual != *expected).then(|| Difference {
                    path: file.rel.clone(),
                    kind: DiffKind::Sha256,
                    detail: format!(
                        "same size, SHA-256 {}… locally, {}… on the instance",
                        &actual[..12.min(actual.len())],
                        &expected[..12.min(expected.len())]
                    ),
                }))
            }))
            .buffered(opts.parallel.max(1))
            .try_collect()
            .await?;
        differences.extend(hashed.into_iter().flatten());
    }
    differences.sort_by(|a, b| a.path.cmp(&b.path));

    let outcome = VerifyOutcome {
        local_files: scan.files.len(),
        local_bytes: scan.total_bytes(),
        remote_files: manifest.files.len(),
        sha256_compared,
        size_only,
        differences,
        left_out: scan
            .skipped
            .iter()
            .map(|s| (s.rel.clone(), s.why.clone()))
            .collect(),
    };

    if ctx.json {
        output::json_out(&json!({
            "datasetId": id,
            "name": name,
            "status": status,
            "dir": root.display().to_string(),
            "match": outcome.matches(),
            "localFiles": outcome.local_files,
            "localBytes": outcome.local_bytes,
            "remoteFiles": outcome.remote_files,
            "remoteBytes": remote_bytes,
            "sha256Compared": outcome.sha256_compared,
            "sizeOnly": outcome.size_only,
            "differences": outcome.differences.iter().map(|d| json!({
                "path": d.path, "kind": d.kind.key(), "detail": d.detail,
            })).collect::<Vec<_>>(),
            "leftOut": outcome.left_out.iter().map(|(path, why)| json!({
                "path": path, "reason": why,
            })).collect::<Vec<_>>(),
        }))?;
        return Ok(outcome);
    }

    output::field(
        "Compared",
        &format!(
            "path and size of every file · SHA-256 of {}{}",
            outcome.sha256_compared,
            match outcome.size_only {
                0 => String::new(),
                n => format!(" · {n} by size only"),
            }
        ),
    );
    if !scan.skipped.is_empty() {
        output::field(
            "Left out",
            &format!(
                "{} entr{} of the directory — not compared, NOT on the instance:",
                scan.skipped.len(),
                if scan.skipped.len() == 1 { "y" } else { "ies" }
            ),
        );
        for line in skipped_lines(&scan.skipped, usize::MAX) {
            println!("      {} {line}", "-".bright_black());
        }
    }
    println!();

    if status != "ready" {
        output::warn(&format!(
            "The dataset is {status}, not ready — its files were compared as they are stored."
        ));
    }
    if outcome.matches() {
        output::success(&format!(
            "Every one of the {} file(s), {}, is on the instance with the same path and size{}",
            outcome.local_files,
            human_bytes(outcome.local_bytes),
            match (outcome.sha256_compared, outcome.size_only) {
                (0, _) => String::new(),
                (n, 0) => format!(", and the same SHA-256 ({n} files)"),
                (n, _) => format!(", and the same SHA-256 for {n} of them"),
            }
        ));
        if outcome.size_only > 0 {
            output::warn(&format!(
                "{} file(s) were compared by size only: {}.",
                outcome.size_only,
                if opts.no_checksum {
                    "--no-checksum"
                } else {
                    "the instance has no SHA-256 for them (uploaded with --no-checksum, or not uploaded by rmsctl)"
                }
            ));
        }
        if !scan.skipped.is_empty() {
            output::warn(
                "The left-out entries above exist only in this directory; deleting it deletes them.",
            );
        }
    } else {
        output::warn(&format!(
            "{} difference(s) between {} and the instance:",
            outcome.differences.len(),
            root.display()
        ));
        for d in &outcome.differences {
            println!(
                "      {} {:<24} {}  {}",
                "-".bright_black(),
                d.kind.label().red(),
                d.path.bold(),
                format!("({})", d.detail).bright_black()
            );
        }
    }
    println!();
    Ok(outcome)
}

#[cfg(test)]
#[path = "dataset_verify_tests.rs"]
mod tests;
