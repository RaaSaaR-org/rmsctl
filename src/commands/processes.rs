use crate::cli::ProcessesCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::models::Pagination;
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub async fn run(cmd: &ProcessesCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        ProcessesCommand::List => list(client, ctx).await,
        ProcessesCommand::Show { id } => show(client, ctx, id).await,
        ProcessesCommand::Instances { status } => instances(client, ctx, status.as_deref()).await,
        ProcessesCommand::Instance { id } => instance(client, ctx, id).await,
        ProcessesCommand::Tasks => tasks(client, ctx).await,
        ProcessesCommand::TaskQueue => task_queue(client, ctx).await,
        ProcessesCommand::Start { id } => start(client, ctx, id).await,
        ProcessesCommand::Pause { instance_id } => {
            lifecycle(client, ctx, instance_id, Lifecycle::Pause).await
        }
        ProcessesCommand::Resume { instance_id } => {
            lifecycle(client, ctx, instance_id, Lifecycle::Resume).await
        }
        ProcessesCommand::Cancel { instance_id } => {
            lifecycle(client, ctx, instance_id, Lifecycle::Cancel).await
        }
        ProcessesCommand::Retry { instance_id } => {
            lifecycle(client, ctx, instance_id, Lifecycle::Retry).await
        }
    }
}

// ── Response shapes ─────────────────────────────────────────────────────
//
// Every list route in this group answers `{data: [...], pagination: {...}}`
// and spells the page size `limit`, so the list commands use
// `ctx.paging_limit()`. The two detail routes and the queue-stats route
// answer a bare object with no wrapper.

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StepTemplate {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    order: Option<u32>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    action_type: Option<String>,
    #[serde(default)]
    action_config: Option<serde_json::Value>,
    #[serde(default)]
    required_capabilities: Option<Vec<String>>,
    #[serde(default)]
    estimated_duration_minutes: Option<f64>,
    #[serde(default)]
    depends_on_step_ids: Option<Vec<String>>,
    #[serde(default)]
    can_run_parallel: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessDefinition {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    step_templates: Vec<StepTemplate>,
    #[serde(default)]
    required_capabilities: Option<Vec<String>>,
    #[serde(default)]
    estimated_duration_minutes: Option<f64>,
    #[serde(default)]
    max_concurrent_instances: Option<u32>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    trigger_type: Option<String>,
    #[serde(default)]
    cron_expression: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    next_run_at: Option<String>,
    #[serde(default)]
    last_scheduled_run_at: Option<String>,
    #[serde(default)]
    created_by: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DefinitionsResponse {
    #[serde(default)]
    data: Vec<ProcessDefinition>,
    #[serde(default)]
    pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StepInstance {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    order: Option<u32>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    action_type: Option<String>,
    #[serde(default)]
    action_config: Option<serde_json::Value>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    assigned_robot_id: Option<String>,
    #[serde(default)]
    robot_task_id: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    retry_count: Option<u32>,
    #[serde(default)]
    max_retries: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessInstance {
    id: String,
    #[serde(default)]
    process_definition_id: Option<String>,
    #[serde(default)]
    process_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    steps: Vec<StepInstance>,
    #[serde(default)]
    current_step_index: Option<u32>,
    #[serde(default)]
    progress: Option<f64>,
    #[serde(default)]
    preferred_robot_ids: Option<Vec<String>>,
    #[serde(default)]
    assigned_robot_ids: Option<Vec<String>>,
    #[serde(default)]
    scheduled_at: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
    #[serde(default)]
    estimated_completion_at: Option<String>,
    #[serde(default)]
    input_data: Option<serde_json::Value>,
    #[serde(default)]
    output_data: Option<serde_json::Value>,
    #[serde(default)]
    error_message: Option<String>,
    #[serde(default)]
    created_by: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstancesResponse {
    #[serde(default)]
    data: Vec<ProcessInstance>,
    #[serde(default)]
    pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RobotTask {
    id: String,
    #[serde(default)]
    process_instance_id: Option<String>,
    #[serde(default)]
    source: Option<String>,
    // Null until the TaskDistributor picks a robot, so an unassigned task is a
    // normal state rather than a missing field.
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    action_type: Option<String>,
    #[serde(default)]
    instruction: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    retry_count: Option<u32>,
    #[serde(default)]
    max_retries: Option<u32>,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TasksResponse {
    #[serde(default)]
    data: Vec<RobotTask>,
    #[serde(default)]
    pagination: Option<Pagination>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PriorityCounts {
    #[serde(default)]
    critical: Option<u32>,
    #[serde(default)]
    high: Option<u32>,
    #[serde(default)]
    normal: Option<u32>,
    #[serde(default)]
    low: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RobotQueue {
    #[serde(default)]
    queued: Option<u32>,
    #[serde(default)]
    executing: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueueStats {
    #[serde(default)]
    pending: Option<u32>,
    #[serde(default)]
    assigned: Option<u32>,
    #[serde(default)]
    executing: Option<u32>,
    #[serde(default)]
    completed: Option<u32>,
    #[serde(default)]
    failed: Option<u32>,
    #[serde(default)]
    by_priority: Option<PriorityCounts>,
    // BTreeMap, not HashMap: the per-robot rollup must print in the same order
    // on every run.
    #[serde(default)]
    by_robot: Option<BTreeMap<String, RobotQueue>>,
}

// ── Shared rendering ────────────────────────────────────────────────────

/// Priority as a glyph plus colour.
///
/// `status_indicator` does not know the process priority vocabulary
/// (low/normal/high/critical) and `severity_indicator` upper-cases everything,
/// so this group renders its own.
fn priority_cell(priority: Option<&str>) -> String {
    let p = priority.unwrap_or("-");
    match p.to_lowercase().as_str() {
        "critical" => format!("{} {}", "▲".red(), p.red().bold()),
        "high" => format!("{} {}", "▲".yellow(), p.yellow().bold()),
        "normal" => format!("{} {}", "·".bright_black(), p.normal()),
        "low" => format!("{} {}", "·".bright_black(), p.bright_black()),
        _ => format!("{} {}", "·".bright_black(), p.normal()),
    }
}

/// One-line view of a free-form `actionConfig` / `inputData` object.
///
/// Those objects have a different shape per action type
/// (`{location:{zone,floor}, zoneName}` for a move, `{object, grip}` for a
/// pickup), so only the top-level scalars are rendered; `--json` carries the
/// nested remainder.
fn scalar_summary(value: Option<&serde_json::Value>) -> Option<String> {
    let obj = value?.as_object()?;
    let parts: Vec<String> = obj
        .iter()
        .filter_map(|(k, v)| match v {
            serde_json::Value::String(s) => Some(format!("{k}={s}")),
            serde_json::Value::Number(n) => Some(format!("{k}={n}")),
            serde_json::Value::Bool(b) => Some(format!("{k}={b}")),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("  "))
    }
}

fn minutes(m: Option<f64>) -> Option<String> {
    m.map(|m| format!("{m:.0} min"))
}

fn robot_list(ids: Option<&Vec<String>>) -> Option<String> {
    let ids = ids?;
    if ids.is_empty() {
        None
    } else {
        Some(ids.join(", "))
    }
}

// ── list ────────────────────────────────────────────────────────────────

/// Trigger column: the cron expression is more informative than the literal
/// `triggerType`, and a scheduled definition with `enabled: false` never fires,
/// which is the one thing an operator must not miss.
fn trigger_cell(p: &ProcessDefinition) -> String {
    let kind = p.trigger_type.as_deref().unwrap_or("-");
    match (p.cron_expression.as_deref(), p.enabled) {
        (Some(cron), Some(false)) => format!("{} {}", cron, "(disabled)".yellow()),
        (Some(cron), _) => cron.to_string(),
        (None, Some(false)) if kind == "scheduled" => format!("{} {}", kind, "(disabled)".yellow()),
        (None, _) => kind.to_string(),
    }
}

async fn list(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let query = ctx.paging_limit();

    if ctx.json {
        return output::json_out(&client.get_value_query("/processes", &query).await?);
    }

    let resp: DefinitionsResponse = client.get_query("/processes", &query).await?;

    output::section("◈", "Process Definitions");

    if resp.data.is_empty() {
        output::info("No process definitions.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("ID", output::ID_W),
        ("NAME", 24),
        ("STATUS", 14),
        ("STEPS", 5),
        ("TRIGGER", 24),
    ]);
    for p in &resp.data {
        println!(
            "    {} {} {} {} {}",
            output::pad(&p.id, output::ID_W),
            output::pad(&output::truncate(p.name.as_deref().unwrap_or("-"), 24), 24),
            output::pad(
                &output::status_indicator(p.status.as_deref().unwrap_or("-")),
                14
            ),
            output::pad(&p.step_templates.len().to_string(), 5),
            trigger_cell(p)
        );
    }

    // Only a `ready` definition can be started; draft ones need publishing
    // first, so say so rather than letting `processes start` 400.
    let draft = resp
        .data
        .iter()
        .filter(|p| p.status.as_deref() == Some("draft"))
        .count();
    if draft > 0 {
        println!();
        output::info(&format!(
            "{draft} definition(s) are draft — only a 'ready' definition can be started."
        ));
    }

    let pag = resp.pagination.as_ref();
    output::count_paged(
        "process definition(s)",
        resp.data.len(),
        pag.and_then(|p| p.total()),
        pag.and_then(|p| p.total_pages()),
    );
    println!();
    Ok(())
}

// ── show ────────────────────────────────────────────────────────────────

async fn show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/processes/{id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let p: ProcessDefinition = client.get(&path).await?;

    output::section(
        "◈",
        &format!("Process: {}", p.name.as_deref().unwrap_or(&p.id).bold()),
    );

    output::field("ID", &p.id);
    output::field(
        "Status",
        &output::status_indicator(p.status.as_deref().unwrap_or("-")),
    );
    if let Some(v) = p.version {
        output::field("Version", &v.to_string());
    }
    if let Some(d) = &p.description {
        output::field("Description", d);
    }
    output::field("Trigger", p.trigger_type.as_deref().unwrap_or("-"));
    if let Some(cron) = &p.cron_expression {
        output::field("Cron", cron);
    }
    if p.trigger_type.as_deref() == Some("scheduled") {
        output::field_colored(
            "Enabled",
            match p.enabled {
                Some(true) => "yes".green(),
                Some(false) => "no — this schedule will not fire".yellow(),
                None => "-".bright_black(),
            },
        );
    }
    if let Some(next) = &p.next_run_at {
        output::field("Next run", next);
    }
    if let Some(last) = &p.last_scheduled_run_at {
        output::field("Last run", last);
    }
    if let Some(est) = minutes(p.estimated_duration_minutes) {
        output::field("Estimated", &est);
    }
    if let Some(max) = p.max_concurrent_instances {
        output::field("Max parallel", &max.to_string());
    }
    if let Some(caps) = robot_list(p.required_capabilities.as_ref()) {
        output::field("Capabilities", &caps);
    }
    if let Some(tags) = robot_list(p.tags.as_ref()) {
        output::field("Tags", &tags);
    }
    if let Some(by) = &p.created_by {
        output::field("Created by", by);
    }
    if let Some(at) = &p.created_at {
        output::field("Created", at);
    }
    if let Some(at) = &p.updated_at {
        output::field("Updated", at);
    }

    output::subsection(&format!("Steps ({})", p.step_templates.len()));
    if p.step_templates.is_empty() {
        output::info("This definition has no steps.");
    } else {
        output::table_header(&[("#", 3), ("NAME", 34), ("ACTION", 18), ("EST", 8)]);
        for s in &p.step_templates {
            println!(
                "    {} {} {} {}",
                output::pad(
                    &s.order.map(|o| o.to_string()).unwrap_or_else(|| "-".into()),
                    3
                ),
                output::pad(&output::truncate(s.name.as_deref().unwrap_or("-"), 34), 34),
                output::pad(
                    &output::truncate(s.action_type.as_deref().unwrap_or("-"), 18),
                    18
                ),
                minutes(s.estimated_duration_minutes).unwrap_or_else(|| "-".into())
            );
            if let Some(cfg) = scalar_summary(s.action_config.as_ref()) {
                println!("        {}", output::truncate(&cfg, 90).bright_black());
            }
            if s.can_run_parallel == Some(true) {
                println!("        {}", "can run in parallel".bright_black());
            }
        }
    }

    println!();
    Ok(())
}

// ── instances ───────────────────────────────────────────────────────────

const INSTANCE_STATUSES: [&str; 7] = [
    "pending",
    "queued",
    "in_progress",
    "paused",
    "completed",
    "failed",
    "cancelled",
];

/// Validate `--status` locally.
///
/// The server does not: an unknown status simply matches nothing and returns
/// `{data: [], pagination: {total: 0}}`, so `--status running` would report an
/// empty fleet instead of an error. Map the obvious synonyms, reject the rest.
fn normalize_instance_status(status: &str) -> RmsResult<String> {
    let s = status.trim().to_lowercase().replace('-', "_");
    let mapped = match s.as_str() {
        "running" | "active" | "in_progress" => "in_progress",
        "canceled" | "cancelled" => "cancelled",
        other => other,
    };
    if INSTANCE_STATUSES.contains(&mapped) {
        Ok(mapped.to_string())
    } else {
        Err(RmsError::InvalidInput(format!(
            "--status {status} is not a process instance status; expected one of: {}",
            INSTANCE_STATUSES.join(", ")
        )))
    }
}

/// Robot column for an instance.
///
/// `assignedRobotIds` is what is actually executing; when it is empty the
/// preferred robot is shown dimmed, because a preference is not an assignment.
fn instance_robot_cell(i: &ProcessInstance) -> String {
    match robot_list(i.assigned_robot_ids.as_ref()) {
        Some(assigned) => output::truncate(&assigned, 18),
        None => match robot_list(i.preferred_robot_ids.as_ref()) {
            Some(pref) => output::truncate(&pref, 18).bright_black().to_string(),
            None => "-".bright_black().to_string(),
        },
    }
}

async fn instances(client: &ApiClient, ctx: &Ctx, status: Option<&str>) -> RmsResult<()> {
    let status = status.map(normalize_instance_status).transpose()?;
    let mut own: Vec<(&'static str, String)> = Vec::new();
    if let Some(s) = &status {
        own.push(("status", s.clone()));
    }
    let query = ctx.with_paging_limit(own);

    if ctx.json {
        return output::json_out(
            &client
                .get_value_query("/processes/instances/list", &query)
                .await?,
        );
    }

    let resp: InstancesResponse = client
        .get_query("/processes/instances/list", &query)
        .await?;

    output::section(
        "◈",
        &match &status {
            // The normalized value, not what was typed: `--status running`
            // queries `in_progress` and the header must say so.
            Some(s) => format!("Process Instances ({s})"),
            None => "Process Instances".to_string(),
        },
    );

    if resp.data.is_empty() {
        output::info("No process instances match.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("ID", output::ID_W),
        ("PROCESS", 22),
        ("STATUS", 14),
        ("PRIORITY", 11),
        ("PROG", 5),
        ("ROBOT", 18),
    ]);
    for i in &resp.data {
        println!(
            "    {} {} {} {} {} {}",
            output::pad(&i.id, output::ID_W),
            output::pad(
                &output::truncate(i.process_name.as_deref().unwrap_or("-"), 22),
                22
            ),
            output::pad(
                &output::status_indicator(i.status.as_deref().unwrap_or("-")),
                14
            ),
            output::pad(&priority_cell(i.priority.as_deref()), 11),
            output::pad(
                &i.progress
                    .map(|p| format!("{p:.0}%"))
                    .unwrap_or_else(|| "-".into()),
                5
            ),
            instance_robot_cell(i)
        );
    }

    let stuck = resp
        .data
        .iter()
        .filter(|i| matches!(i.status.as_deref(), Some("failed") | Some("cancelled")))
        .count();
    let paused = resp
        .data
        .iter()
        .filter(|i| i.status.as_deref() == Some("paused"))
        .count();
    if stuck > 0 || paused > 0 {
        println!();
        if stuck > 0 {
            output::info(&format!(
                "{stuck} failed/cancelled on this page — re-run with: rmsctl processes retry <instance-id>"
            ));
        }
        if paused > 0 {
            output::info(&format!(
                "{paused} paused on this page — continue with: rmsctl processes resume <instance-id>"
            ));
        }
    }

    let pag = resp.pagination.as_ref();
    output::count_paged(
        "process instance(s)",
        resp.data.len(),
        pag.and_then(|p| p.total()),
        pag.and_then(|p| p.total_pages()),
    );
    println!();
    Ok(())
}

// ── instance ────────────────────────────────────────────────────────────

async fn instance(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/processes/instances/{id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let i: ProcessInstance = client.get(&path).await?;

    output::section(
        "◈",
        &format!(
            "Instance: {}",
            i.process_name.as_deref().unwrap_or(&i.id).bold()
        ),
    );

    output::field("ID", &i.id);
    if let Some(def) = &i.process_definition_id {
        output::field("Definition", def);
    }
    let status = i.status.as_deref().unwrap_or("-");
    output::field("Status", &output::status_indicator(status));
    output::field("Priority", &priority_cell(i.priority.as_deref()));
    if let Some(d) = &i.description {
        output::field("Description", d);
    }
    if let Some(progress) = i.progress {
        let color = match status {
            "failed" | "cancelled" => "red",
            "paused" => "yellow",
            "completed" => "green",
            _ => "orange",
        };
        output::meter("Progress", progress, 100.0, color);
    }
    if !i.steps.is_empty() {
        let at = i.current_step_index.unwrap_or(0) as usize;
        output::field("Step", &format!("{} of {}", at + 1, i.steps.len()));
    }
    if let Some(assigned) = robot_list(i.assigned_robot_ids.as_ref()) {
        output::field("Assigned", &assigned);
    }
    if let Some(pref) = robot_list(i.preferred_robot_ids.as_ref()) {
        output::field("Preferred", &pref);
    }
    if let Some(at) = &i.scheduled_at {
        output::field("Scheduled", at);
    }
    if let Some(at) = &i.started_at {
        output::field("Started", at);
    }
    if let Some(at) = &i.estimated_completion_at {
        output::field("ETA", at);
    }
    if let Some(at) = &i.completed_at {
        output::field("Completed", at);
    }
    if let Some(err) = &i.error_message {
        output::field_colored("Error", err.red());
    }
    if let Some(input) = scalar_summary(i.input_data.as_ref()) {
        output::field("Input", &output::truncate(&input, 80));
    }
    if let Some(out) = scalar_summary(i.output_data.as_ref()) {
        output::field("Output", &output::truncate(&out, 80));
    }
    if let Some(by) = &i.created_by {
        output::field("Created by", by);
    }
    if let Some(at) = &i.created_at {
        output::field("Created", at);
    }

    output::subsection(&format!("Steps ({})", i.steps.len()));
    if i.steps.is_empty() {
        output::info("This instance has no steps.");
    } else {
        output::table_header(&[
            ("#", 4),
            ("NAME", 30),
            ("ACTION", 16),
            ("STATUS", 14),
            ("ROBOT", 18),
            ("TRY", 5),
        ]);
        let current = i.current_step_index.unwrap_or(u32::MAX);
        for (idx, s) in i.steps.iter().enumerate() {
            // Mark the step the instance is sitting on, but only while it is
            // actually executing — a cancelled instance still reports
            // currentStepIndex 0.
            let marker = if idx as u32 == current && status == "in_progress" {
                format!("{} {}", "›".truecolor(255, 103, 0), idx)
            } else {
                format!("  {idx}")
            };
            println!(
                "    {} {} {} {} {} {}/{}",
                output::pad(&marker, 4),
                output::pad(&output::truncate(s.name.as_deref().unwrap_or("-"), 30), 30),
                output::pad(
                    &output::truncate(s.action_type.as_deref().unwrap_or("-"), 16),
                    16
                ),
                output::pad(
                    &output::status_indicator(s.status.as_deref().unwrap_or("-")),
                    14
                ),
                output::pad(
                    &output::truncate(s.assigned_robot_id.as_deref().unwrap_or("-"), 18),
                    18
                ),
                s.retry_count.unwrap_or(0),
                s.max_retries.unwrap_or(0)
            );
            if let Some(err) = &s.error {
                println!("        {}", output::truncate(err, 90).red());
            }
        }
    }

    println!();
    Ok(())
}

// ── tasks ───────────────────────────────────────────────────────────────

async fn tasks(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let query = ctx.paging_limit();

    if ctx.json {
        return output::json_out(
            &client
                .get_value_query("/processes/tasks/list", &query)
                .await?,
        );
    }

    let resp: TasksResponse = client.get_query("/processes/tasks/list", &query).await?;

    output::section("◈", "Robot Tasks");

    if resp.data.is_empty() {
        output::info("No robot tasks. Steps only become tasks once an instance is running.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("ID", output::ID_W),
        ("ROBOT", 18),
        ("STATUS", 14),
        ("PRIORITY", 11),
        ("INSTRUCTION", 26),
    ]);
    for t in &resp.data {
        println!(
            "    {} {} {} {} {}",
            output::pad(&t.id, output::ID_W),
            output::pad(
                &match t.robot_id.as_deref() {
                    Some(r) => output::truncate(r, 18),
                    None => "unassigned".bright_black().to_string(),
                },
                18
            ),
            output::pad(
                &output::status_indicator(t.status.as_deref().unwrap_or("-")),
                14
            ),
            output::pad(&priority_cell(t.priority.as_deref()), 11),
            output::truncate(
                t.instruction
                    .as_deref()
                    .or(t.action_type.as_deref())
                    .unwrap_or("-"),
                26
            )
        );
        if let Some(err) = &t.error {
            println!("        {}", output::truncate(err, 90).red());
        }
    }

    let pag = resp.pagination.as_ref();
    output::count_paged(
        "robot task(s)",
        resp.data.len(),
        pag.and_then(|p| p.total()),
        pag.and_then(|p| p.total_pages()),
    );
    println!();
    Ok(())
}

// ── task-queue ──────────────────────────────────────────────────────────

async fn task_queue(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let path = "/processes/tasks/queue/stats";

    if ctx.json {
        return output::json_out(&client.get_value(path).await?);
    }

    let s: QueueStats = client.get(path).await?;

    output::section("◈", "Task Queue");

    let pending = s.pending.unwrap_or(0);
    let assigned = s.assigned.unwrap_or(0);
    let executing = s.executing.unwrap_or(0);
    let failed = s.failed.unwrap_or(0);

    output::stat_line(
        if pending + assigned > 0 { "■" } else { "│" },
        "Queue depth",
        &(pending + assigned).to_string(),
        if pending + assigned > 0 {
            "orange"
        } else {
            "dim"
        },
    );
    output::stat_line("│", "Pending", &pending.to_string(), "dim");
    output::stat_line("│", "Assigned", &assigned.to_string(), "dim");
    output::stat_line(
        "│",
        "Executing",
        &executing.to_string(),
        if executing > 0 { "orange" } else { "dim" },
    );
    output::stat_line(
        "│",
        "Completed",
        &s.completed.unwrap_or(0).to_string(),
        "green",
    );
    output::stat_line(
        if failed > 0 { "■" } else { "│" },
        "Failed",
        &failed.to_string(),
        if failed > 0 { "red" } else { "dim" },
    );

    // byPriority counts only pending + assigned work, so it is the queue's
    // shape rather than a lifetime total.
    if let Some(p) = &s.by_priority {
        output::subsection("Waiting by priority");
        for (label, count) in [
            ("critical", p.critical),
            ("high", p.high),
            ("normal", p.normal),
            ("low", p.low),
        ] {
            let n = count.unwrap_or(0);
            output::stat_line(
                "│",
                label,
                &n.to_string(),
                match (label, n) {
                    (_, 0) => "dim",
                    ("critical", _) => "red",
                    ("high", _) => "yellow",
                    _ => "orange",
                },
            );
        }
    }

    output::subsection("Per robot");
    match s.by_robot.as_ref().filter(|m| !m.is_empty()) {
        Some(robots) => {
            output::table_header(&[("ROBOT", 26), ("QUEUED", 8), ("EXECUTING", 10)]);
            for (robot, q) in robots {
                println!(
                    "    {} {} {}",
                    output::pad(&output::truncate(robot, 26), 26),
                    output::pad(&q.queued.unwrap_or(0).to_string(), 8),
                    q.executing.unwrap_or(0)
                );
            }
        }
        None => output::info("No robot is holding queued or executing work."),
    }

    println!();
    Ok(())
}

// ── start ───────────────────────────────────────────────────────────────

async fn start(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    // Read the definition first: it gives the confirm prompt a name and step
    // count, and it catches the one precondition the server hides behind a
    // generic 400 ("Definition may not exist or not be ready").
    let def: ProcessDefinition = client.get(&format!("/processes/{id}")).await?;
    let name = def.name.as_deref().unwrap_or("unnamed process");

    if def.status.as_deref() != Some("ready") {
        return Err(RmsError::InvalidInput(format!(
            "process '{name}' is {}; only a 'ready' definition can be started",
            def.status.as_deref().unwrap_or("in an unknown state")
        )));
    }

    // Empty body: the server defaults to normal priority and autoStart=true,
    // so the first step is dispatched to a robot immediately.
    if !output::confirm(
        &format!(
            "This will START process '{name}' ({id}) — {} step(s), first one dispatched to a robot now",
            def.step_templates.len()
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let resp = client
        .post_value(&format!("/processes/{id}/start"), &serde_json::json!({}))
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    output::success(&format!("Started '{name}'"));
    if let Some(instance_id) = resp.get("id").and_then(|v| v.as_str()) {
        output::field("Instance", instance_id);
        if let Some(status) = resp.get("status").and_then(|v| v.as_str()) {
            output::field("Status", &output::status_indicator(status));
        }
        output::info(&format!(
            "Follow it: rmsctl processes instance {instance_id}"
        ));
    }
    println!();
    Ok(())
}

// ── pause / resume / cancel / retry ─────────────────────────────────────

#[derive(Clone, Copy)]
enum Lifecycle {
    Pause,
    Resume,
    Cancel,
    Retry,
}

impl Lifecycle {
    /// Doubles as the last path segment: the routes are
    /// `PUT /processes/instances/:id/{pause,resume,cancel,retry}`.
    fn verb(&self) -> &'static str {
        match self {
            Lifecycle::Pause => "pause",
            Lifecycle::Resume => "resume",
            Lifecycle::Cancel => "cancel",
            Lifecycle::Retry => "retry",
        }
    }

    fn done(&self) -> &'static str {
        match self {
            Lifecycle::Pause => "Paused",
            Lifecycle::Resume => "Resumed",
            Lifecycle::Cancel => "Cancelled",
            Lifecycle::Retry => "Retrying",
        }
    }

    /// States the server's own guard accepts
    /// (ProcessManager.{pauseProcess,resumeProcess,cancelProcess,retryProcess}).
    fn allowed(&self) -> &'static [&'static str] {
        match self {
            Lifecycle::Pause => &["in_progress"],
            Lifecycle::Resume => &["paused"],
            Lifecycle::Cancel => &["pending", "queued", "in_progress", "paused"],
            Lifecycle::Retry => &["failed", "cancelled"],
        }
    }

    fn effect(&self, what: &str) -> String {
        match self {
            // Pausing flips the instance status only: a step already handed to
            // a robot runs to completion, nothing further is dispatched.
            Lifecycle::Pause => format!(
                "This will PAUSE {what} — the step already dispatched finishes, no further steps are handed out"
            ),
            Lifecycle::Resume => {
                format!("This will RESUME {what} — the next step is dispatched to a robot")
            }
            Lifecycle::Cancel => format!(
                "This will CANCEL {what} — every pending step and its robot tasks are cancelled"
            ),
            Lifecycle::Retry => format!(
                "This will RETRY {what} — steps from the first failed one are reset and execution restarts now"
            ),
        }
    }
}

async fn lifecycle(
    client: &ApiClient,
    ctx: &Ctx,
    instance_id: &str,
    act: Lifecycle,
) -> RmsResult<()> {
    let path = format!("/processes/instances/{instance_id}");
    let inst: ProcessInstance = client.get(&path).await?;
    let name = inst.process_name.as_deref().unwrap_or("unnamed process");

    // The server refuses an illegal transition with a vague 400 ("Cannot pause
    // process. It may not be running."), so check here where the accepted
    // states can be named.
    if let Some(current) = inst.status.as_deref()
        && !act.allowed().contains(&current.to_lowercase().as_str())
    {
        return Err(RmsError::InvalidInput(format!(
            "instance {instance_id} ('{name}') is {current}; {} only accepts {}",
            act.verb(),
            act.allowed().join(" or ")
        )));
    }

    let what = format!("process instance '{name}' ({instance_id})");
    if !output::confirm(&act.effect(&what), ctx.yes) {
        return Ok(());
    }

    // These four are PUT, not POST, and read no body.
    let resp = client
        .put_value(&format!("{path}/{}", act.verb()), &serde_json::json!({}))
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    output::success(&format!("{} '{name}'", act.done()));
    if let Some(status) = resp.get("status").and_then(|v| v.as_str()) {
        output::field("Status", &output::status_indicator(status));
    }
    if let Some(progress) = resp.get("progress").and_then(|v| v.as_f64()) {
        output::field("Progress", &format!("{progress:.0}%"));
    }
    println!();
    Ok(())
}
