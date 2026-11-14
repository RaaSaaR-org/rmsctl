use super::dataset_download;
use super::dataset_upload;
use super::dataset_verify;
use crate::cli::DatasetsCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::models::Pagination;
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

pub async fn run(cmd: &DatasetsCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        DatasetsCommand::List => list(client, ctx).await,
        DatasetsCommand::Show { id } => show(client, ctx, &resolve_id(client, id).await?).await,
        DatasetsCommand::Episodes { id } => {
            episodes(client, ctx, &resolve_id(client, id).await?).await
        }
        DatasetsCommand::Files { id } => {
            dataset_download::files(
                client,
                ctx,
                dataset_download::Kind::Dataset,
                &resolve_id(client, id).await?,
            )
            .await
        }
        DatasetsCommand::Quality { id } => quality(client, ctx, id).await,
        DatasetsCommand::Stats { id } => stats(client, ctx, id).await,
        DatasetsCommand::Validate { id, advanced } => validate(client, ctx, id, *advanced).await,
        DatasetsCommand::Push {
            id,
            hf_token,
            repo_id,
        } => push(client, ctx, id, hf_token, repo_id).await,
        DatasetsCommand::PushStatus { id } => push_status(client, ctx, id).await,
        DatasetsCommand::Upload {
            dir,
            exclude,
            lerobot_only,
            no_default_excludes,
            name,
            description,
            robot_type,
            chunk_size,
            parallel,
            retries,
            resume,
            restart,
            no_checksum,
            no_wait,
            wait_timeout,
            part_timeout,
        } => {
            let opts = dataset_upload::UploadOptions {
                dir: dir.clone(),
                name: name.clone(),
                description: description.clone(),
                robot_type: robot_type.clone(),
                chunk_size: *chunk_size,
                parallel: *parallel as usize,
                retries: *retries,
                resume: resume.clone(),
                restart: *restart,
                no_checksum: *no_checksum,
                no_wait: *no_wait,
                wait_timeout: Duration::from_secs(*wait_timeout),
                part_timeout: Duration::from_secs((*part_timeout).max(1)),
                rules: dataset_upload::ScanRules {
                    exclude: exclude.clone(),
                    lerobot_only: *lerobot_only,
                    no_default_excludes: *no_default_excludes,
                },
                state_dir: None,
                tuning: dataset_upload::Tuning::default(),
            };
            dataset_upload::run_upload(client, ctx, opts).await
        }
        DatasetsCommand::Download {
            id,
            out,
            include,
            parallel,
            retries,
            resume,
            force,
            no_checksum,
            stall_timeout,
        } => {
            let opts = dataset_download::DownloadOptions {
                kind: dataset_download::Kind::Dataset,
                id: resolve_id(client, id).await?,
                out: out.clone(),
                include: include.clone(),
                parallel: *parallel as usize,
                retries: *retries,
                resume: *resume,
                force: *force,
                no_checksum: *no_checksum,
                stall_timeout: Duration::from_secs((*stall_timeout).max(1)),
                tuning: dataset_upload::Tuning::default(),
            };
            dataset_download::run_download(client, ctx, opts).await
        }
        DatasetsCommand::Verify {
            id,
            dir,
            exclude,
            lerobot_only,
            no_default_excludes,
            no_checksum,
            require_checksum,
            parallel,
        } => {
            let opts = dataset_verify::VerifyOptions {
                id: id.clone(),
                dir: dir.clone(),
                rules: dataset_upload::ScanRules {
                    exclude: exclude.clone(),
                    lerobot_only: *lerobot_only,
                    no_default_excludes: *no_default_excludes,
                },
                no_checksum: *no_checksum,
                require_checksum: *require_checksum,
                parallel: *parallel as usize,
                tuning: dataset_upload::Tuning::default(),
            };
            dataset_verify::run_verify(client, ctx, opts).await
        }
        DatasetsCommand::UploadStatus { id } => dataset_upload::status(client, ctx, id).await,
        DatasetsCommand::UploadCancel { id } => dataset_upload::cancel(client, ctx, id, None).await,
        DatasetsCommand::Flagged { id } => flagged(client, ctx, id).await,
        DatasetsCommand::Flag { id, index, reason } => {
            flag(client, ctx, id, *index, reason.as_deref()).await
        }
    }
}

// ── Response shapes ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatasetsResponse {
    #[serde(default)]
    datasets: Vec<Dataset>,
    #[serde(default)]
    pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatasetEnvelope {
    #[serde(default)]
    dataset: Option<Dataset>,
}

/// A resolved reference the server attaches so a client does not have to
/// re-fetch the row: `robotType` next to `robotTypeId`, `skill` next to
/// `skillId`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct NamedRef {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    demonstration_count: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Dataset {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    status: Option<String>,
    // 'materialized' | 'view' (TASK-240). A view is a Dataset row that owns no
    // bytes, only a selection of its parent's episodes — without this column a
    // fork and the dataset it was forked from are indistinguishable.
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    parent_dataset_id: Option<String>,
    #[serde(default)]
    parent: Option<NamedRef>,
    #[serde(default)]
    frozen_at: Option<String>,
    #[serde(default)]
    materialized_path: Option<String>,
    #[serde(default)]
    selection: Option<Value>,
    #[serde(default)]
    robot_type_id: Option<String>,
    #[serde(default)]
    robot_type: Option<NamedRef>,
    // Absent on every row this server holds: the domain mapper turns a null
    // `skillId` into `undefined` and JSON.stringify drops it.
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    skill: Option<NamedRef>,
    #[serde(default)]
    fps: Option<f64>,
    #[serde(default)]
    total_frames: Option<f64>,
    #[serde(default)]
    total_duration: Option<f64>,
    #[serde(default)]
    demonstration_count: Option<f64>,
    #[serde(default)]
    quality_score: Option<f64>,
    #[serde(default)]
    quality_breakdown: Option<QualityBreakdown>,
    #[serde(default)]
    validation: Option<ValidationSummary>,
    #[serde(default)]
    storage_path: Option<String>,
    #[serde(default)]
    lerobot_version: Option<String>,
    #[serde(default)]
    hugging_face_repo_id: Option<String>,
    #[serde(default)]
    source_revision: Option<String>,
    #[serde(default)]
    source_license: Option<String>,
    #[serde(default)]
    import_mode: Option<String>,
    #[serde(default)]
    import_error: Option<String>,
    #[serde(default)]
    info_json: Option<Value>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

impl Dataset {
    fn is_view(&self) -> bool {
        self.kind.as_deref() == Some("view") || self.parent_dataset_id.is_some()
    }

    fn label(&self) -> &str {
        self.name.as_deref().unwrap_or("(unnamed)")
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QualityBreakdown {
    #[serde(default)]
    demonstration_count: Option<f64>,
    #[serde(default)]
    duration: Option<f64>,
    // Named `diversity` since before it measured anything; the server now fills
    // it with sensor coverage + file integrity (0-20).
    #[serde(default)]
    diversity: Option<f64>,
    #[serde(default)]
    format_compliance: Option<f64>,
    #[serde(default)]
    total: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationSummary {
    #[serde(default)]
    validated_at: Option<String>,
    #[serde(default)]
    valid: Option<bool>,
    #[serde(default)]
    lerobot_version: Option<String>,
    // Kept as raw values: the stored reports carry `{code, message}` objects,
    // but older rows hold bare strings and a typed Vec would fail the parse.
    #[serde(default)]
    errors: Option<Vec<Value>>,
    #[serde(default)]
    warnings: Option<Vec<Value>>,
    #[serde(default)]
    image_keys: Option<Vec<String>>,
    #[serde(default)]
    file_count: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewsResponse {
    #[serde(default)]
    views: Vec<ViewSummary>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewSummary {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    parent_dataset_id: Option<String>,
    #[serde(default)]
    parent_name: Option<String>,
    #[serde(default)]
    parent_demonstration_count: Option<f64>,
    #[serde(default)]
    root_dataset_id: Option<String>,
    #[serde(default)]
    demonstration_count: Option<f64>,
    #[serde(default)]
    selection: Option<Value>,
    #[serde(default)]
    frozen_at: Option<String>,
    #[serde(default)]
    materialized_path: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EpisodesResponse {
    #[serde(default)]
    episodes: Vec<Episode>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Episode {
    #[serde(default)]
    index: Option<u64>,
    #[serde(default)]
    frame_count: Option<f64>,
    #[serde(default)]
    duration_seconds: Option<f64>,
    #[serde(default)]
    flagged: Option<bool>,
    // v3.0 chunked video only: `{ <camera>: {from, to, chunk, file} }`, attached
    // per episode so a player can seek its slice of a concatenated mp4.
    #[serde(default)]
    video_windows: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QualityResponse {
    #[serde(default)]
    has_quality_report: Option<bool>,
    #[serde(default)]
    report: Option<QualityReport>,
    #[serde(default)]
    validation: Option<ValidationSummary>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QualityReport {
    #[serde(default)]
    dataset_name: Option<String>,
    #[serde(default)]
    generated_at: Option<String>,
    #[serde(default)]
    overall_score: Option<f64>,
    #[serde(default)]
    score_breakdown: Option<QualityBreakdown>,
    #[serde(default)]
    trajectory_count: Option<f64>,
    #[serde(default)]
    flagged_trajectory_count: Option<f64>,
    #[serde(default)]
    anomalous_trajectory_count: Option<f64>,
    #[serde(default)]
    clean_trajectory_percentage: Option<f64>,
    #[serde(default)]
    flagged_summary: Option<Vec<FlaggedEpisode>>,
    #[serde(default)]
    validation_status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsResponse {
    #[serde(default)]
    has_stats: Option<bool>,
    #[serde(default)]
    computed_at: Option<String>,
    // `{ <feature>: {min, max, mean, std, count} }`, each value an array whose
    // width is the feature's dimensionality (and nested for image features).
    #[serde(default)]
    stats: Option<serde_json::Map<String, Value>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FlaggedResponse {
    #[serde(default)]
    total: Option<u32>,
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    flagged: Vec<FlaggedEpisode>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FlaggedEpisode {
    #[serde(default)]
    trajectory_index: Option<u64>,
    #[serde(default)]
    flag_reason: Option<String>,
    #[serde(default)]
    anomaly_types: Option<Vec<String>>,
    #[serde(default)]
    quality_score: Option<f64>,
    #[serde(default)]
    reviewed: Option<bool>,
    #[serde(default)]
    reviewed_at: Option<String>,
    #[serde(default)]
    reviewed_by: Option<String>,
    #[serde(default)]
    review_decision: Option<String>,
}

// ── Local formatting helpers ────────────────────────────────────────────

/// The dataset status vocabulary, which `output::status_indicator` does not
/// know: it renders the healthy terminal state `ready` through its catch-all,
/// so a ready dataset looks exactly like `unknown`.
fn dataset_status(status: &str) -> String {
    let (icon, colored) = match status.to_lowercase().as_str() {
        "ready" => ("●", status.green().bold()),
        "uploading" | "validating" | "importing" | "processing" => {
            ("◐", status.truecolor(255, 103, 0).bold())
        }
        "failed" => ("●", status.red().bold()),
        _ => return output::status_indicator(status),
    };
    format!("{} {}", icon.bright_black(), colored)
}

fn kind_label(kind: Option<&str>) -> ColoredString {
    match kind {
        Some("view") => "view".cyan().bold(),
        Some("materialized") => "materialized".bright_black(),
        Some(other) => other.normal(),
        None => "-".bright_black(),
    }
}

fn thousands(n: f64) -> String {
    let neg = n < 0.0;
    let digits = format!("{:.0}", n.abs());
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    if neg { format!("-{out}") } else { out }
}

fn opt_count(v: Option<f64>) -> String {
    v.map(thousands).unwrap_or_else(|| "-".to_string())
}

fn hms(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{:.1}s", seconds)
    }
}

/// A number as short as it can be without lying: integers grouped, tiny
/// magnitudes in exponent form rather than as `0.000`.
fn fmt_num(v: f64) -> String {
    if v == 0.0 {
        "0".into()
    } else if v.fract() == 0.0 && v.abs() < 1e15 {
        thousands(v)
    } else if v.abs() < 1e-3 || v.abs() >= 1e9 {
        format!("{v:.2e}")
    } else {
        format!("{v:.3}")
    }
}

/// The first scalar inside a stats value, which is an array of the feature's
/// width (and an array of arrays for image features).
fn first_num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Array(a) => a.iter().find_map(first_num),
        _ => None,
    }
}

fn width_of(v: Option<&Value>) -> usize {
    match v {
        Some(Value::Array(a)) => a.len(),
        Some(_) => 1,
        None => 0,
    }
}

/// One validation error or warning, whichever shape it was stored in.
fn issue_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(o) => {
            let msg = o
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("(no message)");
            match o.get("code").and_then(|c| c.as_str()) {
                Some(code) => format!("[{code}] {msg}"),
                None => msg.to_string(),
            }
        }
        other => other.to_string(),
    }
}

/// `name (uuid)` when the server resolved the reference, the bare id when it
/// did not — the detail response carries both and printing only the UUID makes
/// the operator go look up what they already have.
fn named(resolved: Option<&NamedRef>, id: Option<&str>) -> String {
    let name = resolved.and_then(|r| r.name.as_deref());
    let uuid = resolved.and_then(|r| r.id.as_deref()).or(id);
    match (name, uuid) {
        (Some(n), Some(u)) => format!("{n} {}", format!("({u})").bright_black()),
        (Some(n), None) => n.to_string(),
        (None, Some(u)) => u.to_string(),
        (None, None) => "-".to_string(),
    }
}

/// How many episodes a view selects, and how the selection was arrived at.
fn selection_summary(selection: Option<&Value>) -> Option<String> {
    let sel = selection?;
    let episodes = sel
        .get("episodes")
        .and_then(|e| e.as_array())
        .map(|a| a.len());
    let origin = sel
        .get("origin")
        .and_then(|o| o.get("kind"))
        .and_then(|k| k.as_str());
    match (episodes, origin) {
        (Some(n), Some(k)) => Some(format!("{n} episode(s), origin: {k}")),
        (Some(n), None) => Some(format!("{n} episode(s)")),
        (None, Some(k)) => Some(format!("origin: {k}")),
        (None, None) => None,
    }
}

fn total_pages_for(total: Option<u32>, limit: Option<u32>) -> Option<u32> {
    match (total, limit) {
        (Some(t), Some(l)) if l > 0 => Some(t.div_ceil(l)),
        _ => None,
    }
}

// ── list ────────────────────────────────────────────────────────────────

async fn list(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    // This route spells its page size `limit`, not `pageSize`, and defaults to
    // 20 rows whether or not the caller asks for paging.
    let query = ctx.paging_limit();

    if ctx.json {
        return output::json_out(&client.get_value_query("/datasets", &query).await?);
    }

    let resp: DatasetsResponse = client.get_query("/datasets", &query).await?;

    output::section("◈", "Datasets");

    if resp.datasets.is_empty() {
        output::info("No datasets registered.");
        println!();
        return Ok(());
    }

    // NAME grows with the terminal; the other columns keep their widths. On a
    // terminal the full table does not fit, ids are cut to 8 characters (the
    // commands that take an id resolve that prefix) and KIND is left out when
    // every row has the same one, which the footer then names.
    let term = output::term_width();
    let layout = list_layout(
        term,
        resp.datasets.iter().map(|d| d.kind.as_deref()).collect(),
    );
    let mut others = vec![layout.id_w];
    if layout.kind_column {
        others.push(13);
    }
    others.extend([12, 9, 11, 7]);
    let name_w = output::flex_width(
        layout.name_min,
        output::fixed_width(&others),
        output::longest(resp.datasets.iter().map(|d| d.label())),
        term,
    );
    let mut columns = vec![("ID", layout.id_w)];
    if layout.kind_column {
        columns.push(("KIND", 13));
    }
    columns.extend([
        ("NAME", name_w),
        ("STATUS", 12),
        ("EPISODES", 9),
        ("FRAMES", 11),
        ("QUALITY", 7),
    ]);
    output::table_header(&columns);

    for d in &resp.datasets {
        let kind = if layout.kind_column {
            format!(
                "{} ",
                output::pad(&kind_label(d.kind.as_deref()).to_string(), 13)
            )
        } else {
            String::new()
        };
        println!(
            "    {} {}{} {} {} {} {}",
            output::pad(&output::id_cell(&d.id, layout.id_w), layout.id_w),
            kind,
            output::pad(&output::truncate(d.label(), name_w), name_w),
            output::pad(
                &dataset_status(d.status.as_deref().unwrap_or("unknown")),
                12
            ),
            output::pad(&opt_count(d.demonstration_count), 9),
            output::pad(&opt_count(d.total_frames), 11),
            match d.quality_score {
                Some(q) => format!("{q:.0}"),
                None => "-".bright_black().to_string(),
            }
        );

        // The listing applies no `kind` filter, so views sit interleaved with
        // the datasets they were forked from. Name the parent under the row.
        if d.is_view() {
            let parent = d
                .parent
                .as_ref()
                .and_then(|p| p.name.as_deref())
                .unwrap_or("unknown parent");
            let pid = d
                .parent_dataset_id
                .as_deref()
                .or_else(|| d.parent.as_ref().and_then(|p| p.id.as_deref()))
                .unwrap_or("-");
            let pid = output::id_cell(pid, layout.id_w);
            println!(
                "    {} {}",
                "└".bright_black(),
                format!("view of {parent} ({pid})").bright_black()
            );
        }
    }

    let views = resp.datasets.iter().filter(|d| d.is_view()).count();
    let pag = resp.pagination.as_ref();
    output::count_paged(
        "dataset(s)",
        resp.datasets.len(),
        pag.and_then(|p| p.total()),
        pag.and_then(|p| p.total_pages()),
    );
    if views > 0 {
        output::info(&format!(
            "{views} of these are views — a view owns no bytes, only a selection of its parent's episodes."
        ));
    }
    for note in layout.notes(resp.datasets.first().and_then(|d| d.kind.as_deref())) {
        output::info(&note);
    }
    println!();
    Ok(())
}

/// How `datasets list` fits the terminal.
#[derive(Debug, PartialEq)]
struct ListLayout {
    id_w: usize,
    kind_column: bool,
    name_min: usize,
}

impl ListLayout {
    /// What the footer has to say about what the narrow table left out, one
    /// short line each so the footer does not wrap where the table did not.
    fn notes(&self, kind: Option<&str>) -> Vec<String> {
        let mut notes = Vec::new();
        if self.id_w < 36 {
            notes.push(format!(
                "IDs cut to {} characters; show, episodes, files and download take them.",
                output::SHORT_ID_W
            ));
        }
        if !self.kind_column {
            notes.push(format!(
                "KIND left out: every row is {}.",
                kind.unwrap_or("-")
            ));
        }
        notes
    }
}

/// The full table (36-character ids, KIND) whenever it fits or there is no
/// terminal; otherwise short ids, a narrower NAME minimum, and no KIND column
/// when every row has the same kind.
fn list_layout(term: Option<usize>, kinds: Vec<Option<&str>>) -> ListLayout {
    let full = output::fixed_width(&[output::ID_W, 13, 12, 9, 11, 7]);
    if !output::too_narrow(full, 26, term) {
        return ListLayout {
            id_w: output::ID_W,
            kind_column: true,
            name_min: 26,
        };
    }
    let same_kind = kinds.windows(2).all(|w| w[0] == w[1]);
    ListLayout {
        id_w: output::SHORT_ID_W,
        kind_column: !same_kind,
        name_min: output::NARROW_NAME_MIN,
    }
}

/// Rows asked for per page when a short id is looked up in the listing.
const RESOLVE_PAGE: u32 = 200;

/// `ident` as a full dataset id.
///
/// The server looks a dataset up by its whole id only, so `datasets list`'s
/// short ids would 404. An argument of 8 or more hex characters that is not a
/// whole UUID is matched against the ids of every listed dataset (views
/// included); anything else goes to the server as typed. A prefix that matches
/// no id, or more than one, is refused with the candidates.
pub(super) async fn resolve_id(client: &ApiClient, ident: &str) -> RmsResult<String> {
    if !output::is_id_prefix(ident) {
        return Ok(ident.to_string());
    }
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut page = 1u32;
    loop {
        let query = [
            ("page", page.to_string()),
            ("limit", RESOLVE_PAGE.to_string()),
        ];
        let resp: DatasetsResponse = client.get_query("/datasets", &query).await?;
        let got = resp.datasets.len();
        rows.extend(
            resp.datasets
                .iter()
                .map(|d| (d.id.clone(), d.label().to_string())),
        );
        let pages = resp
            .pagination
            .as_ref()
            .and_then(|p| p.total_pages())
            .unwrap_or(1);
        // A page cap: a listing that never ends must not loop forever.
        if got == 0 || page >= pages || page >= 100 {
            break;
        }
        page += 1;
    }
    match_prefix(&rows, ident)
}

fn match_prefix(rows: &[(String, String)], ident: &str) -> RmsResult<String> {
    output::match_id_prefix(rows.iter().map(|(id, _)| id.as_str()), ident).map_err(|hits| {
        if hits.is_empty() {
            return RmsError::InvalidInput(format!(
                "no dataset id starts with '{ident}' — see 'rmsctl datasets list'"
            ));
        }
        let candidates = hits
            .iter()
            .map(|id| {
                let name = rows
                    .iter()
                    .find(|(rid, _)| rid == id)
                    .map(|(_, n)| n.as_str())
                    .unwrap_or("-");
                format!("{id} ({name})")
            })
            .collect::<Vec<_>>()
            .join(", ");
        RmsError::InvalidInput(format!(
            "'{ident}' is the start of {} dataset ids — pass more of the id: {candidates}",
            hits.len()
        ))
    })
}

// ── show ────────────────────────────────────────────────────────────────

async fn show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/datasets/{id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let env: DatasetEnvelope = client.get(&path).await?;
    let d = env
        .dataset
        .ok_or_else(|| RmsError::Other(format!("the server returned no dataset for {id}")))?;

    output::section("◈", &format!("Dataset: {}", d.label().bold()));

    output::field("ID", &d.id);
    output::field_colored("Kind", kind_label(d.kind.as_deref()));
    output::field(
        "Status",
        &dataset_status(d.status.as_deref().unwrap_or("unknown")),
    );
    if let Some(desc) = &d.description {
        output::field("Description", &output::truncate(desc, 88));
    }
    output::field(
        "Robot type",
        &named(d.robot_type.as_ref(), d.robot_type_id.as_deref()),
    );
    output::field("Skill", &named(d.skill.as_ref(), d.skill_id.as_deref()));
    output::field("Episodes", &opt_count(d.demonstration_count));
    output::field("Frames", &opt_count(d.total_frames));
    if let Some(secs) = d.total_duration {
        output::field("Duration", &hms(secs));
    }
    if let Some(fps) = d.fps {
        output::field("FPS", &format!("{fps:.2}"));
    }
    if let Some(v) = &d.lerobot_version {
        output::field("LeRobot", v);
    }
    match d.quality_score {
        Some(q) => output::field("Quality", &format!("{q:.0} / 100")),
        None => output::field_colored("Quality", "not scored".bright_black()),
    }
    if let Some(repo) = &d.hugging_face_repo_id {
        output::field("HF repo", repo);
    }
    if let Some(rev) = &d.source_revision {
        output::field("Source rev", rev);
    }
    if let Some(lic) = &d.source_license {
        output::field("License", lic);
    }
    if let Some(mode) = &d.import_mode {
        output::field("Import mode", mode);
    }
    if let Some(err) = &d.import_error {
        output::field_colored("Import error", output::truncate(err, 84).red());
    }
    if let Some(p) = &d.storage_path {
        output::field("Storage", &output::truncate(p, 88));
    }
    if let Some(ts) = &d.created_at {
        output::field("Created", ts);
    }
    if let Some(ts) = &d.updated_at {
        output::field("Updated", ts);
    }
    // `info.json` is the LeRobot manifest the importer copied; the only part of
    // it worth a line here is the codebase version it declares, which can
    // disagree with the column above on a converted dataset.
    if let Some(v) = d
        .info_json
        .as_ref()
        .and_then(|i| i.get("codebase_version"))
        .and_then(|r| r.as_str())
        && Some(v) != d.lerobot_version.as_deref()
    {
        output::field_colored(
            "info.json",
            format!("codebase_version: {v} — disagrees with the row").yellow(),
        );
    }

    // What makes a view a view (TASK-240). Without this block a fork looks
    // exactly like the dataset it was forked from, and its episode counts read
    // as bytes it does not own.
    if d.is_view() {
        output::subsection("View");
        output::field(
            "Parent",
            &named(d.parent.as_ref(), d.parent_dataset_id.as_deref()),
        );
        if let Some(n) = d.parent.as_ref().and_then(|p| p.demonstration_count) {
            output::field(
                "Parent episodes",
                &format!(
                    "{} (this view selects {})",
                    thousands(n),
                    opt_count(d.demonstration_count)
                ),
            );
        }
        if let Some(sel) = selection_summary(d.selection.as_ref()) {
            output::field("Selection", &sel);
        }
        match &d.frozen_at {
            Some(ts) => output::field_colored(
                "Frozen",
                format!("{ts} — cited by a run, cannot be deleted").yellow(),
            ),
            None => output::field_colored("Frozen", "no".bright_black()),
        }
        match &d.materialized_path {
            Some(p) => output::field("Materialized", &output::truncate(p, 84)),
            None => output::field_colored("Materialized", "no — resolved on read".bright_black()),
        }
        output::info(
            "A view stores no episodes of its own; its validation report is borrowed from the root dataset.",
        );
    }

    if let Some(b) = &d.quality_breakdown {
        render_breakdown(b);
    }
    if let Some(v) = &d.validation {
        render_validation(v);
    }

    // `GET /datasets/{id}/views` has no subcommand of its own, so the forks of
    // this dataset are folded in here — they are the only way to tell that
    // someone has already curated a selection from it.
    match client
        .get::<ViewsResponse>(&format!("/datasets/{id}/views"))
        .await
    {
        Ok(v) if !v.views.is_empty() => {
            output::subsection(&format!("Forks ({} view(s))", v.views.len()));
            output::table_header(&[
                ("ID", output::ID_W),
                ("NAME", 24),
                ("EPISODES", 9),
                ("FROZEN", 8),
                ("SELECTION", 28),
            ]);
            for view in &v.views {
                println!(
                    "    {} {} {} {} {}",
                    output::pad(view.id.as_deref().unwrap_or("-"), output::ID_W),
                    output::pad(
                        &output::truncate(view.name.as_deref().unwrap_or("(unnamed)"), 24),
                        24
                    ),
                    output::pad(&opt_count(view.demonstration_count), 9),
                    output::pad(&output::bool_indicator(Some(view.frozen_at.is_some())), 8),
                    output::truncate(
                        selection_summary(view.selection.as_ref())
                            .as_deref()
                            .unwrap_or("-"),
                        28
                    )
                );
            }
        }
        Ok(_) => {}
        // Not fatal: the detail view above is the answer the user asked for.
        Err(e) => {
            println!();
            output::info(&format!("Fork list unavailable: {e}"));
        }
    }

    println!();
    Ok(())
}

fn render_breakdown(b: &QualityBreakdown) {
    output::subsection("Quality breakdown");
    if let Some(v) = b.demonstration_count {
        output::meter("Episode count", v, 40.0, "orange");
    }
    if let Some(v) = b.duration {
        output::meter("Duration", v, 30.0, "orange");
    }
    // The server key is `diversity`, but what it scores is sensor coverage
    // (cameras present) plus file integrity.
    if let Some(v) = b.diversity {
        output::meter("Coverage", v, 20.0, "orange");
    }
    if let Some(v) = b.format_compliance {
        output::meter("Format", v, 10.0, "orange");
    }
    if let Some(v) = b.total {
        output::meter(
            "Total",
            v,
            100.0,
            if v >= 70.0 {
                "green"
            } else if v >= 40.0 {
                "yellow"
            } else {
                "red"
            },
        );
    }
}

fn render_validation(v: &ValidationSummary) {
    output::subsection("Validation");
    match v.valid {
        Some(true) => output::field_colored("Result", "valid".green().bold()),
        Some(false) => output::field_colored("Result", "INVALID".red().bold()),
        None => output::field_colored("Result", "not validated".bright_black()),
    }
    if let Some(ts) = &v.validated_at {
        output::field("Validated", ts);
    }
    if let Some(ver) = &v.lerobot_version {
        output::field("Format", ver);
    }
    if let Some(n) = v.file_count {
        output::field("Files opened", &thousands(n));
    }
    if let Some(keys) = &v.image_keys {
        output::field(
            "Cameras",
            &if keys.is_empty() {
                "none — a state-only dataset cannot train a VLA".to_string()
            } else {
                keys.join(", ")
            },
        );
    }
    for e in v.errors.iter().flatten() {
        output::field_colored("Error", output::truncate(&issue_text(e), 84).red());
    }
    for w in v.warnings.iter().flatten() {
        output::field_colored("Warning", output::truncate(&issue_text(w), 84).yellow());
    }
}

/// The dataset a direct upload ended as: what `show` leads with, plus the
/// validation report. Takes the raw `GET /datasets/{id}` body so the upload
/// module does not need this file's structs.
pub(super) fn render_result(body: &Value) -> RmsResult<()> {
    let env: DatasetEnvelope = serde_json::from_value(body.clone())?;
    let Some(d) = env.dataset else {
        return Err(RmsError::Other("the server returned no dataset".into()));
    };
    output::field("ID", &d.id);
    output::field("Name", d.label());
    output::field(
        "Status",
        &dataset_status(d.status.as_deref().unwrap_or("unknown")),
    );
    output::field(
        "Robot type",
        &named(d.robot_type.as_ref(), d.robot_type_id.as_deref()),
    );
    output::field("Episodes", &opt_count(d.demonstration_count));
    output::field("Frames", &opt_count(d.total_frames));
    if let Some(secs) = d.total_duration {
        output::field("Duration", &hms(secs));
    }
    if let Some(v) = &d.lerobot_version {
        output::field("LeRobot", v);
    }
    if let Some(q) = d.quality_score {
        output::field("Quality", &format!("{q:.0} / 100"));
    }
    if let Some(mode) = &d.import_mode {
        output::field("Import mode", mode);
    }
    if let Some(p) = &d.storage_path {
        output::field("Storage", &output::truncate(p, 88));
    }
    if let Some(err) = &d.import_error {
        output::field_colored("Import error", output::truncate(err, 84).red());
    }
    match &d.validation {
        Some(v) => render_validation(v),
        None => output::info("No validation report on the dataset."),
    }
    Ok(())
}

// ── episodes ────────────────────────────────────────────────────────────

/// Rows shown when the caller names no `--limit`: the route has no paging of
/// its own and a 400-episode dataset is not a useful wall of output.
const EPISODE_PAGE: usize = 50;

async fn episodes(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/datasets/{id}/episodes");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp: EpisodesResponse = client.get(&path).await?;

    output::section("◈", &format!("Episodes: {}", id.bold()));

    if resp.episodes.is_empty() {
        output::info("No episodes — nothing has read this dataset's metadata yet.");
        println!();
        return Ok(());
    }

    let total = resp.episodes.len();
    let flagged = resp
        .episodes
        .iter()
        .filter(|e| e.flagged == Some(true))
        .count();
    let frames: f64 = resp.episodes.iter().filter_map(|e| e.frame_count).sum();
    let duration: f64 = resp
        .episodes
        .iter()
        .filter_map(|e| e.duration_seconds)
        .sum();

    output::stat_line("│", "Episodes", &thousands(total as f64), "orange");
    output::stat_line("│", "Frames", &thousands(frames), "orange");
    output::stat_line("│", "Duration", &hms(duration), "orange");
    output::stat_line(
        if flagged == 0 { "│" } else { "■" },
        "Flagged",
        &flagged.to_string(),
        if flagged == 0 { "dim" } else { "yellow" },
    );
    println!();

    // The route answers with every episode in one body, so the slice is done
    // here rather than in the query string.
    let limit = ctx.limit.map(|l| l as usize).unwrap_or(EPISODE_PAGE).max(1);
    let page = ctx.page.unwrap_or(1).max(1) as usize;
    let pages = total.div_ceil(limit);
    let rows: Vec<&Episode> = resp
        .episodes
        .iter()
        .skip((page - 1) * limit)
        .take(limit)
        .collect();

    if rows.is_empty() {
        output::warn(&format!(
            "Page {page} is past the end — this dataset has {pages} page(s) of {limit}."
        ));
        println!();
        return Ok(());
    }

    let windows = resp.episodes.iter().any(|e| e.video_windows.is_some());
    let mut columns: Vec<(&str, usize)> = vec![
        ("INDEX", 7),
        ("FRAMES", 9),
        ("DURATION", 10),
        ("FLAGGED", 8),
    ];
    if windows {
        columns.push(("VIDEO WINDOWS", 34));
    }
    output::table_header(&columns);

    for e in &rows {
        let mut line = format!(
            "    {} {} {} {}",
            output::pad(
                &e.index.map(|i| i.to_string()).unwrap_or_else(|| "-".into()),
                7
            ),
            output::pad(&opt_count(e.frame_count), 9),
            output::pad(
                &e.duration_seconds
                    .map(|d| format!("{d:.2}s"))
                    .unwrap_or_else(|| "-".into()),
                10
            ),
            output::pad(&output::bool_indicator(e.flagged), 8),
        );
        if windows {
            // Present only for LeRobot v3.0 chunked video: the camera keys are
            // what a viewer needs, the from/to offsets only matter to a player.
            let cams = e
                .video_windows
                .as_ref()
                .and_then(|w| w.as_object())
                .map(|o| o.keys().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_else(|| "-".into());
            line.push(' ');
            line.push_str(&output::truncate(&cams, 34));
        }
        println!("{line}");
    }

    output::count_paged(
        "episode(s)",
        rows.len(),
        Some(total as u32),
        Some(pages as u32),
    );
    if pages > 1 {
        output::info(&format!(
            "page {page} of {pages} — sliced client-side, the server returns every episode in one response."
        ));
    }
    println!();
    Ok(())
}

// ── quality ─────────────────────────────────────────────────────────────

async fn quality(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/datasets/{id}/quality");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp: QualityResponse = client.get(&path).await?;

    output::section("◈", &format!("Quality: {}", id.bold()));

    match &resp.report {
        Some(r) => {
            if let Some(name) = &r.dataset_name {
                output::field("Dataset", name);
            }
            if let Some(ts) = &r.generated_at {
                output::field("Generated", ts);
            }
            if let Some(status) = &r.validation_status {
                output::field("Validation", &output::status_indicator(status));
            }
            match r.overall_score {
                Some(s) => output::field("Score", &format!("{s:.0} / 100")),
                None => output::field_colored("Score", "not scored".bright_black()),
            }
            output::field("Trajectories", &opt_count(r.trajectory_count));
            output::field(
                "Flagged",
                &format!(
                    "{} ({} anomalous)",
                    opt_count(r.flagged_trajectory_count),
                    opt_count(r.anomalous_trajectory_count)
                ),
            );
            if let Some(pct) = r.clean_trajectory_percentage {
                output::meter(
                    "Clean",
                    pct,
                    100.0,
                    if pct >= 90.0 { "green" } else { "yellow" },
                );
            }
            if let Some(b) = &r.score_breakdown {
                render_breakdown(b);
            }

            let flagged = r.flagged_summary.as_deref().unwrap_or(&[]);
            if !flagged.is_empty() {
                output::subsection("Flagged trajectories");
                render_flagged_rows(flagged);
            }
        }
        None => {
            output::info(
                "No quality report — nothing has scored this dataset. Run: rmsctl datasets validate <id>",
            );
            if resp.has_quality_report == Some(true) {
                output::warn("The server said hasQualityReport=true but sent no report object.");
            }
        }
    }

    match &resp.validation {
        Some(v) => render_validation(v),
        None => output::info(
            "No structural validation on record — no file of this dataset has been opened.",
        ),
    }

    println!();
    Ok(())
}

// ── stats ───────────────────────────────────────────────────────────────

async fn stats(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/datasets/{id}/stats");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp: StatsResponse = client.get(&path).await?;

    output::section("◈", &format!("Statistics: {}", id.bold()));

    let stats = match &resp.stats {
        Some(s) if !s.is_empty() => s,
        // `hasStats: false` is the normal state of a locally registered
        // dataset: nothing has computed stats.json for it.
        _ => {
            output::info("No statistics stored for this dataset (stats.json was never computed).");
            println!();
            return Ok(());
        }
    };

    if let Some(ts) = &resp.computed_at {
        output::field("Computed", ts);
        println!();
    }

    output::table_header(&[
        ("FEATURE", 34),
        ("DIM", 5),
        ("COUNT", 12),
        ("MIN", 14),
        ("MAX", 14),
        ("MEAN", 14),
        ("STD", 14),
    ]);

    for (feature, v) in stats.iter() {
        let get = |key: &str| {
            v.get(key)
                .and_then(first_num)
                .map(fmt_num)
                .unwrap_or_else(|| "-".into())
        };
        println!(
            "    {} {} {} {} {} {} {}",
            output::pad(&output::truncate(feature, 34), 34),
            output::pad(
                &width_of(v.get("mean").or_else(|| v.get("min"))).to_string(),
                5
            ),
            output::pad(&get("count"), 12),
            output::pad(&get("min"), 14),
            output::pad(&get("max"), 14),
            output::pad(&get("mean"), 14),
            output::pad(&get("std"), 14),
        );
    }

    output::count_paged("feature(s)", stats.len(), Some(stats.len() as u32), None);
    output::info("MIN/MAX/MEAN/STD show the first component; DIM is the feature's width.");
    println!();
    Ok(())
}

// ── validate ────────────────────────────────────────────────────────────

async fn validate(client: &ApiClient, ctx: &Ctx, id: &str, advanced: bool) -> RmsResult<()> {
    if advanced {
        return validate_advanced(client, ctx, id).await;
    }

    if !output::confirm(
        &format!(
            "This will start a structural validation of dataset {id}, opening every file its info.json declares"
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let resp = client
        .post_value(&format!("/datasets/{id}/validate"), &serde_json::json!({}))
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    output::success(&format!("Validation accepted for {id}"));
    // `queued` = a NATS worker picked it up; `started` = the server is running
    // it off the request. The two differ in where a failure shows up.
    if let Some(state) = resp.get("state").and_then(|s| s.as_str()) {
        output::field(
            "State",
            &match state {
                "queued" => "queued — a worker will run it".to_string(),
                "started" => "started — running on the server process".to_string(),
                other => other.to_string(),
            },
        );
    }
    if let Some(url) = resp.get("progressUrl").and_then(|u| u.as_str()) {
        output::field("Progress", url);
    }
    output::info(&format!(
        "Poll the outcome with: rmsctl datasets quality {id}"
    ));
    println!();
    Ok(())
}

/// Always answers 501 on this server: the metrics exist in `DataQualityService`
/// but no job runs them over a dataset, and the route refuses rather than
/// queueing work nothing will ever finish.
async fn validate_advanced(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    output::warn(
        "Advanced (per-trajectory) validation is not implemented server-side — this route answers 501.",
    );

    if !output::confirm(
        &format!(
            "This will ask the server to start advanced trajectory validation for dataset {id}"
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    match client
        .post_value(
            &format!("/datasets/{id}/validate-advanced"),
            &serde_json::json!({}),
        )
        .await
    {
        Ok(resp) => {
            if ctx.json {
                return output::json_out(&resp);
            }
            output::success(&format!("Advanced validation accepted for {id}"));
            if let Some(state) = resp.get("state").and_then(|s| s.as_str()) {
                output::field("State", state);
            }
            println!();
            Ok(())
        }
        // Re-word the 501 so the exit code still fails but the message names the
        // command that does work.
        Err(RmsError::Api {
            status: 501,
            message,
        }) => Err(RmsError::Api {
            status: 501,
            message: format!(
                "{message} — structural validation does run: rmsctl datasets validate {id} (without --advanced)"
            ),
        }),
        Err(e) => Err(e),
    }
}

// ── push / push-status ──────────────────────────────────────────────────

/// Starts a Hugging Face upload on the server.
///
/// The server spawns a hard-coded interpreter path
/// (`/home/mindcube/miniconda3/envs/lerobot312/bin/python`) to run
/// `scripts/hf_push_dataset.py`, so on a development box the job is accepted
/// with 202 and then fails; `push-status` is where that shows up.
async fn push(
    client: &ApiClient,
    ctx: &Ctx,
    id: &str,
    hf_token: &str,
    repo_id: &str,
) -> RmsResult<()> {
    let token = hf_token.trim();
    let repo = repo_id.trim();
    if token.is_empty() {
        return Err(RmsError::InvalidInput(
            "--hf-token is required: the server rejects a push without a Hugging Face token".into(),
        ));
    }
    if repo.is_empty() {
        return Err(RmsError::InvalidInput(
            "--repo-id is required, as org/dataset-name".into(),
        ));
    }

    // The server refuses anything but `ready` with a 400; read the row first so
    // the refusal names the state and the gate can name the dataset.
    let env: DatasetEnvelope = client.get(&format!("/datasets/{id}")).await?;
    let d = env
        .dataset
        .ok_or_else(|| RmsError::Other(format!("the server returned no dataset for {id}")))?;
    let status = d.status.as_deref().unwrap_or("unknown");
    if status != "ready" {
        return Err(RmsError::InvalidInput(format!(
            "dataset {} is '{status}' — only a 'ready' dataset can be pushed",
            d.label()
        )));
    }

    output::warn(
        "The server pushes by spawning a hard-coded python path; on a dev box the job is accepted and then fails.",
    );

    if !output::confirm(
        &format!(
            "This will upload dataset {} ({} episodes, {} frames) to the Hugging Face Hub as {repo}",
            d.label(),
            opt_count(d.demonstration_count),
            opt_count(d.total_frames)
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    // Body keys are `token` and `repoId`; the token is never echoed back.
    let body = serde_json::json!({ "token": token, "repoId": repo });
    let resp = client
        .post_value(&format!("/datasets/{id}/push-to-hub"), &body)
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    output::success(&format!("Push started for {} -> {repo}", d.label()));
    if let Some(msg) = resp.get("message").and_then(|m| m.as_str()) {
        output::info(msg);
    }
    output::info(&format!("Follow it with: rmsctl datasets push-status {id}"));
    println!();
    Ok(())
}

async fn push_status(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/datasets/{id}/push-status");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp = client.get_value(&path).await?;

    output::section("◈", &format!("Hub push: {}", id.bold()));

    let status = resp
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("unknown");

    // `none` means no job in memory AND no huggingFaceRepoId on the row — the
    // job table is in-process, so a server restart also lands here.
    if status == "none" {
        output::info("No push on record for this dataset.");
        println!();
        return Ok(());
    }

    output::field("Status", &output::status_indicator(status));
    if let Some(p) = resp.get("progress").and_then(|p| p.as_str()) {
        output::field("Progress", p);
    }
    if let Some(url) = resp.get("url").and_then(|u| u.as_str()) {
        output::field("URL", url);
    }
    if let Some(ts) = resp.get("startedAt").and_then(|t| t.as_str()) {
        output::field("Started", ts);
    }
    if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
        output::field_colored("Error", output::truncate(err, 84).red());
    }
    println!();
    Ok(())
}

// ── flagged / flag ──────────────────────────────────────────────────────

async fn flagged(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/datasets/{id}/flagged");
    let query = ctx.paging_limit();

    if ctx.json {
        return output::json_out(&client.get_value_query(&path, &query).await?);
    }

    let resp: FlaggedResponse = client.get_query(&path, &query).await?;

    output::section("◈", &format!("Flagged episodes: {}", id.bold()));

    if resp.flagged.is_empty() {
        output::info("No flagged episodes.");
        println!();
        return Ok(());
    }

    render_flagged_rows(&resp.flagged);

    // This envelope carries page/limit/total flat rather than in a `pagination`
    // object, so the page count is derived here.
    output::count_paged(
        "flagged episode(s)",
        resp.flagged.len(),
        resp.total,
        total_pages_for(resp.total, resp.limit),
    );
    output::info(
        "SCORE and ANOMALIES are placeholders — nothing computes them (validate-advanced answers 501).",
    );
    println!();
    Ok(())
}

fn render_flagged_rows(rows: &[FlaggedEpisode]) {
    output::table_header(&[
        ("EPISODE", 8),
        ("REASON", 34),
        ("REVIEWED", 9),
        ("DECISION", 10),
        ("SCORE", 6),
        ("ANOMALIES", 20),
    ]);
    for f in rows {
        println!(
            "    {} {} {} {} {} {}",
            output::pad(
                &f.trajectory_index
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "-".into()),
                8
            ),
            output::pad(
                &output::truncate(f.flag_reason.as_deref().unwrap_or("-"), 34),
                34
            ),
            output::pad(&output::bool_indicator(f.reviewed), 9),
            output::pad(f.review_decision.as_deref().unwrap_or("-"), 10),
            output::pad(
                &f.quality_score
                    .map(|s| format!("{s:.0}"))
                    .unwrap_or_else(|| "-".into()),
                6
            ),
            output::truncate(
                &f.anomaly_types
                    .as_ref()
                    .filter(|a| !a.is_empty())
                    .map(|a| a.join(", "))
                    .unwrap_or_else(|| "-".into()),
                20
            )
        );
    }
}

async fn flag(
    client: &ApiClient,
    ctx: &Ctx,
    id: &str,
    index: u32,
    reason: Option<&str>,
) -> RmsResult<()> {
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());

    if !output::confirm(
        &format!(
            "This will flag episode {index} of dataset {id} for review{}",
            match reason {
                Some(r) => format!(" — reason: {r}"),
                None => String::new(),
            }
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    // `flagged` must be a boolean or the server answers 400. There is no
    // subcommand for clearing a flag, so this only ever sets it.
    let mut body = serde_json::json!({ "flagged": true });
    if let Some(r) = reason {
        body["reason"] = Value::String(r.to_string());
    }

    let resp = client
        .patch_value(&format!("/datasets/{id}/episodes/{index}/flag"), &body)
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    output::success(&format!("Episode {index} flagged"));
    let flag = resp.get("flag");
    if let Some(r) = flag.and_then(|f| f.get("reason")).and_then(|r| r.as_str()) {
        output::field("Reason", r);
    }
    if let Some(ts) = flag
        .and_then(|f| f.get("updatedAt"))
        .and_then(|t| t.as_str())
    {
        output::field("Recorded", ts);
    }
    output::info(&format!(
        "Review the list with: rmsctl datasets flagged {id}"
    ));
    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: [Option<&str>; 4] = [Some("materialized"); 4];

    #[test]
    fn the_full_table_stays_without_a_terminal_and_on_a_wide_one() {
        let full = ListLayout {
            id_w: output::ID_W,
            kind_column: true,
            name_min: 26,
        };
        assert_eq!(list_layout(None, KINDS.to_vec()), full);
        assert_eq!(list_layout(Some(200), KINDS.to_vec()), full);
        assert!(full.notes(Some("materialized")).is_empty());
    }

    #[test]
    fn a_narrow_terminal_gets_short_ids_and_no_kind_column_when_all_kinds_match() {
        for term in [80, 120] {
            let l = list_layout(Some(term), KINDS.to_vec());
            assert_eq!(l.id_w, output::SHORT_ID_W);
            assert!(!l.kind_column);
            // The row fits: the other columns, one spare, and NAME's minimum.
            let fixed = output::fixed_width(&[l.id_w, 12, 9, 11, 7]);
            assert!(fixed + 1 + l.name_min <= term, "{term}: {fixed}");
            let notes = l.notes(Some("materialized"));
            assert_eq!(notes.len(), 2);
            assert!(notes[0].contains("IDs cut to 8 characters"), "{notes:?}");
            assert_eq!(notes[1], "KIND left out: every row is materialized.");
            // "  [--] " plus the note fits the narrowest terminal checked.
            assert!(
                notes.iter().all(|n| 7 + n.chars().count() <= 80),
                "{notes:?}"
            );
        }
    }

    #[test]
    fn a_narrow_terminal_keeps_kind_when_views_are_listed() {
        let l = list_layout(Some(120), vec![Some("materialized"), Some("view")]);
        assert_eq!(l.id_w, output::SHORT_ID_W);
        assert!(l.kind_column);
        assert_eq!(l.notes(None).len(), 1);
    }

    fn listed() -> Vec<(String, String)> {
        [
            ("9923225e-9d0a-463f-b072-eeded251142a", "Bottle Sorting"),
            ("9b7c63ca-fc76-4dd1-aed2-2dada9bdcb64", "GR00T reference"),
            (
                "9b7c63ca-1111-4dd1-aed2-2dada9bdcb64",
                "GR00T reference (fork)",
            ),
        ]
        .iter()
        .map(|(i, n)| (i.to_string(), n.to_string()))
        .collect()
    }

    #[test]
    fn a_unique_prefix_resolves_to_the_whole_id() {
        assert_eq!(
            match_prefix(&listed(), "9923225e").unwrap(),
            "9923225e-9d0a-463f-b072-eeded251142a"
        );
        assert_eq!(
            match_prefix(&listed(), "9b7c63ca-fc").unwrap(),
            "9b7c63ca-fc76-4dd1-aed2-2dada9bdcb64"
        );
    }

    #[test]
    fn an_ambiguous_or_unknown_prefix_is_refused_with_the_candidates() {
        let e = match_prefix(&listed(), "9b7c63ca").unwrap_err().to_string();
        assert!(e.contains("is the start of 2 dataset ids"), "{e}");
        assert!(
            e.contains("9b7c63ca-fc76-4dd1-aed2-2dada9bdcb64 (GR00T reference)"),
            "{e}"
        );
        assert!(e.contains("(GR00T reference (fork))"), "{e}");
        let e = match_prefix(&listed(), "deadbeef").unwrap_err().to_string();
        assert!(e.contains("no dataset id starts with 'deadbeef'"), "{e}");
    }
}
