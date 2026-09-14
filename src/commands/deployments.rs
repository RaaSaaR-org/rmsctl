use crate::cli::DeploymentsCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::models::Pagination;
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub async fn run(cmd: &DeploymentsCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        DeploymentsCommand::List { active } => list(client, ctx, *active).await,
        DeploymentsCommand::Show { id } => show(client, ctx, id).await,
        DeploymentsCommand::Metrics { id } => metrics(client, ctx, id).await,
        DeploymentsCommand::Promote { id } => promote(client, ctx, id).await,
        DeploymentsCommand::Rollback { id, reason } => {
            rollback(client, ctx, id, reason.as_deref()).await
        }
        DeploymentsCommand::Cancel { id } => cancel(client, ctx, id).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────
//
// `/deployments` and `/deployments/active` return the raw Deployment entity:
// the rollout counters (currentStage, deployedCount, …) exist only on the
// `/deployments/{id}` envelope, where the server computes them. Anything
// derived for a list row therefore has to come out of trafficPercentage +
// canaryConfig.stages, exactly as the detail handler does it.

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CanaryStage {
    #[serde(default)]
    percentage: Option<f64>,
    #[serde(default)]
    duration_minutes: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CanaryConfig {
    #[serde(default)]
    stages: Vec<CanaryStage>,
    #[serde(default)]
    success_threshold: Option<f64>,
    #[serde(default)]
    metrics_window: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RollbackThresholds {
    #[serde(default)]
    error_rate: Option<f64>,
    #[serde(default)]
    latency_p99: Option<f64>,
    #[serde(default)]
    failure_rate: Option<f64>,
}

/// The resolved model version the server attaches as `modelVersion` — both on
/// list rows and on the detail envelope. Carries the human name and semver, so
/// no view has to fall back to the bare UUID.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelVersionRef {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    deployment_status: Option<String>,
    #[serde(default)]
    model_type: Option<String>,
    #[serde(default)]
    artifact_uri: Option<String>,
}

impl ModelVersionRef {
    fn label(&self) -> Option<String> {
        let name = self.name.as_deref()?;
        Some(match self.version.as_deref() {
            Some(v) => format!("{name} ({v})"),
            None => name.to_string(),
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Deployment {
    id: String,
    #[serde(default)]
    model_version_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    strategy: Option<String>,
    #[serde(default)]
    traffic_percentage: Option<f64>,
    #[serde(default)]
    target_robot_types: Option<Vec<String>>,
    #[serde(default)]
    target_zones: Option<Vec<String>>,
    #[serde(default)]
    canary_config: Option<CanaryConfig>,
    #[serde(default)]
    rollback_thresholds: Option<RollbackThresholds>,
    #[serde(default)]
    deployed_robot_ids: Option<Vec<String>>,
    #[serde(default)]
    failed_robot_ids: Option<Vec<String>>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    model_version: Option<ModelVersionRef>,
}

impl Deployment {
    /// Stage as the server computes it: the highest canary stage whose
    /// percentage the rollout has already reached.
    fn stage(&self) -> Option<(usize, usize)> {
        let stages = &self.canary_config.as_ref()?.stages;
        if stages.is_empty() {
            return None;
        }
        let traffic = self.traffic_percentage.unwrap_or(0.0);
        let current = stages
            .iter()
            .filter(|s| s.percentage.is_some_and(|p| traffic >= p))
            .count();
        Some((current, stages.len()))
    }

    fn stage_label(&self) -> String {
        match self.stage() {
            Some((c, t)) => format!("{c}/{t}"),
            None => "-".to_string(),
        }
    }

    fn deployed(&self) -> usize {
        self.deployed_robot_ids.as_ref().map_or(0, Vec::len)
    }

    fn failed(&self) -> usize {
        self.failed_robot_ids.as_ref().map_or(0, Vec::len)
    }

    fn model_label(&self) -> String {
        self.model_version
            .as_ref()
            .and_then(ModelVersionRef::label)
            .or_else(|| self.model_version_id.clone())
            .unwrap_or_else(|| "-".to_string())
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeploymentsResponse {
    #[serde(default)]
    deployments: Vec<Deployment>,
    #[serde(default)]
    pagination: Option<Pagination>,
    /// `/deployments/active` sends a bare count instead of a pagination block.
    #[serde(default)]
    count: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RobotMetricsSummary {
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    total_requests: Option<u64>,
    #[serde(default)]
    error_count: Option<u64>,
    #[serde(default)]
    avg_latency_ms: Option<f64>,
    #[serde(default)]
    last_sample_time: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AggregatedMetrics {
    #[serde(default)]
    total_inferences: Option<u64>,
    #[serde(default)]
    successful_inferences: Option<u64>,
    #[serde(default)]
    error_rate: Option<f64>,
    #[serde(default)]
    latency_p50: Option<f64>,
    #[serde(default)]
    latency_p95: Option<f64>,
    #[serde(default)]
    latency_p99: Option<f64>,
    #[serde(default)]
    task_success_rate: Option<f64>,
    #[serde(default)]
    robot_count: Option<u32>,
    #[serde(default)]
    metrics_per_robot: Option<HashMap<String, RobotMetricsSummary>>,
}

/// `GET /deployments/{id}`. The five rollout counters and `nextStageTime` are
/// top-level siblings of `deployment`, not fields on it.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeploymentDetail {
    #[serde(default)]
    deployment: Option<Deployment>,
    #[serde(default)]
    model_version: Option<ModelVersionRef>,
    #[serde(default)]
    current_stage: Option<u32>,
    #[serde(default)]
    total_stages: Option<u32>,
    #[serde(default)]
    eligible_robot_count: Option<u32>,
    #[serde(default)]
    deployed_count: Option<u32>,
    #[serde(default)]
    failed_count: Option<u32>,
    /// Only present while a stage timer is running with a non-zero duration.
    #[serde(default)]
    next_stage_time: Option<String>,
    #[serde(default)]
    metrics: Option<AggregatedMetrics>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetricsResponse {
    #[serde(default)]
    deployment_id: Option<String>,
    #[serde(default)]
    metrics: Option<AggregatedMetrics>,
    #[serde(default)]
    is_monitoring: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MutationResponse {
    #[serde(default)]
    deployment: Option<Deployment>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

// ── Status vocabulary ───────────────────────────────────────────────────

/// `output::status_indicator` does not know the deployment vocabulary —
/// `production` (the success terminal state) and the three withdrawal states
/// all fall through to uncolored text there, which leaves the STATUS column
/// carrying no signal for this resource.
fn deploy_status(status: &str) -> String {
    let (icon, colored) = match status.to_lowercase().as_str() {
        "production" => ("●", status.green().bold()),
        "canary" | "deploying" => ("◐", status.truecolor(255, 103, 0).bold()),
        "rolling_back" => ("◐", status.yellow().bold()),
        "rolled_back" | "cancelled" | "canceled" => ("○", status.bright_black()),
        "failed" => ("●", status.red().bold()),
        "pending" => ("●", status.yellow().bold()),
        _ => return output::status_indicator(status),
    };
    format!("{} {}", icon.bright_black(), colored)
}

fn status_of(d: &Deployment) -> String {
    deploy_status(d.status.as_deref().unwrap_or("unknown"))
}

/// Server-side guards on the mutating routes, mirrored so the CLI can say why
/// a call is about to be refused instead of surfacing a bare 400.
fn allowed_states(action: &str) -> &'static [&'static str] {
    match action {
        "promote" => &["deploying", "canary"],
        "rollback" => &["deploying", "canary", "production"],
        _ => &["pending", "deploying", "canary"],
    }
}

// ── list ────────────────────────────────────────────────────────────────

async fn list(client: &ApiClient, ctx: &Ctx, active: bool) -> RmsResult<()> {
    // `/active` takes no query parameters: it always returns the full set of
    // in-flight rollouts.
    let (path, query) = if active {
        ("/deployments/active", Vec::new())
    } else {
        ("/deployments", ctx.paging())
    };

    if ctx.json {
        return output::json_out(&client.get_value_query(path, &query).await?);
    }

    let resp: DeploymentsResponse = client.get_query(path, &query).await?;

    output::section(
        "◈",
        if active {
            "Active Deployments"
        } else {
            "Deployments"
        },
    );

    if active && ctx.has_paging() {
        output::info("--active lists every in-flight rollout; the server ignores paging here.");
    }

    if resp.deployments.is_empty() {
        output::info(if active {
            "No deployments in flight."
        } else {
            "No deployments found."
        });
        println!();
        return Ok(());
    }

    // ID stays at full UUID width: the server resolves exact ids only, so a
    // clipped one cannot be pasted into `deployments show`.
    output::table_header(&[
        ("ID", output::ID_W),
        ("STATUS", 16),
        ("STAGE", 6),
        ("TRAFFIC", 8),
        ("ROBOTS", 7),
        ("MODEL", 30),
    ]);

    for d in &resp.deployments {
        let traffic = d
            .traffic_percentage
            .map(|t| format!("{t:.0}%"))
            .unwrap_or_else(|| "-".to_string());
        // An empty deployedRobotIds is a real zero; only a missing array is "-".
        let robots = if d.deployed_robot_ids.is_none() && d.failed_robot_ids.is_none() {
            "-".to_string()
        } else {
            match (d.deployed(), d.failed()) {
                (ok, 0) => ok.to_string(),
                (ok, bad) => format!("{ok}/{bad}✗"),
            }
        };

        println!(
            "    {} {} {} {} {} {}",
            output::pad(&d.id, output::ID_W),
            output::pad(&status_of(d), 16),
            output::pad(&d.stage_label(), 6),
            output::pad(&traffic, 8),
            output::pad(&robots, 7),
            output::truncate(&d.model_label(), 30)
        );
    }

    let pag = resp.pagination.as_ref();
    output::count_paged(
        if active {
            "active deployment(s)"
        } else {
            "deployment(s)"
        },
        resp.deployments.len(),
        pag.and_then(Pagination::total).or(resp.count),
        pag.and_then(Pagination::total_pages),
    );
    println!();
    Ok(())
}

// ── show ────────────────────────────────────────────────────────────────

async fn fetch_detail(client: &ApiClient, id: &str) -> RmsResult<DeploymentDetail> {
    let detail: DeploymentDetail = client.get(&format!("/deployments/{id}")).await?;
    if detail.deployment.is_none() {
        return Err(RmsError::Parse(format!(
            "the server returned no `deployment` object for {id}"
        )));
    }
    Ok(detail)
}

async fn show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let id = resolve_id(client, id).await?;
    let path = format!("/deployments/{id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let detail = fetch_detail(client, &id).await?;
    let d = detail.deployment.as_ref().expect("checked in fetch_detail");

    output::section("◈", &format!("Deployment {}", id.bold()));

    output::field("Status", &status_of(d));
    output::field("Strategy", d.strategy.as_deref().unwrap_or("-"));

    // The detail handler resolves modelVersionId through the model repository;
    // prefer that name and keep the UUID as a secondary row.
    let model = detail.model_version.as_ref().or(d.model_version.as_ref());
    match model.and_then(ModelVersionRef::label) {
        Some(label) => {
            output::field("Model", &label);
            if let Some(uuid) = model
                .and_then(|m| m.id.as_deref())
                .or(d.model_version_id.as_deref())
            {
                output::field_colored("Model version", uuid.bright_black());
            }
        }
        None => output::field(
            "Model version",
            d.model_version_id.as_deref().unwrap_or("-"),
        ),
    }
    if let Some(ms) = model.and_then(|m| m.deployment_status.as_deref()) {
        output::field("Model status", &output::status_indicator(ms));
    }

    if let Some(t) = d.traffic_percentage {
        output::field("Traffic", &format!("{t:.0}%"));
    }
    if let Some(types) = d.target_robot_types.as_ref().filter(|v| !v.is_empty()) {
        output::field("Robot types", &types.join(", "));
    }
    if let Some(zones) = d.target_zones.as_ref().filter(|v| !v.is_empty()) {
        output::field("Zones", &zones.join(", "));
    }
    if let Some(ts) = &d.created_at {
        output::field("Created", ts);
    }
    if let Some(ts) = &d.started_at {
        output::field("Started", ts);
    }
    if let Some(ts) = &d.completed_at {
        output::field("Completed", ts);
    }

    // ── Rollout ─────────────────────────────────────────────────────────
    let current = detail
        .current_stage
        .map(|c| c as usize)
        .or_else(|| d.stage().map(|(c, _)| c));
    let total = detail
        .total_stages
        .map(|t| t as usize)
        .or_else(|| d.stage().map(|(_, t)| t));

    output::subsection("Rollout");
    match (current, total) {
        (Some(c), Some(t)) if t > 0 => {
            output::field("Stage", &format!("{c} of {t}"));
            output::meter("Progress", c as f64, t as f64, "orange");
        }
        _ => output::field("Stage", "-"),
    }

    let eligible = detail
        .eligible_robot_count
        .map(|v| v as usize)
        .unwrap_or_else(|| d.deployed() + d.failed());
    let deployed = detail
        .deployed_count
        .map(|v| v as usize)
        .unwrap_or_else(|| d.deployed());
    let failed = detail
        .failed_count
        .map(|v| v as usize)
        .unwrap_or_else(|| d.failed());

    output::stat_line("│", "Eligible robots", &eligible.to_string(), "orange");
    output::stat_line(
        "│",
        "Deployed",
        &deployed.to_string(),
        if deployed > 0 { "green" } else { "dim" },
    );
    output::stat_line(
        if failed > 0 { "■" } else { "│" },
        "Failed",
        &failed.to_string(),
        if failed > 0 { "red" } else { "dim" },
    );
    if let Some(next) = &detail.next_stage_time {
        output::field("Next stage", next);
    }

    // ── Canary plan ─────────────────────────────────────────────────────
    if let Some(cfg) = &d.canary_config
        && !cfg.stages.is_empty()
    {
        output::subsection("Canary Stages");
        for (i, s) in cfg.stages.iter().enumerate() {
            let n = i + 1;
            let reached = current.is_some_and(|c| n <= c);
            let pct = s
                .percentage
                .map(|p| format!("{p:.0}% traffic"))
                .unwrap_or_else(|| "-".to_string());
            let hold = match s.duration_minutes {
                Some(m) if m > 0.0 => format!(" · hold {m:.0} min"),
                // durationMinutes 0 means the stage advances on demand.
                _ => " · manual advance".to_string(),
            };
            let marker = if reached { "●" } else { "○" };
            let line = format!("stage {n}  {pct}{hold}");
            println!(
                "    {} {} {}",
                "│".bright_black(),
                marker.bright_black(),
                if reached {
                    line.normal()
                } else {
                    line.bright_black()
                }
            );
        }
        if let Some(th) = cfg.success_threshold {
            output::field("Success gate", &format!("{:.0}%", th * 100.0));
        }
        if let Some(w) = cfg.metrics_window {
            output::field("Metrics window", &format!("{w:.0} min"));
        }
    }

    if let Some(t) = &d.rollback_thresholds {
        output::subsection("Auto-rollback Thresholds");
        if let Some(v) = t.error_rate {
            output::field("Error rate", &format!("> {:.1}%", v * 100.0));
        }
        if let Some(v) = t.failure_rate {
            output::field("Failure rate", &format!("> {:.1}%", v * 100.0));
        }
        if let Some(v) = t.latency_p99 {
            output::field("Latency p99", &format!("> {v:.0} ms"));
        }
    }

    if let Some(ids) = d.deployed_robot_ids.as_ref().filter(|v| !v.is_empty()) {
        output::subsection("Deployed Robots");
        for rid in ids.iter().take(10) {
            output::field("robot", rid);
        }
        if ids.len() > 10 {
            output::info(&format!("+ {} more (use --json)", ids.len() - 10));
        }
    }
    if let Some(ids) = d.failed_robot_ids.as_ref().filter(|v| !v.is_empty()) {
        output::subsection("Failed Robots");
        for rid in ids.iter().take(10) {
            output::field_colored("robot", rid.red());
        }
        if ids.len() > 10 {
            output::info(&format!("+ {} more (use --json)", ids.len() - 10));
        }
    }

    match &detail.metrics {
        Some(m) => {
            output::subsection("Live Metrics");
            render_metrics(m);
        }
        None => {
            output::subsection("Live Metrics");
            output::info("No metrics window open — see `deployments metrics <id>`.");
        }
    }

    println!();
    Ok(())
}

// ── metrics ─────────────────────────────────────────────────────────────

async fn metrics(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let id = resolve_id(client, id).await?;
    let path = format!("/deployments/{id}/metrics");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp: MetricsResponse = client.get(&path).await?;

    output::section("◈", &format!("Deployment Metrics {}", id.bold()));
    output::field_colored(
        "Monitoring",
        match resp.is_monitoring {
            Some(true) => "active".green().bold(),
            Some(false) => "off".bright_black(),
            None => "-".bright_black(),
        },
    );
    if let Some(did) = resp.deployment_id.as_deref().filter(|d| *d != id) {
        output::field("Deployment", did);
    }

    match &resp.metrics {
        Some(m) => render_metrics(m),
        None => {
            // The metrics service aggregates only over a live sample window:
            // once a rollout finishes (or before it starts) it returns null.
            output::info(
                "No samples in the current window — metrics exist only while a rollout is being monitored.",
            );
        }
    }

    println!();
    Ok(())
}

fn render_metrics(m: &AggregatedMetrics) {
    if let Some(total) = m.total_inferences {
        let ok = m.successful_inferences.unwrap_or(0);
        output::field("Inferences", &format!("{total} ({ok} ok)"));
    }
    if let Some(n) = m.robot_count {
        output::field("Robots", &n.to_string());
    }
    if let Some(rate) = m.error_rate {
        output::meter(
            "Error rate",
            rate * 100.0,
            100.0,
            if rate > 0.05 { "red" } else { "green" },
        );
    }
    if let Some(rate) = m.task_success_rate {
        output::meter(
            "Task success",
            rate * 100.0,
            100.0,
            if rate < 0.9 { "yellow" } else { "green" },
        );
    }
    let lat = [
        ("Latency p50", m.latency_p50),
        ("Latency p95", m.latency_p95),
        ("Latency p99", m.latency_p99),
    ];
    for (label, v) in lat {
        if let Some(v) = v {
            output::field(label, &format!("{v:.0} ms"));
        }
    }

    if let Some(per_robot) = m.metrics_per_robot.as_ref().filter(|p| !p.is_empty()) {
        output::subsection("Per Robot");
        output::table_header(&[
            ("ROBOT", output::ID_W),
            ("REQUESTS", 10),
            ("ERRORS", 8),
            ("AVG MS", 8),
        ]);
        let mut rows: Vec<(&String, &RobotMetricsSummary)> = per_robot.iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        for (key, r) in rows {
            let errors = r.error_count.unwrap_or(0);
            println!(
                "    {} {} {} {}",
                output::pad(r.robot_id.as_deref().unwrap_or(key), output::ID_W),
                output::pad(
                    &r.total_requests
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".into()),
                    10
                ),
                output::pad(
                    &if errors > 0 {
                        errors.to_string().red().to_string()
                    } else {
                        errors.to_string()
                    },
                    8
                ),
                r.avg_latency_ms
                    .map(|v| format!("{v:.0}"))
                    .unwrap_or_else(|| "-".into())
            );
        }
    }
}

// ── promote / rollback / cancel ─────────────────────────────────────────

/// Accept a UUID prefix as well as a full id.
///
/// The detail route does an exact lookup (no prefix matching), so resolve a
/// short id against the list before touching it — otherwise a copied-and-cut
/// id just 404s.
async fn resolve_id(client: &ApiClient, id: &str) -> RmsResult<String> {
    let id = id.trim();
    if id.is_empty() {
        return Err(RmsError::InvalidInput(
            "give a deployment ID (see `rmsctl deployments list`)".into(),
        ));
    }
    if id.len() == 36 {
        return Ok(id.to_string());
    }

    let resp: DeploymentsResponse = client
        .get_query("/deployments", &[("pageSize", "200")])
        .await?;
    let hits: Vec<&Deployment> = resp
        .deployments
        .iter()
        .filter(|d| d.id.starts_with(id))
        .collect();

    match hits.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => Err(RmsError::InvalidInput(format!(
            "no deployment starts with '{id}' — run `rmsctl deployments list`"
        ))),
        many => Err(RmsError::InvalidInput(format!(
            "'{id}' matches {} deployments; use the full ID",
            many.len()
        ))),
    }
}

/// Human description of where a rollout currently sits, for the confirm text.
fn where_it_is(detail: &DeploymentDetail, d: &Deployment) -> String {
    let stage = match (detail.current_stage, detail.total_stages) {
        (Some(c), Some(t)) => format!("stage {c}/{t}"),
        _ => format!("stage {}", d.stage_label()),
    };
    match d.traffic_percentage {
        Some(t) => format!("{stage} ({t:.0}% traffic)"),
        None => stage,
    }
}

/// Warn, before the gate, when the current status is one the server refuses.
fn warn_if_wrong_state(d: &Deployment, action: &str) {
    let status = d.status.as_deref().unwrap_or("unknown");
    if !allowed_states(action).contains(&status.to_lowercase().as_str()) {
        output::warn(&format!(
            "status is '{status}'; the server only accepts {action} for {}",
            allowed_states(action).join(", ")
        ));
    }
}

async fn promote(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let id = resolve_id(client, id).await?;
    let detail = fetch_detail(client, &id).await?;
    let d = detail.deployment.as_ref().expect("checked in fetch_detail");

    warn_if_wrong_state(d, "promote");

    if !output::confirm(
        &format!(
            "This will PROMOTE deployment {id} — model {} — from {} straight to PRODUCTION, putting it on 100% of eligible robots",
            d.model_label(),
            where_it_is(&detail, d)
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let resp = client
        .post_value(
            &format!("/deployments/{id}/promote"),
            &serde_json::json!({}),
        )
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    report_mutation(&resp, &format!("Deployment {id} promoted to production"));
    Ok(())
}

async fn rollback(client: &ApiClient, ctx: &Ctx, id: &str, reason: Option<&str>) -> RmsResult<()> {
    // The route rejects a rollback without a reason (400); ask for it here so
    // the failure names the flag.
    let reason = match reason.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => r.to_string(),
        None => {
            return Err(RmsError::InvalidInput(
                "--reason is required: the server rejects a rollback without one".into(),
            ));
        }
    };

    let id = resolve_id(client, id).await?;
    let detail = fetch_detail(client, &id).await?;
    let d = detail.deployment.as_ref().expect("checked in fetch_detail");

    warn_if_wrong_state(d, "rollback");

    let reverting = match d.deployed() {
        0 => "no robots are carrying it yet".to_string(),
        n => format!("{n} robot(s) revert to their previous model"),
    };

    if !output::confirm(
        &format!(
            "This will ROLL BACK deployment {id} — model {} — out of {}: {reverting}; reason: {reason}",
            d.model_label(),
            where_it_is(&detail, d)
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let resp = client
        .post_value(
            &format!("/deployments/{id}/rollback"),
            &serde_json::json!({ "reason": reason }),
        )
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    report_mutation(&resp, &format!("Deployment {id} rolled back"));
    Ok(())
}

async fn cancel(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let id = resolve_id(client, id).await?;
    let detail = fetch_detail(client, &id).await?;
    let d = detail.deployment.as_ref().expect("checked in fetch_detail");

    warn_if_wrong_state(d, "cancel");

    if !output::confirm(
        &format!(
            "This will CANCEL deployment {id} — model {} — at {}; the rollout stops before the next stage and cannot be resumed",
            d.model_label(),
            where_it_is(&detail, d)
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let resp = client
        .post_value(&format!("/deployments/{id}/cancel"), &serde_json::json!({}))
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    report_mutation(&resp, &format!("Deployment {id} cancelled"));
    Ok(())
}

/// All three mutating routes answer `{deployment, message}`; show the new
/// status so the operator does not have to re-run `show`.
fn report_mutation(resp: &serde_json::Value, headline: &str) {
    output::success(headline);
    let parsed: Option<MutationResponse> = serde_json::from_value(resp.clone()).ok();
    if let Some(p) = &parsed {
        if let Some(msg) = &p.message {
            output::info(msg);
        }
        if let Some(d) = &p.deployment {
            output::field("Status", &status_of(d));
            if let Some(t) = d.traffic_percentage {
                output::field("Traffic", &format!("{t:.0}%"));
            }
            output::field("Stage", &d.stage_label());
            if let Some(reason) = &p.reason {
                output::field("Reason", reason);
            }
        }
    }
    println!();
}
