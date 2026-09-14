use crate::cli::TrainingCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::models::Pagination;
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub async fn run(cmd: &TrainingCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        TrainingCommand::List => list(client, ctx).await,
        TrainingCommand::Show { id } => show(client, ctx, id).await,
        TrainingCommand::Active => active(client, ctx).await,
        TrainingCommand::Queue => queue(client, ctx).await,
        TrainingCommand::Workers => workers(client, ctx).await,
        TrainingCommand::Start { config } => start(client, ctx, config).await,
        TrainingCommand::Cancel { id } => cancel(client, ctx, id).await,
        TrainingCommand::Retry { id } => retry(client, ctx, id).await,
        TrainingCommand::Export { id } => export(client, ctx, id).await,
        TrainingCommand::Estimate { id } => estimate(client, ctx, id).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────

/// One member of a job's dataset mixture.
///
/// `job.datasetId` is only member 0, so a two-dataset run read through
/// `datasetId` alone looks like a single-dataset run with no weights.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatasetRef {
    #[serde(default)]
    dataset_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    weight: Option<f64>,
    #[serde(default)]
    position: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GpuRequirements {
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    memory: Option<f64>,
    #[serde(default, rename = "type")]
    gpu_type: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrainingJob {
    id: String,
    /// Discriminator: supervised | sim_rl | reward_model | annotate. It is what
    /// explains the nulls — a sim_rl job has no dataset and no base model, an
    /// annotate job has no fine-tune method.
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    dataset_id: Option<String>,
    #[serde(default)]
    base_model: Option<String>,
    #[serde(default)]
    fine_tune_method: Option<String>,
    #[serde(default)]
    scene_id: Option<String>,
    #[serde(default)]
    twin_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    /// Whole percent, 0-100 (the orchestrator rounds `step / totalSteps * 100`).
    #[serde(default)]
    progress: Option<f64>,
    #[serde(default)]
    current_epoch: Option<u32>,
    #[serde(default)]
    total_epochs: Option<u32>,
    #[serde(default)]
    hyperparameters: Option<Value>,
    #[serde(default)]
    gpu_requirements: Option<GpuRequirements>,
    /// Loss curves live here, not at job level: `final_loss`, `training_loss[]`,
    /// `learning_rate[]`, plus `mean_reward`/`success_rate` for sim_rl.
    #[serde(default)]
    metrics: Option<Value>,
    #[serde(default)]
    datasets: Option<Vec<DatasetRef>>,
    /// The server spells this `errorMessage`; there is no `error` key on a job.
    #[serde(default, rename = "errorMessage")]
    error: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    init_from_model_version_id: Option<String>,
    #[serde(default)]
    init_from_checkpoint_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrainingJobsResponse {
    #[serde(default)]
    jobs: Vec<TrainingJob>,
    #[serde(default)]
    pagination: Option<Pagination>,
}

/// Live queue progress. Null whenever the server has no NATS connection, which
/// is the normal case on a dev server — so every read of it is optional.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobProgress {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    progress: Option<f64>,
    #[serde(default)]
    current_epoch: Option<u32>,
    #[serde(default)]
    total_epochs: Option<u32>,
    #[serde(default)]
    metrics: Option<Value>,
    #[serde(default)]
    eta: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrainingJobDetailResponse {
    job: TrainingJob,
    /// Mirrored from the job by the route so a caller holding either one has it.
    #[serde(default)]
    datasets: Option<Vec<DatasetRef>>,
    #[serde(default)]
    progress: Option<JobProgress>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrainingActiveResponse {
    #[serde(default)]
    jobs: Vec<TrainingJob>,
    #[serde(default)]
    count: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamInfo {
    #[serde(default)]
    messages: Option<u64>,
    #[serde(default)]
    bytes: Option<u64>,
    #[serde(default)]
    first_seq: Option<u64>,
    #[serde(default)]
    last_seq: Option<u64>,
    #[serde(default)]
    consumer_count: Option<u32>,
}

/// `/training/queue/stats`. The running-job count is called `running` on the
/// wire — there is no `active` key — and `queued` (handed to the queue, not yet
/// claimed) is a distinct state from `pending`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueueStats {
    #[serde(default)]
    pending: Option<u32>,
    #[serde(default)]
    queued: Option<u32>,
    #[serde(default)]
    running: Option<u32>,
    #[serde(default)]
    completed: Option<u32>,
    #[serde(default, rename = "completed_24h")]
    completed_24h: Option<u32>,
    #[serde(default)]
    failed: Option<u32>,
    #[serde(default)]
    stream_info: Option<StreamInfo>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkerCurrentJob {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    base_model: Option<String>,
    #[serde(default)]
    dataset_id: Option<String>,
    #[serde(default)]
    age_seconds: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Worker {
    #[serde(default)]
    worker_id: Option<String>,
    #[serde(default)]
    device: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    current_job: Option<WorkerCurrentJob>,
    #[serde(default)]
    gpu_util: Option<f64>,
    #[serde(default)]
    memory_util: Option<f64>,
    #[serde(default)]
    last_heartbeat_at: Option<String>,
    #[serde(default)]
    last_heartbeat_age_seconds: Option<f64>,
    #[serde(default)]
    first_seen_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkersResponse {
    #[serde(default)]
    workers: Vec<Worker>,
    /// Counted from the database, so these are true even with no worker connected.
    #[serde(default)]
    queued_jobs: Option<u32>,
    #[serde(default)]
    running_jobs: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DurationEstimate {
    #[serde(default)]
    estimated_minutes: Option<f64>,
    #[serde(default)]
    estimated_steps: Option<f64>,
    #[serde(default)]
    steps_per_second: Option<f64>,
    #[serde(default)]
    confidence: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobMutationResponse {
    #[serde(default)]
    job: Option<TrainingJob>,
    #[serde(default)]
    message: Option<String>,
}

// ── Export manifest ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestJob {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    base_model: Option<String>,
    #[serde(default)]
    fine_tune_method: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    init_from: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestDataset {
    #[serde(default)]
    dataset_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    weight: Option<f64>,
    #[serde(default)]
    normalized_weight: Option<f64>,
    #[serde(default)]
    lerobot_version: Option<String>,
    #[serde(default)]
    robot_type: Option<String>,
    #[serde(default)]
    total_episodes: Option<u64>,
    #[serde(default)]
    total_frames: Option<u64>,
    /// False when the run reads a local path no other cluster can open.
    #[serde(default)]
    portable: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatAxis {
    #[serde(default)]
    axis: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Compatibility {
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    headline: Option<String>,
    #[serde(default)]
    recommendation: Option<String>,
    #[serde(default)]
    axes: Option<Vec<CompatAxis>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestRuntime {
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    entrypoint: Option<String>,
    #[serde(default)]
    command: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestCompliance {
    #[serde(default)]
    dataset_licenses: Option<Vec<String>>,
    #[serde(default)]
    residency: Option<String>,
    #[serde(default)]
    notes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunManifest {
    #[serde(default)]
    schema_version: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    source_server: Option<String>,
    #[serde(default)]
    job: Option<ManifestJob>,
    #[serde(default)]
    datasets: Option<Vec<ManifestDataset>>,
    #[serde(default)]
    compatibility: Option<Compatibility>,
    #[serde(default)]
    hyperparameters: Option<Value>,
    #[serde(default)]
    gpu: Option<GpuRequirements>,
    #[serde(default)]
    runtime: Option<ManifestRuntime>,
    #[serde(default)]
    compliance: Option<ManifestCompliance>,
    #[serde(default)]
    warnings: Option<Vec<String>>,
}

// ── Formatting helpers ──────────────────────────────────────────────────

/// Print integral floats without a decimal tail: weights and epoch counts
/// arrive as JSON numbers but read as counts.
fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn fmt_value(v: &Value) -> String {
    match v {
        Value::Null => "-".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.as_f64().map(fmt_num).unwrap_or_else(|| n.to_string()),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn human_minutes(m: f64) -> String {
    let total = m.round().max(0.0) as u64;
    let (d, h, mi) = (total / 1440, (total % 1440) / 60, total % 60);
    match (d, h) {
        (0, 0) => format!("{mi}m"),
        (0, _) => format!("{h}h {mi}m"),
        _ => format!("{d}d {h}h {mi}m"),
    }
}

fn human_secs(s: f64) -> String {
    let total = s.round().max(0.0) as u64;
    match total {
        0..=99 => format!("{total}s"),
        100..=5999 => format!("{}m", total / 60),
        _ => format!("{}h", total / 3600),
    }
}

const WRAP: usize = 84;

fn wrap_lines(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Manifest prose (headlines, compliance notes, warnings) runs past 200
/// characters, which `output::field` would leave on one unreadable line.
fn prose(text: &str) {
    for line in wrap_lines(text, WRAP) {
        println!("      {}", line.bright_black());
    }
}

/// As `output::warn`, but for a paragraph rather than a sentence.
fn warn_prose(text: &str) {
    let mut lines = wrap_lines(text, WRAP).into_iter();
    if let Some(first) = lines.next() {
        println!("  {} {}", "[!]".yellow().bold(), first);
        for line in lines {
            println!("      {line}");
        }
    }
}

/// Key/value rows whose keys outgrow `output::field`'s 14-column label, such as
/// `gradient_accumulation_steps`.
fn wide_field(label: &str, value: &str) {
    println!(
        "    {} {} {}",
        "│".bright_black(),
        format!("{label:<28}").bright_black(),
        value
    );
}

fn render_map(v: Option<&Value>) {
    let Some(map) = v.and_then(|h| h.as_object()) else {
        return;
    };
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for k in keys {
        wide_field(k, &fmt_value(&map[k]));
    }
}

/// Mixture verdicts are not lifecycle statuses, so `status_indicator` does not
/// know them: `identical`/`compatible` are fine, `multi_embodiment` costs the
/// operator a decision, `incompatible` refuses the run.
fn verdict_cell(verdict: &str) -> ColoredString {
    match verdict {
        "identical" | "compatible" | "match" => verdict.green().bold(),
        "multi_embodiment" | "differs" => verdict.yellow().bold(),
        "incompatible" | "blocking" => verdict.red().bold(),
        _ => verdict.normal(),
    }
}

fn get_num(v: Option<&Value>, key: &str) -> Option<f64> {
    v?.get(key)?.as_f64()
}

/// Last numeric entry of a metrics series, skipping trailing nulls.
fn last_num(v: Option<&Value>, key: &str) -> Option<f64> {
    v?.get(key)?
        .as_array()?
        .iter()
        .rev()
        .find_map(serde_json::Value::as_f64)
}

fn non_empty_object(v: Option<&Value>) -> Option<&Value> {
    v.filter(|m| m.as_object().is_some_and(|o| !o.is_empty()))
}

/// Loss: the run's `final_loss`, else the newest point on the training curve.
fn loss_of(metrics: Option<&Value>) -> Option<f64> {
    get_num(metrics, "final_loss").or_else(|| last_num(metrics, "training_loss"))
}

/// Learning rate: the configured hyperparameter, else the newest logged value.
fn lr_of(hyper: Option<&Value>, metrics: Option<&Value>) -> Option<f64> {
    get_num(hyper, "learning_rate").or_else(|| last_num(metrics, "learning_rate"))
}

/// A null column on a non-supervised job is the contract, not missing data:
/// sim_rl jobs carry a scene instead of a dataset and base model, and
/// annotate/reward_model jobs have no fine-tune method. Render those as a
/// dimmed `n/a` so they never read as a gap in the record.
fn absent(kind: Option<&str>, field: &str) -> String {
    let explained = matches!(
        (kind.unwrap_or(""), field),
        ("sim_rl", "dataset" | "model" | "method") | ("annotate" | "reward_model", "method")
    );
    if explained {
        "n/a".bright_black().to_string()
    } else {
        "-".bright_black().to_string()
    }
}

fn progress_color(v: f64) -> &'static str {
    if v >= 100.0 { "green" } else { "orange" }
}

// ── list ────────────────────────────────────────────────────────────────

async fn list(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let query = ctx.paging();

    if ctx.json {
        return output::json_out(&client.get_value_query("/training/jobs", &query).await?);
    }

    let resp: TrainingJobsResponse = client.get_query("/training/jobs", &query).await?;

    output::section("◈", "Training Jobs");

    if resp.jobs.is_empty() {
        output::info("No training jobs found.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("ID", output::ID_W),
        ("KIND", 11),
        ("MODEL", 14),
        ("METHOD", 10),
        ("MIX", 4),
        ("STATUS", 12),
        ("PROGRESS", 8),
    ]);

    for j in &resp.jobs {
        let kind = j.kind.as_deref().unwrap_or("-");
        let model = match j.base_model.as_deref() {
            Some(m) => output::truncate(m, 14),
            None => absent(j.kind.as_deref(), "model"),
        };
        let method = match j.fine_tune_method.as_deref() {
            Some(m) => output::truncate(m, 10),
            None => absent(j.kind.as_deref(), "method"),
        };
        // Mixture size, so a two-dataset run is not mistaken for a single one.
        let mix = match j.datasets.as_ref().map(Vec::len).unwrap_or(0) {
            0 => "-".bright_black().to_string(),
            1 => "1".bright_black().to_string(),
            n => format!("{n}").truecolor(255, 103, 0).to_string(),
        };
        let status = j.status.as_deref().unwrap_or("unknown");
        let progress = j
            .progress
            .map(|v| format!("{v:.0}%"))
            .unwrap_or_else(|| "-".into());

        println!(
            "    {} {} {} {} {} {} {}",
            output::pad(&j.id, output::ID_W),
            output::pad(kind, 11),
            output::pad(&model, 14),
            output::pad(&method, 10),
            output::pad(&mix, 4),
            output::pad(&output::status_indicator(status), 12),
            progress
        );
    }

    let pag = resp.pagination.as_ref();
    output::count_paged(
        "job(s)",
        resp.jobs.len(),
        pag.and_then(|p| p.total()),
        pag.and_then(|p| p.total_pages()),
    );
    println!();
    Ok(())
}

// ── show ────────────────────────────────────────────────────────────────

async fn show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/training/jobs/{id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp: TrainingJobDetailResponse = client.get(&path).await?;
    let j = &resp.job;
    let kind = j.kind.as_deref();

    output::section("◈", &format!("Training Job: {}", j.id.bold()));

    output::field("ID", &j.id);
    output::field("Kind", kind.unwrap_or("-"));
    if let Some(v) = &j.status {
        output::field("Status", &output::status_indicator(v));
    }
    match &j.base_model {
        Some(v) => output::field("Base Model", v),
        None => output::field("Base Model", &absent(kind, "model")),
    }
    match &j.fine_tune_method {
        Some(v) => output::field("Method", v),
        None => output::field("Method", &absent(kind, "method")),
    }
    // sim_rl trains in a simulated scene, so it has no dataset at all.
    if let Some(v) = &j.scene_id {
        output::field("Scene", v);
    }
    if let Some(v) = &j.twin_id {
        output::field("Twin", v);
    }
    if let Some(v) = &j.init_from_model_version_id {
        output::field("Init from", &format!("model version {v}"));
    }
    if let Some(v) = &j.init_from_checkpoint_id {
        output::field("Init from", &format!("checkpoint {v}"));
    }

    render_mixture(
        resp.datasets.as_deref().or(j.datasets.as_deref()),
        j.dataset_id.as_deref(),
        kind,
    );

    // The live progress object outranks the stored job row while a run is in
    // flight; it is null whenever the queue is not connected.
    let live = resp.progress.as_ref();
    let metrics = non_empty_object(live.and_then(|p| p.metrics.as_ref()))
        .or_else(|| non_empty_object(j.metrics.as_ref()));
    let hyper = non_empty_object(j.hyperparameters.as_ref());

    output::subsection("Progress");
    let pct = live.and_then(|p| p.progress).or(j.progress);
    if let Some(v) = pct {
        output::meter("Completion", v, 100.0, progress_color(v));
    }
    let cur = live.and_then(|p| p.current_epoch).or(j.current_epoch);
    let total = live.and_then(|p| p.total_epochs).or(j.total_epochs);
    match (cur, total) {
        (Some(c), Some(t)) => output::field("Epoch", &format!("{c} / {t}")),
        (Some(c), None) => output::field("Epoch", &c.to_string()),
        (None, Some(t)) => output::field("Epochs", &t.to_string()),
        (None, None) => {}
    }
    if let Some(v) = loss_of(metrics) {
        output::field("Loss", &format!("{v:.6}"));
    }
    if let Some(v) = lr_of(hyper, metrics) {
        output::field("Learning Rate", &format!("{v:.2e}"));
    }
    if let Some(v) = get_num(metrics, "best_epoch") {
        output::field("Best Epoch", &fmt_num(v));
    }
    // sim_rl reports reward and success rate instead of a loss curve.
    if let Some(v) = get_num(metrics, "mean_reward") {
        output::field("Mean Reward", &format!("{v:.4}"));
    }
    if let Some(v) = get_num(metrics, "success_rate") {
        output::field("Success Rate", &format!("{:.1}%", v * 100.0));
    }
    if let Some(v) = get_num(metrics, "total_timesteps") {
        output::field("Timesteps", &fmt_num(v));
    }
    if let Some(p) = live {
        if let Some(v) = &p.eta {
            output::field("ETA", v);
        }
        if let Some(v) = &p.message {
            output::field("Message", v);
        }
        if let Some(v) = &p.updated_at {
            output::field("Reported", v);
        }
    } else {
        output::field("Live feed", &"not connected".bright_black().to_string());
    }

    if hyper.is_some() {
        output::subsection("Hyperparameters");
        render_map(hyper);
    }

    if let Some(g) = &j.gpu_requirements {
        output::subsection("GPU");
        let count = g.count.unwrap_or(1);
        let mem = g.memory.map(|m| format!("{} GB", fmt_num(m)));
        output::field(
            "Requested",
            &format!(
                "{count} × {}{}",
                mem.as_deref().unwrap_or("unspecified"),
                g.gpu_type
                    .as_deref()
                    .map(|t| format!(" ({t})"))
                    .unwrap_or_default()
            ),
        );
    }

    output::subsection("Timeline");
    if let Some(v) = &j.created_at {
        output::field("Created", v);
    }
    if let Some(v) = &j.started_at {
        output::field("Started", v);
    }
    if let Some(v) = &j.completed_at {
        output::field("Completed", v);
    }
    if let Some(v) = &j.updated_at {
        output::field("Updated", v);
    }

    if let Some(v) = &j.error {
        println!();
        println!("    {} {} {}", "▲".red(), "Error:".red().bold(), v.red());
    }

    println!();
    Ok(())
}

/// One row per mixture member with its sampling weight and share.
fn render_mixture(datasets: Option<&[DatasetRef]>, dataset_id: Option<&str>, kind: Option<&str>) {
    let members = match datasets {
        Some(m) if !m.is_empty() => m,
        _ => {
            // No mixture rows: fall back to the single id, or say why there is none.
            match dataset_id {
                Some(id) => output::field("Dataset", id),
                None => output::field("Dataset", &absent(kind, "dataset")),
            }
            return;
        }
    };

    let sum: f64 = members.iter().filter_map(|d| d.weight).sum();
    output::subsection(&if members.len() == 1 {
        "Dataset".to_string()
    } else {
        format!("Datasets — mixture of {}", members.len())
    });
    output::table_header(&[
        ("ID", output::ID_W),
        ("NAME", 34),
        ("WEIGHT", 7),
        ("SHARE", 6),
    ]);
    for d in members {
        let share = match (d.weight, sum) {
            (Some(w), s) if s > 0.0 => format!("{:.0}%", w / s * 100.0),
            _ => "-".to_string(),
        };
        println!(
            "    {} {} {} {}",
            output::pad(d.dataset_id.as_deref().unwrap_or("-"), output::ID_W),
            output::pad(&output::truncate(d.name.as_deref().unwrap_or("-"), 34), 34),
            output::pad(&d.weight.map(fmt_num).unwrap_or_else(|| "-".into()), 7),
            share
        );
    }
}

// ── active ──────────────────────────────────────────────────────────────

async fn active(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/training/active").await?);
    }

    let resp: TrainingActiveResponse = client.get("/training/active").await?;

    output::section("◈", "Active Training");

    if resp.jobs.is_empty() {
        output::info("No jobs pending, queued or running.");
        println!();
        return Ok(());
    }

    // The route returns pending + queued + running, not only running, so each
    // block prints its status: a queued job must not look like a training one.
    for j in &resp.jobs {
        println!(
            "    {} {}  {}",
            "│".bright_black(),
            j.id.bold(),
            output::status_indicator(j.status.as_deref().unwrap_or("unknown"))
        );
        output::field("Kind", j.kind.as_deref().unwrap_or("-"));
        let model = match j.base_model.as_deref() {
            Some(m) => m.to_string(),
            None => absent(j.kind.as_deref(), "model"),
        };
        let method = match j.fine_tune_method.as_deref() {
            Some(m) => m.to_string(),
            None => absent(j.kind.as_deref(), "method"),
        };
        output::field("Model", &format!("{model} / {method}"));

        if let Some(v) = j.progress {
            output::meter("Progress", v, 100.0, progress_color(v));
        }
        match (j.current_epoch, j.total_epochs) {
            (Some(c), Some(t)) => output::field("Epoch", &format!("{c} / {t}")),
            (None, Some(t)) => output::field("Epochs", &t.to_string()),
            _ => {}
        }
        let metrics = non_empty_object(j.metrics.as_ref());
        if let Some(v) = loss_of(metrics) {
            output::field("Loss", &format!("{v:.6}"));
        }
        println!();
    }

    output::count_paged(
        "job(s) pending, queued or running",
        resp.jobs.len(),
        resp.count,
        None,
    );
    println!();
    Ok(())
}

// ── queue ───────────────────────────────────────────────────────────────

async fn queue(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/training/queue/stats").await?);
    }

    let s: QueueStats = client.get("/training/queue/stats").await?;

    output::section("◈", "Training Queue");

    let rows: [(&str, u32, &str); 6] = [
        ("Pending", s.pending.unwrap_or(0), "yellow"),
        ("Queued", s.queued.unwrap_or(0), "yellow"),
        ("Running", s.running.unwrap_or(0), "orange"),
        ("Completed", s.completed.unwrap_or(0), "green"),
        ("Completed (24h)", s.completed_24h.unwrap_or(0), "green"),
        ("Failed", s.failed.unwrap_or(0), "red"),
    ];
    for (label, value, color) in rows {
        output::stat_line(
            if value > 0 { "■" } else { "│" },
            label,
            &value.to_string(),
            if value > 0 { color } else { "dim" },
        );
    }

    if let Some(st) = &s.stream_info {
        output::subsection("Stream");
        output::field("Messages", &st.messages.unwrap_or(0).to_string());
        output::field("Bytes", &st.bytes.unwrap_or(0).to_string());
        output::field(
            "Sequence",
            &format!(
                "{} → {}",
                st.first_seq.unwrap_or(0),
                st.last_seq.unwrap_or(0)
            ),
        );
        let consumers = st.consumer_count.unwrap_or(0);
        output::field("Consumers", &consumers.to_string());
        // Nothing consuming the stream while work waits means no worker will
        // ever claim it, which the counters alone do not reveal.
        if consumers == 0 && s.pending.unwrap_or(0) + s.queued.unwrap_or(0) > 0 {
            println!();
            output::warn(
                "Work is waiting but the stream has no consumers — no worker is claiming jobs.",
            );
        }
    }

    println!();
    Ok(())
}

// ── workers ─────────────────────────────────────────────────────────────

async fn workers(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/training/workers").await?);
    }

    let resp: WorkersResponse = client.get("/training/workers").await?;

    output::section("◈", "Training Workers");

    // Queue/run counts come from the database, so they are worth printing even
    // when no worker has registered — that combination is the interesting one.
    let queued = resp.queued_jobs.unwrap_or(0);
    let running = resp.running_jobs.unwrap_or(0);
    output::stat_line(
        "│",
        "Workers",
        &resp.workers.len().to_string(),
        if resp.workers.is_empty() {
            "dim"
        } else {
            "orange"
        },
    );
    output::stat_line(
        "│",
        "Queued jobs",
        &queued.to_string(),
        if queued > 0 { "yellow" } else { "dim" },
    );
    output::stat_line(
        "│",
        "Running jobs",
        &running.to_string(),
        if running > 0 { "orange" } else { "dim" },
    );

    if resp.workers.is_empty() {
        println!();
        output::info("No training workers have registered.");
        if queued > 0 {
            output::warn(&format!(
                "{queued} job(s) are queued with no worker to claim them."
            ));
        }
        println!();
        return Ok(());
    }

    output::subsection("Workers");
    output::table_header(&[
        ("WORKER", 20),
        ("DEVICE", 8),
        ("STATUS", 10),
        ("GPU", 6),
        ("MEM", 6),
        ("HB", 6),
        ("CURRENT JOB", output::ID_W),
    ]);

    for w in &resp.workers {
        println!(
            "    {} {} {} {} {} {} {}",
            output::pad(
                &output::truncate(w.worker_id.as_deref().unwrap_or("-"), 20),
                20
            ),
            output::pad(&output::truncate(w.device.as_deref().unwrap_or("-"), 8), 8),
            output::pad(
                &output::status_indicator(w.status.as_deref().unwrap_or("unknown")),
                10
            ),
            output::pad(
                &w.gpu_util
                    .map(|v| format!("{v:.0}%"))
                    .unwrap_or_else(|| "-".into()),
                6
            ),
            output::pad(
                &w.memory_util
                    .map(|v| format!("{v:.0}%"))
                    .unwrap_or_else(|| "-".into()),
                6
            ),
            output::pad(
                &w.last_heartbeat_age_seconds
                    .map(human_secs)
                    .unwrap_or_else(|| "-".into()),
                6
            ),
            w.current_job
                .as_ref()
                .and_then(|c| c.id.clone())
                .unwrap_or_else(|| "-".into())
        );
    }

    let stale = resp
        .workers
        .iter()
        .filter(|w| w.status.as_deref() == Some("stale"))
        .count();
    if stale > 0 {
        println!();
        output::warn(&format!(
            "{stale} worker(s) are stale — they stopped sending heartbeats."
        ));
    }

    output::count_paged("worker(s)", resp.workers.len(), None, None);
    println!();
    Ok(())
}

// ── start ───────────────────────────────────────────────────────────────

/// Read the submission body from a file.
///
/// The job config has nested hyperparameters, a GPU block and an optional
/// dataset mixture, so it is a JSON document rather than a set of flags; the
/// body is posted exactly as written.
fn read_config(path: &str) -> RmsResult<Value> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        RmsError::InvalidInput(format!("--config {path}: cannot read the file ({e})"))
    })?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| RmsError::InvalidInput(format!("--config {path}: not valid JSON ({e})")))?;
    if !value.is_object() {
        return Err(RmsError::InvalidInput(format!(
            "--config {path}: expected a JSON object holding the job config"
        )));
    }
    Ok(value)
}

fn str_field(cfg: &Value, key: &str) -> Option<String> {
    cfg.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn has_rows(cfg: &Value, key: &str) -> bool {
    cfg.get(key)
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
}

async fn start(client: &ApiClient, ctx: &Ctx, config: &str) -> RmsResult<()> {
    let cfg = read_config(config)?;
    let kind = str_field(&cfg, "kind").unwrap_or_else(|| "supervised".to_string());

    // Validate what the route validates, so a bad config fails naming the key
    // instead of coming back as a bare 400.
    let what = if kind == "sim_rl" {
        let scene = str_field(&cfg, "sceneId").ok_or_else(|| {
            RmsError::InvalidInput(
                "config is missing \"sceneId\", required for a sim_rl job".into(),
            )
        })?;
        format!("sim_rl job in scene {scene}")
    } else {
        let mixture = has_rows(&cfg, "mixture") || has_rows(&cfg, "datasetIds");
        let dataset = str_field(&cfg, "datasetId");
        if !mixture && dataset.is_none() {
            return Err(RmsError::InvalidInput(
                "config is missing \"datasetId\" (or a non-empty \"mixture\"/\"datasetIds\")"
                    .into(),
            ));
        }
        let base_model = str_field(&cfg, "baseModel")
            .ok_or_else(|| RmsError::InvalidInput("config is missing \"baseModel\"".into()))?;
        let method = str_field(&cfg, "fineTuneMethod")
            .ok_or_else(|| RmsError::InvalidInput("config is missing \"fineTuneMethod\"".into()))?;
        let on = if mixture {
            "a dataset mixture".to_string()
        } else {
            format!("dataset {}", dataset.unwrap_or_default())
        };
        format!("{kind} job: {base_model} / {method} on {on}")
    };

    if !output::confirm(
        &format!("This will SUBMIT a new {what} — config from {config}"),
        ctx.yes,
    ) {
        return Ok(());
    }

    let resp = client.post_value("/training/jobs", &cfg).await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    let id = resp
        .get("job")
        .and_then(|j| j.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("(unknown)");
    output::success(&format!("Training job submitted: {id}"));
    if let Some(msg) = resp.get("message").and_then(Value::as_str) {
        output::info(msg);
    }
    println!();
    Ok(())
}

// ── cancel / retry ──────────────────────────────────────────────────────

async fn cancel(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    if !output::confirm(
        &format!("This will CANCEL training job {id}, discarding its progress"),
        ctx.yes,
    ) {
        return Ok(());
    }
    mutate(
        client,
        ctx,
        &format!("/training/jobs/{id}/cancel"),
        "cancelled",
        id,
    )
    .await
}

async fn retry(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    if !output::confirm(
        &format!("This will RE-QUEUE training job {id} for another run"),
        ctx.yes,
    ) {
        return Ok(());
    }
    mutate(
        client,
        ctx,
        &format!("/training/jobs/{id}/retry"),
        "re-queued",
        id,
    )
    .await
}

/// Both routes take no body and answer `{ job, message }`.
async fn mutate(client: &ApiClient, ctx: &Ctx, path: &str, verb: &str, id: &str) -> RmsResult<()> {
    let raw = client.post_value(path, &serde_json::json!({})).await?;

    if ctx.json {
        return output::json_out(&raw);
    }

    output::success(&format!("Training job {id} {verb}"));
    let resp: JobMutationResponse = serde_json::from_value(raw)
        .map_err(|e| RmsError::Parse(format!("unexpected response shape: {e}")))?;
    if let Some(msg) = &resp.message {
        output::info(msg);
    }
    if let Some(status) = resp.job.as_ref().and_then(|j| j.status.as_ref()) {
        output::field("Status", &output::status_indicator(status));
    }
    println!();
    Ok(())
}

// ── export ──────────────────────────────────────────────────────────────

async fn export(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/training/jobs/{id}/export");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let m: RunManifest = client.get(&path).await?;

    output::section("◈", &format!("Run Manifest: {}", id.bold()));
    output::field("Schema", m.schema_version.as_deref().unwrap_or("-"));
    output::field("Run", m.run_id.as_deref().unwrap_or(id));
    output::field("Exported", m.created_at.as_deref().unwrap_or("-"));
    output::field("Source", m.source_server.as_deref().unwrap_or("-"));

    if let Some(j) = &m.job {
        let kind = j.kind.as_deref();
        output::field("Kind", kind.unwrap_or("-"));
        match &j.base_model {
            Some(v) => output::field("Base Model", v),
            None => output::field("Base Model", &absent(kind, "model")),
        }
        match &j.fine_tune_method {
            Some(v) => output::field("Method", v),
            None => output::field("Method", &absent(kind, "method")),
        }
        if let Some(v) = &j.status {
            output::field("Status", &output::status_indicator(v));
        }
        if let Some(init) = j.init_from.as_ref().filter(|v| !v.is_null()) {
            output::field("Init from", &fmt_value(init));
        }
    }

    if let Some(datasets) = m.datasets.as_ref().filter(|d| !d.is_empty()) {
        output::subsection(&format!("Datasets ({})", datasets.len()));
        output::table_header(&[
            ("NAME", 30),
            ("VERSION", 8),
            ("ROBOT", 16),
            ("W", 3),
            ("SHARE", 6),
            ("EPISODES", 9),
            ("FRAMES", 9),
            ("PORT", 4),
        ]);
        for d in datasets {
            println!(
                "    {} {} {} {} {} {} {} {}",
                output::pad(&output::truncate(d.name.as_deref().unwrap_or("-"), 30), 30),
                output::pad(d.lerobot_version.as_deref().unwrap_or("-"), 8),
                output::pad(
                    &output::truncate(d.robot_type.as_deref().unwrap_or("-"), 16),
                    16
                ),
                output::pad(&d.weight.map(fmt_num).unwrap_or_else(|| "-".into()), 3),
                output::pad(
                    &d.normalized_weight
                        .map(|w| format!("{:.0}%", w * 100.0))
                        .unwrap_or_else(|| "-".into()),
                    6
                ),
                output::pad(
                    &d.total_episodes
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".into()),
                    9
                ),
                output::pad(
                    &d.total_frames
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".into()),
                    9
                ),
                output::bool_indicator(d.portable)
            );
        }
        // The scheme-tagged URI is what another cluster actually opens, so it
        // gets its own line per member rather than a clipped table cell.
        for d in datasets {
            if let Some(uri) = &d.uri {
                println!();
                println!(
                    "    {} {} {}",
                    "│".bright_black(),
                    d.name.as_deref().unwrap_or("(unnamed)").bold(),
                    d.license
                        .as_deref()
                        .map(|l| format!("({l})"))
                        .unwrap_or_default()
                        .bright_black()
                );
                prose(uri);
            }
        }
        // A non-portable member is a local path no other cluster can open.
        let stuck: Vec<&str> = datasets
            .iter()
            .filter(|d| d.portable == Some(false))
            .map(|d| d.name.as_deref().unwrap_or("(unnamed)"))
            .collect();
        if !stuck.is_empty() {
            println!();
            output::warn(&format!(
                "Not portable: {} — this run cannot be handed to another cluster as-is.",
                stuck.join(", ")
            ));
        }
    }

    if let Some(c) = &m.compatibility {
        output::subsection("Compatibility");
        output::field_colored("Verdict", verdict_cell(c.verdict.as_deref().unwrap_or("-")));
        if let Some(h) = &c.headline {
            prose(h);
        }
        if let Some(r) = &c.recommendation {
            println!();
            prose(r);
        }
        if let Some(axes) = c.axes.as_ref().filter(|a| !a.is_empty()) {
            println!();
            output::table_header(&[("AXIS", 20), ("VERDICT", 10), ("NOTE", 60)]);
            for a in axes {
                println!(
                    "    {} {} {}",
                    output::pad(
                        &output::truncate(
                            a.label.as_deref().or(a.axis.as_deref()).unwrap_or("-"),
                            20
                        ),
                        20
                    ),
                    output::pad(
                        &verdict_cell(a.verdict.as_deref().unwrap_or("-")).to_string(),
                        10
                    ),
                    output::truncate(a.note.as_deref().unwrap_or("-"), 60)
                );
            }
        }
    }

    if non_empty_object(m.hyperparameters.as_ref()).is_some() {
        output::subsection("Hyperparameters");
        render_map(m.hyperparameters.as_ref());
    }

    if let Some(g) = &m.gpu {
        output::subsection("GPU");
        output::field("Count", &g.count.unwrap_or(1).to_string());
        if let Some(mem) = g.memory {
            output::field("Memory", &format!("{} GB", fmt_num(mem)));
        }
        output::field("Type", g.gpu_type.as_deref().unwrap_or("any"));
    }

    if let Some(r) = &m.runtime {
        output::subsection("Runtime");
        output::field("Image", r.image.as_deref().unwrap_or("-"));
        output::field("Entrypoint", r.entrypoint.as_deref().unwrap_or("-"));
        if let Some(cmd) = &r.command {
            output::field("Command", &cmd.join(" "));
        }
    }

    if let Some(c) = &m.compliance {
        output::subsection("Compliance");
        output::field(
            "Licenses",
            &c.dataset_licenses
                .as_ref()
                .map(|l| l.join(", "))
                .unwrap_or_else(|| "-".into()),
        );
        output::field("Residency", c.residency.as_deref().unwrap_or("unknown"));
        for n in c.notes.iter().flatten() {
            println!();
            prose(n);
        }
    }

    for w in m.warnings.iter().flatten() {
        println!();
        warn_prose(w);
    }

    println!();
    output::info("Use --json to save the manifest verbatim.");
    println!();
    Ok(())
}

// ── estimate ────────────────────────────────────────────────────────────

async fn estimate(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/training/jobs/{id}/estimate");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    // The route answers 400 when the job has no dataset (sim_rl): the estimate
    // is frame-based, so there is nothing to count.
    let e: DurationEstimate = client.get(&path).await?;

    output::section("◈", &format!("Duration Estimate: {}", id.bold()));

    if let Some(m) = e.estimated_minutes {
        output::field(
            "Duration",
            &format!("{} ({} min)", human_minutes(m), fmt_num(m.round())),
        );
    }
    if let Some(s) = e.estimated_steps {
        output::field("Steps", &fmt_num(s));
    }
    if let Some(s) = e.steps_per_second {
        output::field("Throughput", &format!("{s} steps/s"));
    }
    if let Some(c) = &e.confidence {
        output::field_colored(
            "Confidence",
            match c.as_str() {
                "high" => c.green(),
                "medium" => c.yellow(),
                "low" => c.red(),
                _ => c.normal(),
            },
        );
    }

    println!();
    output::info("Estimated from dataset frame counts and the job's hyperparameters.");
    println!();
    Ok(())
}
