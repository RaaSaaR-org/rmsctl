use super::dataset_download;
use super::dataset_upload::{Tuning, human_bytes};
use super::model_upload;
use crate::cli::ModelsCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::models::Pagination;
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

pub async fn run(cmd: &ModelsCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        ModelsCommand::List => list(client, ctx).await,
        ModelsCommand::Show { id } => show(client, ctx, id).await,
        ModelsCommand::Lineage { id } => lineage(client, ctx, id).await,
        ModelsCommand::Compare {
            version_a,
            version_b,
            metrics,
        } => compare(client, ctx, version_a, version_b, metrics.as_deref()).await,
        ModelsCommand::Upload {
            dir,
            name,
            model_version,
            model_type,
            base_model,
            dataset,
            metrics,
            eval_metrics,
            notes,
            exclude,
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
            let opts = model_upload::ModelUploadOptions {
                dir: dir.clone(),
                name: name.clone(),
                version: model_version.clone(),
                model_type: model_type.clone(),
                base_model: base_model.clone(),
                dataset: dataset.clone(),
                metrics: metrics.clone(),
                eval_metrics: eval_metrics.clone(),
                notes: notes.clone(),
                exclude: exclude.clone(),
                chunk_size: *chunk_size,
                parallel: *parallel as usize,
                retries: *retries,
                resume: resume.clone(),
                restart: *restart,
                no_checksum: *no_checksum,
                no_wait: *no_wait,
                wait_timeout: Duration::from_secs(*wait_timeout),
                part_timeout: Duration::from_secs((*part_timeout).max(1)),
                state_dir: None,
                tuning: Tuning::default(),
            };
            model_upload::run_upload(client, ctx, opts).await
        }
        ModelsCommand::UploadStatus { id } => {
            model_upload::status(client, ctx, id.as_deref()).await
        }
        ModelsCommand::UploadCancel { id } => model_upload::cancel(client, ctx, id, None).await,
        ModelsCommand::Files { id } => model_upload::files(client, ctx, id).await,
        ModelsCommand::Download {
            id,
            out,
            parallel,
            retries,
            resume,
            force,
            no_checksum,
            stall_timeout,
        } => {
            // The dataset download's engine: the server hands a model's files
            // out the same way.
            let opts = dataset_download::DownloadOptions {
                kind: dataset_download::Kind::Model,
                id: resolve_id(client, id).await?,
                out: out.clone(),
                parallel: *parallel as usize,
                retries: *retries,
                resume: *resume,
                force: *force,
                no_checksum: *no_checksum,
                stall_timeout: Duration::from_secs((*stall_timeout).max(1)),
                tuning: Tuning::default(),
            };
            dataset_download::run_download(client, ctx, opts).await
        }
        ModelsCommand::Delete { id } => delete(client, ctx, id).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────
//
// The registry is flat: the server stores model VERSIONS and has no
// registered-model / version-rollup concept, so one row type serves the
// collection, the detail route, the parent edge and the lineage chain. The
// relation fields at the bottom are sent by `GET /models/versions/:id` only
// (`findByIdWithRelations`), never by the collection.

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelVersionRow {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    model_type: Option<String>,
    #[serde(default)]
    deployment_status: Option<String>,
    #[serde(default)]
    source_kind: Option<String>,
    #[serde(default)]
    artifact_uri: Option<String>,
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    training_job_id: Option<String>,
    #[serde(default)]
    parent_model_version_id: Option<String>,
    // What a model without a training job on this server says about itself:
    // an uploaded checkpoint states its base model and training dataset.
    #[serde(default)]
    base_model: Option<String>,
    #[serde(default)]
    dataset_id: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    // Set only for a model whose files this instance stores.
    #[serde(default)]
    artifact_size_bytes: Option<u64>,
    #[serde(default)]
    artifact_file_count: Option<u64>,
    // Metric values are heterogeneous — a scalar, a string ("trainer") or a
    // per-epoch array — so they stay as raw JSON.
    #[serde(default)]
    training_metrics: Option<Value>,
    #[serde(default)]
    validation_metrics: Option<Value>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    skill: Option<SkillRef>,
    #[serde(default)]
    parent: Option<Box<ModelVersionRow>>,
    #[serde(default)]
    children: Option<Vec<ModelVersionRow>>,
    #[serde(default)]
    checkpoints: Option<Vec<Checkpoint>>,
}

impl ModelVersionRow {
    /// Best human label: rows may carry a name, a version, or neither.
    fn label(&self) -> String {
        self.name
            .as_deref()
            .filter(|n| !n.is_empty())
            .or(self.version.as_deref())
            .unwrap_or(&self.id)
            .to_string()
    }

    /// One-line identity for a relation field: `name · version · status`.
    fn summary(&self) -> String {
        let mut parts = vec![self.label()];
        if let Some(v) = &self.version
            && Some(v.as_str()) != self.name.as_deref()
        {
            parts.push(v.clone());
        }
        if let Some(s) = &self.deployment_status {
            parts.push(s.clone());
        }
        parts.join(" · ")
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillRef {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Checkpoint {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    epoch: Option<i64>,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    training_job_id: Option<String>,
    #[serde(default)]
    metrics: Option<Value>,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelVersionsResponse {
    #[serde(default)]
    model_versions: Vec<ModelVersionRow>,
    #[serde(default)]
    pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelVersionDetailResponse {
    model_version: ModelVersionRow,
    #[serde(default)]
    evaluation: Option<EvalRollup>,
    /// The dataset `datasetId` names, when it still exists on the instance.
    #[serde(default)]
    trained_on_dataset: Option<DatasetRef>,
    /// Present for a model uploaded to this instance.
    #[serde(default)]
    storage: Option<StoredFiles>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatasetRef {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredFiles {
    #[serde(default)]
    storage: Option<String>,
    #[serde(default)]
    files_removed_at: Option<String>,
}

/// How a model got into the registry, in words.
fn source_label(kind: &str) -> &str {
    match kind {
        "uploaded" => "uploaded directly",
        "training" => "trained here",
        "imported" => "registered from outside",
        "derived" => "derived from another model",
        other => other,
    }
}

/// Counted over the EvaluationEpisode rows carrying this model's id, so a model
/// that was never evaluated reports zeros rather than nulls.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EvalRollup {
    #[serde(default)]
    episode_count: Option<u32>,
    #[serde(default)]
    success_count: Option<u32>,
    #[serde(default)]
    success_rate: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LineageResponse {
    #[serde(default)]
    lineage: Option<Lineage>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Lineage {
    #[serde(default)]
    model_version_id: Option<String>,
    #[serde(default)]
    ancestors: Vec<ModelVersionRow>,
    #[serde(default)]
    children: Vec<ModelVersionRow>,
}

// ── list ────────────────────────────────────────────────────────────────

async fn list(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    // Sent for the day the route reads them, but they change nothing today:
    // the handler is `(_req, res)` and calls findAll() with no arguments, so
    // every response is the first page at the repository default of 50.
    let query = ctx.paging();

    if ctx.json {
        return output::json_out(&client.get_value_query("/models/versions", &query).await?);
    }

    let resp: ModelVersionsResponse = client.get_query("/models/versions", &query).await?;

    output::section("◈", "Model Versions");

    if resp.model_versions.is_empty() {
        output::info("No model versions registered.");
        println!();
        return Ok(());
    }

    // NAME grows with the terminal; the other columns keep their widths.
    let name_w = output::flex_width(
        26,
        output::fixed_width(&[output::ID_W, 14, 11, 9, 9, 14]),
        output::longest(resp.model_versions.iter().filter_map(|m| m.name.as_deref())),
        output::term_width(),
    );
    output::table_header(&[
        ("ID", output::ID_W),
        ("NAME", name_w),
        ("VERSION", 14),
        ("BASE", 11),
        ("SOURCE", 9),
        ("SIZE", 9),
        ("STATUS", 14),
    ]);
    for m in &resp.model_versions {
        println!(
            "    {} {} {} {} {} {} {}",
            output::pad(&m.id, output::ID_W),
            output::pad(
                &output::truncate(m.name.as_deref().unwrap_or("-"), name_w),
                name_w
            ),
            output::pad(
                &output::truncate(m.version.as_deref().unwrap_or("-"), 14),
                14
            ),
            // A model that states no base model falls back to its kind.
            output::pad(
                &output::truncate(
                    m.base_model
                        .as_deref()
                        .or(m.model_type.as_deref())
                        .unwrap_or("-"),
                    11
                ),
                11
            ),
            output::pad(m.source_kind.as_deref().unwrap_or("-"), 9),
            output::pad(
                &m.artifact_size_bytes
                    .map(human_bytes)
                    .unwrap_or_else(|| "-".into()),
                9
            ),
            deployment_indicator(m.deployment_status.as_deref().unwrap_or("unknown")),
        );
    }

    let total = resp.pagination.as_ref().and_then(|p| p.total());
    output::count_paged(
        "model version(s)",
        resp.model_versions.len(),
        total,
        resp.pagination.as_ref().and_then(|p| p.total_pages()),
    );

    // count_paged offers --page/--limit when the envelope reports more rows
    // than were printed; on this route neither flag can reach them.
    if ctx.has_paging() || total.is_some_and(|t| t as usize > resp.model_versions.len()) {
        output::warn(
            "GET /models/versions ignores page/pageSize — the server always returns the first 50 rows.",
        );
    }
    println!();
    Ok(())
}

/// `output::status_indicator` has no arm for `production`, so the one stage that
/// matters most would render in neutral grey, indistinguishable from
/// `archived`. Only that value is special-cased; `staging`, `canary` and
/// `archived` keep the shared palette.
fn deployment_indicator(status: &str) -> String {
    match status.to_lowercase().as_str() {
        "production" => format!("{} {}", "●".bright_black(), status.green().bold()),
        _ => output::status_indicator(status),
    }
}

// ── show ────────────────────────────────────────────────────────────────

async fn show(client: &ApiClient, ctx: &Ctx, ident: &str) -> RmsResult<()> {
    let id = resolve_id(client, ident).await?;
    let path = format!("/models/versions/{id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp: ModelVersionDetailResponse = client.get(&path).await?;
    output::section(
        "◈",
        &format!("Model Version: {}", resp.model_version.label().bold()),
    );
    render_detail(&resp);
    println!();
    Ok(())
}

fn render_detail(resp: &ModelVersionDetailResponse) {
    let m = &resp.model_version;
    render_identity(m);
    render_origin(resp);

    if let Some(e) = &resp.evaluation {
        render_evaluation(e, has_metrics(m.validation_metrics.as_ref()));
    }
    render_metrics("Training metrics", m.training_metrics.as_ref());
    render_metrics("Validation metrics", m.validation_metrics.as_ref());
    render_relations(m);
}

/// Draw the body of `GET /models/versions/:id` — for a command that has the
/// body in hand already, like `models upload` once the model is registered.
pub(crate) fn render_result(value: &Value) -> RmsResult<()> {
    let resp: ModelVersionDetailResponse = serde_json::from_value(value.clone())?;
    render_detail(&resp);
    Ok(())
}

/// What the model was made from and what the instance stores of it.
fn render_origin(resp: &ModelVersionDetailResponse) {
    let m = &resp.model_version;
    if let Some(base) = &m.base_model {
        output::field("Base model", base);
    }
    match (&resp.trained_on_dataset, &m.dataset_id) {
        (Some(d), _) => output::field(
            "Trained on",
            &format!(
                "{} ({})",
                d.name.as_deref().unwrap_or("-"),
                d.id.as_deref().unwrap_or("-")
            ),
        ),
        (None, Some(id)) => {
            output::field("Trained on", &format!("{id} (no longer on this instance)"))
        }
        (None, None) => {}
    }
    if let Some(bytes) = m.artifact_size_bytes {
        let files = m
            .artifact_file_count
            .map(|n| format!(" in {n} file(s)"))
            .unwrap_or_default();
        output::field("Stored", &format!("{}{files}", human_bytes(bytes)));
    }
    if let Some(removed) = resp
        .storage
        .as_ref()
        .and_then(|s| s.files_removed_at.as_deref())
    {
        output::field_colored("Files", format!("removed {removed}").red());
    } else if let Some(store) = resp.storage.as_ref().and_then(|s| s.storage.as_deref()) {
        output::field("Store", store);
    }
    if let Some(notes) = m.notes.as_deref().filter(|n| !n.is_empty()) {
        output::field("Notes", notes);
    }
}

// ── delete ──────────────────────────────────────────────────────────────

/// `DELETE /models/versions/:id` archives the row — deployments, evaluations
/// and lineage point at it — and, for a model uploaded to the instance,
/// removes the files it stored.
async fn delete(client: &ApiClient, ctx: &Ctx, ident: &str) -> RmsResult<()> {
    let id = resolve_id(client, ident).await?;
    let path = format!("/models/versions/{id}");
    let detail: ModelVersionDetailResponse = client.get(&path).await?;
    let m = &detail.model_version;
    let stored = match (m.artifact_size_bytes, &detail.storage) {
        (Some(bytes), Some(s)) if s.files_removed_at.is_none() => {
            format!(
                " and delete the {} this instance stores for it",
                human_bytes(bytes)
            )
        }
        _ => String::new(),
    };
    if !output::confirm(
        &format!(
            "This will archive model version {id} (\"{}\"){stored}",
            m.summary()
        ),
        ctx.yes,
    ) {
        return Ok(());
    }
    let body = client.delete_value(&path).await?;
    if ctx.json {
        return output::json_out(&body);
    }
    match body.get("freedBytes").and_then(|b| b.as_u64()) {
        Some(freed) => output::success(&format!(
            "Model version {id} archived; its stored files are removed ({} freed).",
            human_bytes(freed)
        )),
        None => output::success(&format!("Model version {id} archived.")),
    }
    println!();
    Ok(())
}

fn render_identity(m: &ModelVersionRow) {
    output::field("ID", &m.id);
    output::field("Name", m.name.as_deref().unwrap_or("-"));
    output::field("Version", m.version.as_deref().unwrap_or("-"));
    output::field("Type", m.model_type.as_deref().unwrap_or("-"));
    output::field(
        "Status",
        &deployment_indicator(m.deployment_status.as_deref().unwrap_or("unknown")),
    );
    if let Some(src) = &m.source_kind {
        output::field("Source", source_label(src));
    }
    if let Some(uri) = &m.artifact_uri {
        output::field("Artifact", uri);
    }
    match (&m.skill, &m.skill_id) {
        (Some(s), _) => {
            let mut label = s.name.clone().unwrap_or_else(|| "-".to_string());
            if let Some(v) = &s.version {
                label.push_str(&format!(" · {v}"));
            }
            if let Some(id) = &s.id {
                label.push_str(&format!(" ({id})"));
            }
            output::field("Skill", &label);
        }
        (None, Some(id)) => output::field("Skill", id),
        (None, None) => {}
    }
    if let Some(job) = &m.training_job_id {
        output::field("Training job", job);
    }
    if let Some(at) = &m.created_at {
        output::field("Created", at);
    }
    if let Some(at) = &m.updated_at {
        output::field("Updated", at);
    }
}

/// What to say in place of the rollup when this server ran no evaluation
/// episode for the model, or `None` when it ran some.
///
/// The rollup counts evaluation episodes run on this server; the validation
/// metrics are what came with the model. "Episodes 0 / Successes 0" printed
/// above "eval_successes 26 of 50" reads as a contradiction, so a rollup of
/// zeros is one line that says whose numbers are missing.
fn no_evaluation_note(e: &EvalRollup, has_validation_metrics: bool) -> Option<&'static str> {
    if e.episode_count.unwrap_or(0) > 0 {
        return None;
    }
    Some(if has_validation_metrics {
        "no evaluation runs recorded on this server — the validation metrics below came with the model"
    } else {
        "no evaluation runs recorded on this server"
    })
}

fn has_metrics(metrics: Option<&Value>) -> bool {
    metrics
        .and_then(|v| v.as_object())
        .is_some_and(|o| !o.is_empty())
}

fn render_evaluation(e: &EvalRollup, has_validation_metrics: bool) {
    output::subsection("Evaluation");
    if let Some(note) = no_evaluation_note(e, has_validation_metrics) {
        println!("    {} {}", "│".bright_black(), note.bright_black());
        return;
    }
    let episodes = e.episode_count.unwrap_or(0);
    output::field("Episodes", &episodes.to_string());
    output::field("Successes", &e.success_count.unwrap_or(0).to_string());
    match e.success_rate {
        // successRate is successCount/episodeCount, and a model with no
        // episodes gets a literal 0 — that is "never evaluated", not "always
        // fails", so it must not be drawn as an empty bar.
        Some(rate) if episodes > 0 => output::meter(
            "Success rate",
            rate,
            1.0,
            if rate >= 0.8 {
                "green"
            } else if rate >= 0.5 {
                "yellow"
            } else {
                "red"
            },
        ),
        _ => output::field_colored("Success rate", "no evaluation episodes".bright_black()),
    }
}

fn render_metrics(title: &str, metrics: Option<&Value>) {
    output::subsection(title);
    let obj = metrics
        .and_then(|v| v.as_object())
        .filter(|o| !o.is_empty());
    let Some(obj) = obj else {
        println!(
            "    {} {}",
            "│".bright_black(),
            "none recorded".bright_black()
        );
        return;
    };

    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort();
    for k in keys {
        // Keys run past the 14-column label gutter (`total_timesteps`,
        // `validation_loss`); nudging the value right beats clipping the name.
        output::field(
            &output::truncate(k, 24),
            &metric_display(obj.get(k).unwrap_or(&Value::Null)),
        );
    }
}

fn render_relations(m: &ModelVersionRow) {
    output::subsection("Lineage");

    match (&m.parent, &m.parent_model_version_id) {
        (Some(p), _) => output::field("Parent", &format!("{} ({})", p.summary(), p.id)),
        (None, Some(id)) => output::field("Parent", id),
        (None, None) => output::field_colored("Parent", "none — root model".bright_black()),
    }

    let children = m.children.as_deref().unwrap_or(&[]);
    if children.is_empty() {
        output::field_colored("Children", "none".bright_black());
    } else {
        output::field("Children", &children.len().to_string());
        println!();
        render_row_table(children);
    }

    let checkpoints = m.checkpoints.as_deref().unwrap_or(&[]);
    if !checkpoints.is_empty() {
        output::subsection(&format!("Checkpoints ({})", checkpoints.len()));
        output::table_header(&[("EPOCH", 7), ("URI", 46), ("CREATED", 26)]);
        for c in checkpoints {
            println!(
                "    {} {} {}",
                output::pad(
                    &c.epoch.map(|e| e.to_string()).unwrap_or_else(|| "-".into()),
                    7
                ),
                output::pad(&output::truncate(c.uri.as_deref().unwrap_or("-"), 46), 46),
                c.created_at.as_deref().unwrap_or("-"),
            );
        }
    }
}

/// Shared table for the relation lists (children, ancestors) — every one of
/// them is a full model-version row.
fn render_row_table(rows: &[ModelVersionRow]) {
    let name_w = output::flex_width(
        26,
        output::fixed_width(&[output::ID_W, 10, 14]),
        output::longest(rows.iter().filter_map(|r| r.name.as_deref())),
        output::term_width(),
    );
    output::table_header(&[
        ("ID", output::ID_W),
        ("NAME", name_w),
        ("VERSION", 10),
        ("STATUS", 14),
    ]);
    for r in rows {
        println!(
            "    {} {} {} {}",
            output::pad(&r.id, output::ID_W),
            output::pad(
                &output::truncate(r.name.as_deref().unwrap_or("-"), name_w),
                name_w
            ),
            output::pad(
                &output::truncate(r.version.as_deref().unwrap_or("-"), 10),
                10
            ),
            deployment_indicator(r.deployment_status.as_deref().unwrap_or("unknown")),
        );
    }
}

// ── lineage ─────────────────────────────────────────────────────────────

async fn lineage(client: &ApiClient, ctx: &Ctx, ident: &str) -> RmsResult<()> {
    let id = resolve_id(client, ident).await?;
    let path = format!("/models/versions/{id}/lineage");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp: LineageResponse = client.get(&path).await?;
    let l = resp.lineage.ok_or_else(|| {
        RmsError::Parse("lineage response carried no `lineage` block".to_string())
    })?;

    output::section("◈", "Model Lineage");
    output::field("Model", l.model_version_id.as_deref().unwrap_or(&id));

    output::subsection(&format!("Ancestors ({})", l.ancestors.len()));
    if l.ancestors.is_empty() {
        println!(
            "    {} {}",
            "│".bright_black(),
            "none — this is a root model".bright_black()
        );
    } else {
        // getLineage walks the parent edge upward, so ancestors[0] is the
        // direct parent and the last entry is the root.
        output::table_header(&[
            ("HOP", 5),
            ("ID", output::ID_W),
            ("NAME", 26),
            ("VERSION", 10),
            ("STATUS", 14),
        ]);
        for (i, a) in l.ancestors.iter().enumerate() {
            println!(
                "    {} {} {} {} {}",
                output::pad(&format!("-{}", i + 1), 5),
                output::pad(&a.id, output::ID_W),
                output::pad(&output::truncate(a.name.as_deref().unwrap_or("-"), 26), 26),
                output::pad(
                    &output::truncate(a.version.as_deref().unwrap_or("-"), 10),
                    10
                ),
                deployment_indicator(a.deployment_status.as_deref().unwrap_or("unknown")),
            );
        }
    }

    output::subsection(&format!("Children ({})", l.children.len()));
    if l.children.is_empty() {
        println!(
            "    {} {}",
            "│".bright_black(),
            "none — nothing was fine-tuned from this model".bright_black()
        );
    } else {
        render_row_table(&l.children);
    }

    println!();
    output::info("hop -1 is the direct parent; the last hop is the root model.");
    println!();
    Ok(())
}

// ── compare ─────────────────────────────────────────────────────────────

/// One metric key as it appears on either side of the comparison.
struct MetricDiff {
    key: String,
    a: Option<Value>,
    b: Option<Value>,
}

async fn compare(
    client: &ApiClient,
    ctx: &Ctx,
    ident_a: &str,
    ident_b: &str,
    metrics: Option<&str>,
) -> RmsResult<()> {
    // There is no comparison endpoint: the only one on the server is
    // GET /evaluation/compare, which diffs evaluation *episodes* by version
    // string and exposes no metric-key selection. So both versions are fetched
    // and their metric maps diffed here, which is also the only way --metrics
    // can mean anything.
    let filter = parse_metric_filter(metrics)?;

    let (id_a, id_b) = resolve_pair(client, ident_a, ident_b).await?;
    if id_a == id_b {
        return Err(RmsError::InvalidInput(format!(
            "'{ident_a}' and '{ident_b}' are the same model version ({id_a}) — nothing to compare"
        )));
    }

    let (path_a, path_b) = (
        format!("/models/versions/{id_a}"),
        format!("/models/versions/{id_b}"),
    );
    let (raw_a, raw_b) = tokio::join!(client.get_value(&path_a), client.get_value(&path_b));
    let (raw_a, raw_b) = (raw_a?, raw_b?);

    let det_a: ModelVersionDetailResponse = serde_json::from_value(raw_a.clone())?;
    let det_b: ModelVersionDetailResponse = serde_json::from_value(raw_b.clone())?;
    let (a, b) = (&det_a.model_version, &det_b.model_version);

    let training = diff_metrics(
        a.training_metrics.as_ref(),
        b.training_metrics.as_ref(),
        filter.as_deref(),
    );
    let validation = diff_metrics(
        a.validation_metrics.as_ref(),
        b.validation_metrics.as_ref(),
        filter.as_deref(),
    );

    if ctx.json {
        // Nothing to pass through — the diff is computed locally — so both
        // server bodies go out verbatim alongside it.
        return output::json_out(&serde_json::json!({
            "a": raw_a,
            "b": raw_b,
            "trainingMetrics": diff_json(&training),
            "validationMetrics": diff_json(&validation),
        }));
    }

    output::section("◈", "Model Comparison");
    output::field("A", &a.summary());
    output::field("A id", &a.id);
    output::field("B", &b.summary());
    output::field("B id", &b.id);

    render_diff("Training metrics", &training);
    render_diff("Validation metrics", &validation);

    println!();
    if let Some(keys) = &filter {
        let missing: Vec<&str> = keys
            .iter()
            .filter(|k| {
                !training
                    .iter()
                    .chain(validation.iter())
                    .any(|d| &&d.key == k)
            })
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            output::warn(&format!(
                "no such metric on either version: {} — real keys include training_loss, validation_loss, final_loss, best_epoch, success_rate, mean_reward",
                missing.join(", ")
            ));
        }
    }
    output::info("⋯ marks a per-epoch series, reduced to its final value.");
    println!();
    Ok(())
}

/// `--metrics` is a client-side filter over the metric keys; no server route
/// parses a metric-key list.
fn parse_metric_filter(metrics: Option<&str>) -> RmsResult<Option<Vec<String>>> {
    let Some(raw) = metrics else {
        return Ok(None);
    };
    let keys: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string)
        .collect();
    if keys.is_empty() {
        return Err(RmsError::InvalidInput(
            "--metrics was empty: give a comma-separated list of metric keys (e.g. --metrics training_loss,final_loss), or omit it to diff every key".to_string(),
        ));
    }
    Ok(Some(keys))
}

/// Union of the keys on both sides, in the order the user asked for them when
/// `--metrics` was given and alphabetically otherwise. A key the filter names
/// but neither side carries is dropped here and reported by the caller.
fn diff_metrics(
    a: Option<&Value>,
    b: Option<&Value>,
    filter: Option<&[String]>,
) -> Vec<MetricDiff> {
    let ma = a.and_then(|v| v.as_object());
    let mb = b.and_then(|v| v.as_object());

    let keys: Vec<String> = match filter {
        Some(f) => f.to_vec(),
        None => {
            let mut k: Vec<String> = ma
                .map(|o| o.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            k.extend(
                mb.map(|o| o.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
            );
            k.sort();
            k.dedup();
            k
        }
    };

    keys.into_iter()
        .filter_map(|key| {
            let va = ma
                .and_then(|o| o.get(&key))
                .filter(|v| !v.is_null())
                .cloned();
            let vb = mb
                .and_then(|o| o.get(&key))
                .filter(|v| !v.is_null())
                .cloned();
            if va.is_none() && vb.is_none() {
                return None;
            }
            Some(MetricDiff { key, a: va, b: vb })
        })
        .collect()
}

fn diff_json(diffs: &[MetricDiff]) -> Value {
    Value::Array(
        diffs
            .iter()
            .map(|d| {
                let delta = match (
                    d.a.as_ref().and_then(metric_num),
                    d.b.as_ref().and_then(metric_num),
                ) {
                    (Some(na), Some(nb)) => serde_json::json!(nb - na),
                    _ => Value::Null,
                };
                serde_json::json!({
                    "key": d.key,
                    "a": d.a.clone().unwrap_or(Value::Null),
                    "b": d.b.clone().unwrap_or(Value::Null),
                    "delta": delta,
                    "presentIn": match (d.a.is_some(), d.b.is_some()) {
                        (true, true) => "both",
                        (true, false) => "a",
                        (false, true) => "b",
                        (false, false) => "neither",
                    },
                })
            })
            .collect(),
    )
}

fn render_diff(title: &str, diffs: &[MetricDiff]) {
    output::subsection(title);
    if diffs.is_empty() {
        println!(
            "    {} {}",
            "│".bright_black(),
            "no metrics on either version".bright_black()
        );
        return;
    }

    output::table_header(&[("METRIC", 20), ("A", 20), ("B", 20), ("DELTA", 16)]);
    for d in diffs {
        println!(
            "    {} {} {} {}",
            output::pad(&output::truncate(&d.key, 20), 20),
            output::pad(&metric_cell(d.a.as_ref()), 20),
            output::pad(&metric_cell(d.b.as_ref()), 20),
            delta_cell(d.a.as_ref(), d.b.as_ref()),
        );
    }
}

// ── metric formatting ───────────────────────────────────────────────────

/// Numeric view of a metric, for deltas. A per-epoch series reduces to its
/// final point, which is what the two runs are actually compared on.
fn metric_num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Array(items) => items.last().and_then(|l| l.as_f64()),
        _ => None,
    }
}

/// Long form, for the `show` detail view: keeps the series length.
fn metric_display(v: &Value) -> String {
    match v {
        Value::Number(n) => n.as_f64().map(fmt_num).unwrap_or_else(|| n.to_string()),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "-".to_string(),
        Value::Array(items) => match items.last().and_then(|l| l.as_f64()) {
            Some(last) => format!(
                "{} {}",
                fmt_num(last),
                format!("(final of {})", items.len()).bright_black()
            ),
            None => format!("[{} value(s)]", items.len())
                .bright_black()
                .to_string(),
        },
        Value::Object(_) => output::truncate(&v.to_string(), 40),
    }
}

/// Compact form, for the comparison columns.
fn metric_cell(v: Option<&Value>) -> String {
    let Some(v) = v else {
        return "-".bright_black().to_string();
    };
    match v {
        Value::Array(items) => match items.last().and_then(|l| l.as_f64()) {
            Some(last) => format!("{} {}", fmt_num(last), "⋯".bright_black()),
            None => format!("[{}]", items.len()).bright_black().to_string(),
        },
        Value::Number(n) => n.as_f64().map(fmt_num).unwrap_or_else(|| n.to_string()),
        Value::String(s) => output::truncate(s, 18),
        Value::Bool(b) => b.to_string(),
        Value::Null => "-".bright_black().to_string(),
        Value::Object(_) => output::truncate(&v.to_string(), 18),
    }
}

fn delta_cell(a: Option<&Value>, b: Option<&Value>) -> String {
    match (a, b) {
        (Some(_), None) => "only A".yellow().to_string(),
        (None, Some(_)) => "only B".yellow().to_string(),
        (None, None) => "-".bright_black().to_string(),
        (Some(x), Some(y)) => match (metric_num(x), metric_num(y)) {
            (Some(na), Some(nb)) => {
                let d = nb - na;
                // Deliberately not coloured good/bad: lower is better for a
                // loss, higher is better for a reward or a success rate, and
                // the server does not say which kind a key is.
                if d == 0.0 {
                    format!("{} {}", "·".bright_black(), "0".bright_black())
                } else {
                    let sign = if d > 0.0 { "+" } else { "-" };
                    let arrow = if d > 0.0 { "▲" } else { "▼" };
                    format!(
                        "{} {}",
                        arrow.truecolor(255, 103, 0),
                        format!("{sign}{}", fmt_num(d.abs()))
                            .truecolor(255, 103, 0)
                            .bold()
                    )
                }
            }
            _ if x == y => "same".bright_black().to_string(),
            _ => "changed".truecolor(255, 103, 0).to_string(),
        },
    }
}

/// Metrics arrive as anything from 50000000 to 0.2717, so integers print whole
/// and fractions keep four decimals with the padding trimmed off.
fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    if v != 0.0 && v.abs() < 1e-4 {
        return format!("{v:.2e}");
    }
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ── identifier resolution ───────────────────────────────────────────────

/// The server's only single-item route is `GET /models/versions/:id`, resolved
/// with a Prisma `findUnique({ where: { id } })` — there is no lookup by name
/// or by version string anywhere. A non-UUID argument is therefore matched
/// against the collection here. Names and versions are not unique (the live
/// registry holds two rows at version "2.0.0"), so an ambiguous match refuses
/// and lists the candidates instead of silently picking one.
async fn resolve_id(client: &ApiClient, ident: &str) -> RmsResult<String> {
    if looks_like_uuid(ident) {
        return Ok(ident.to_string());
    }
    let resp: ModelVersionsResponse = client.get("/models/versions").await?;
    match_ident(&resp.model_versions, ident)
}

/// As `resolve_id`, but one collection fetch serves both identifiers.
async fn resolve_pair(client: &ApiClient, a: &str, b: &str) -> RmsResult<(String, String)> {
    if looks_like_uuid(a) && looks_like_uuid(b) {
        return Ok((a.to_string(), b.to_string()));
    }
    let resp: ModelVersionsResponse = client.get("/models/versions").await?;
    let rows = &resp.model_versions;
    let id_a = if looks_like_uuid(a) {
        a.to_string()
    } else {
        match_ident(rows, a)?
    };
    let id_b = if looks_like_uuid(b) {
        b.to_string()
    } else {
        match_ident(rows, b)?
    };
    Ok((id_a, id_b))
}

fn match_ident(rows: &[ModelVersionRow], ident: &str) -> RmsResult<String> {
    let needle = ident.trim().to_lowercase();
    let hits: Vec<&ModelVersionRow> = rows
        .iter()
        .filter(|r| {
            r.version
                .as_deref()
                .is_some_and(|v| v.to_lowercase() == needle)
                || r.name
                    .as_deref()
                    .is_some_and(|n| n.to_lowercase() == needle)
        })
        .collect();

    match hits.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => Err(RmsError::InvalidInput(format!(
            "no model version named or versioned '{ident}' — pass a UUID id from 'rmsctl models list'"
        ))),
        many => {
            let candidates = many
                .iter()
                .map(|r| format!("{} ({})", r.id, r.label()))
                .collect::<Vec<_>>()
                .join(", ");
            Err(RmsError::InvalidInput(format!(
                "'{ident}' matches {} model versions — pass one of these ids: {candidates}",
                many.len()
            )))
        }
    }
}

fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rollup(v: Value) -> EvalRollup {
        serde_json::from_value(v).unwrap()
    }

    /// The uploaded model on the demo instance: the server ran no evaluation
    /// episode, the upload stated 26 of 50.
    #[test]
    fn a_zero_rollup_is_one_line_that_does_not_contradict_the_validation_metrics() {
        let zeros = rollup(json!({ "episodeCount": 0, "successCount": 0, "successRate": 0 }));
        let stated = json!({ "eval_episodes": 50, "eval_successes": 26, "success_rate": 0.52 });
        assert!(has_metrics(Some(&stated)));
        let note = no_evaluation_note(&zeros, true).unwrap();
        assert!(note.starts_with("no evaluation runs recorded on this server"));
        assert!(note.contains("validation metrics below"));
    }

    #[test]
    fn a_zero_rollup_without_validation_metrics_points_at_nothing() {
        assert_eq!(
            no_evaluation_note(&rollup(json!({})), false),
            Some("no evaluation runs recorded on this server")
        );
        assert!(!has_metrics(None));
        assert!(!has_metrics(Some(&json!({}))));
        assert!(!has_metrics(Some(&Value::Null)));
    }

    #[test]
    fn a_rollup_with_episodes_is_drawn_in_full() {
        let some = rollup(json!({ "episodeCount": 20, "successCount": 15, "successRate": 0.75 }));
        assert_eq!(no_evaluation_note(&some, true), None);
        assert_eq!(no_evaluation_note(&some, false), None);
    }
}
