use crate::cli::ApprovalsCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub async fn run(cmd: &ApprovalsCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        ApprovalsCommand::List => list(client, ctx).await,
        ApprovalsCommand::Show { id } => show(client, ctx, id).await,
        ApprovalsCommand::Pending { role } => pending(client, ctx, role.as_deref()).await,
        ApprovalsCommand::Overdue => overdue(client, ctx).await,
        ApprovalsCommand::Nearing => nearing(client, ctx).await,
        ApprovalsCommand::Decide {
            id,
            step_id,
            decision,
            notes,
        } => decide(client, ctx, id, step_id, decision, notes.as_deref()).await,
        ApprovalsCommand::Cancel { id } => cancel(client, ctx, id).await,
        ApprovalsCommand::Escalate { id } => escalate(client, ctx, id).await,
        ApprovalsCommand::Metrics => metrics(client, ctx).await,
        ApprovalsCommand::SlaReport => sla_report(client, ctx).await,
    }
}

/// The four decisions `POST /approvals/:id/steps/:stepId/decide` accepts
/// (approval.routes.ts) — anything else is a 400 from the server.
const DECISIONS: [&str; 4] = ["approve", "reject", "defer", "request_info"];

/// The roles `GET /approvals/pending/role/:role` accepts; the server 400s on
/// anything else rather than returning an empty list.
const ROLES: [&str; 4] = ["supervisor", "manager", "safety_officer", "admin"];

// ── Response shapes ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalStep {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    step_order: Option<u32>,
    #[serde(default)]
    approver_role: Option<String>,
    #[serde(default)]
    assigned_to: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    decision: Option<String>,
    #[serde(default)]
    decided_by: Option<String>,
    #[serde(default)]
    decision_notes: Option<String>,
    #[serde(default)]
    active_engagement: Option<bool>,
    #[serde(default)]
    review_duration_sec: Option<f64>,
    #[serde(default)]
    competence_verified: Option<bool>,
    #[serde(default)]
    decided_at: Option<String>,
}

impl ApprovalStep {
    /// A step still waiting for a human. `awaiting` is the step actually in
    /// front of an approver; `pending` steps sit behind it in a sequential
    /// chain and cannot be decided yet.
    fn is_open(&self) -> bool {
        matches!(
            self.status.as_deref(),
            Some("awaiting") | Some("pending") | None
        )
    }

    fn label(&self) -> String {
        let role = self.approver_role.as_deref().unwrap_or("approver");
        match self.step_order {
            Some(o) => format!("step {} ({role})", o + 1),
            None => format!("step ({role})"),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalChain {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    required_steps: Option<u32>,
    #[serde(default)]
    current_step_index: Option<u32>,
    #[serde(default)]
    is_sequential: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RollbackStep {
    #[serde(default)]
    order: Option<u32>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    automated: Option<bool>,
    #[serde(default)]
    estimated_duration_minutes: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RollbackPlan {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    steps: Vec<RollbackStep>,
    #[serde(default)]
    test_procedure: Option<String>,
    #[serde(default)]
    approved_by: Option<String>,
    #[serde(default)]
    approved_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusChange {
    #[serde(default)]
    from_status: Option<String>,
    #[serde(default)]
    to_status: Option<String>,
    #[serde(default)]
    changed_by: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalRequest {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    request_number: Option<String>,
    #[serde(default)]
    entity_type: Option<String>,
    #[serde(default)]
    entity_id: Option<String>,
    /// Free-form per entity type (safety parameters, package versions, shift
    /// windows) — never modelled, rendered key by key.
    #[serde(default)]
    entity_data: Option<serde_json::Value>,
    #[serde(default)]
    approval_type: Option<String>,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    affected_user_id: Option<String>,
    #[serde(default)]
    affected_robot_id: Option<String>,
    #[serde(default)]
    sla_hours: Option<f64>,
    #[serde(default)]
    sla_deadline: Option<String>,
    #[serde(default)]
    escalated_at: Option<String>,
    #[serde(default)]
    escalation_level: Option<u32>,
    #[serde(default)]
    requested_by: Option<String>,
    #[serde(default)]
    request_reason: Option<String>,
    #[serde(default)]
    blocks_execution: Option<bool>,
    #[serde(default)]
    rollback_plan: Option<RollbackPlan>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
    #[serde(default)]
    chain: Option<ApprovalChain>,
    #[serde(default)]
    steps: Vec<ApprovalStep>,
    #[serde(default)]
    status_history: Vec<StatusChange>,
}

impl ApprovalRequest {
    fn handle(&self, fallback_id: &str) -> String {
        self.request_number
            .clone()
            .or_else(|| self.id.clone())
            .unwrap_or_else(|| fallback_id.to_string())
    }

    /// One line naming the request for a confirm prompt.
    fn describe(&self, fallback_id: &str) -> String {
        let mut s = self.handle(fallback_id);
        if let Some(t) = &self.entity_type {
            s.push_str(&format!(" — {t}"));
        }
        if let Some(p) = &self.priority {
            s.push_str(&format!(" (priority {p})"));
        }
        if let Some(r) = &self.affected_robot_id {
            s.push_str(&format!(", robot {r}"));
        }
        s
    }

    /// The step an approver can act on next.
    fn active_step(&self) -> Option<&ApprovalStep> {
        self.steps
            .iter()
            .find(|s| s.status.as_deref() == Some("awaiting"))
            .or_else(|| self.steps.iter().find(|s| s.is_open()))
    }
}

/// `GET /approvals` — paging lives flat on the envelope, not in a nested
/// `pagination` object like the rest of the API.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalListResponse {
    #[serde(default)]
    requests: Vec<ApprovalRequest>,
    #[serde(default)]
    total: Option<u32>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalMetrics {
    #[serde(default)]
    total_requests: Option<u32>,
    #[serde(default)]
    pending_requests: Option<u32>,
    #[serde(default)]
    in_progress_requests: Option<u32>,
    #[serde(default)]
    overdue_requests: Option<u32>,
    #[serde(default)]
    nearing_deadline_requests: Option<u32>,
    #[serde(default)]
    avg_response_time_hours: Option<f64>,
    #[serde(default)]
    sla_compliance_rate: Option<f64>,
    #[serde(default)]
    requests_by_type: Option<BTreeMap<String, i64>>,
    #[serde(default)]
    requests_by_status: Option<BTreeMap<String, i64>>,
    #[serde(default)]
    approvals_by_role: Option<BTreeMap<String, i64>>,
    #[serde(default)]
    active_contests: Option<u32>,
    #[serde(default)]
    pending_viewpoints: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SlaEntityRow {
    #[serde(default)]
    total: Option<i64>,
    #[serde(default)]
    on_time: Option<i64>,
    #[serde(default)]
    overdue: Option<i64>,
    #[serde(default)]
    avg_hours: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SlaReport {
    #[serde(default)]
    total_requests: Option<u32>,
    #[serde(default)]
    on_time_requests: Option<u32>,
    #[serde(default)]
    overdue_requests: Option<u32>,
    #[serde(default)]
    nearing_deadline: Option<u32>,
    #[serde(default)]
    compliance_rate: Option<f64>,
    #[serde(default)]
    avg_response_time_hours: Option<f64>,
    #[serde(default)]
    escalated_count: Option<u32>,
    #[serde(default)]
    by_entity_type: Option<BTreeMap<String, SlaEntityRow>>,
}

// ── Shared rendering ────────────────────────────────────────────────────

fn priority_tag(priority: Option<&str>) -> ColoredString {
    match priority {
        Some(p) if p.eq_ignore_ascii_case("critical") => p.to_uppercase().red().bold(),
        Some(p) if p.eq_ignore_ascii_case("high") => p.truecolor(255, 103, 0).bold(),
        Some(p) if p.eq_ignore_ascii_case("low") => p.bright_black(),
        Some(p) => p.normal(),
        None => "-".bright_black(),
    }
}

fn num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// Scalar-ish rendering of one `entityData` / rollback value. Nested objects
/// and arrays collapse to compact JSON so a detail view stays one line per key.
fn compact(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "-".to_string(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Array(items) => items.iter().map(compact).collect::<Vec<_>>().join(", "),
        serde_json::Value::Object(_) => {
            output::truncate(&serde_json::to_string(v).unwrap_or_default(), 70)
        }
    }
}

/// `packageVersion` -> `Package version`, so arbitrary server keys read as
/// labels without a hand-written table per entity type.
fn humanize(key: &str) -> String {
    let mut out = String::new();
    for (i, c) in key.chars().enumerate() {
        if c == '_' || c == '-' {
            out.push(' ');
        } else if c.is_uppercase() && i > 0 {
            out.push(' ');
            out.push(c.to_ascii_lowercase());
        } else if i == 0 {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn render_object(v: &serde_json::Value) {
    match v {
        serde_json::Value::Object(map) if !map.is_empty() => {
            for (k, val) in map {
                output::field(&humanize(k), &compact(val));
            }
        }
        serde_json::Value::Null => {}
        other => output::field("Value", &compact(other)),
    }
}

/// Block form for the actionable queues (pending / overdue / nearing), where
/// the operator needs the full request id *and* the step id to act.
fn render_queue_item(r: &ApprovalRequest) {
    println!();
    println!("    {} {}", "│".bright_black(), r.handle("-").bold());
    output::field("ID", r.id.as_deref().unwrap_or("-"));
    output::field("Entity", r.entity_type.as_deref().unwrap_or("-"));
    output::field_colored("Priority", priority_tag(r.priority.as_deref()));
    output::field(
        "Status",
        &output::status_indicator(r.status.as_deref().unwrap_or("unknown")),
    );
    if let Some(deadline) = &r.sla_deadline {
        let window = r.sla_hours.map(|h| format!(" ({}h SLA)", num(h)));
        output::field(
            "Deadline",
            &format!("{deadline}{}", window.unwrap_or_default()),
        );
    }
    if let Some(robot) = &r.affected_robot_id {
        output::field("Robot", robot);
    }
    if let Some(step) = r.active_step() {
        output::field(
            "Waiting on",
            &format!(
                "{} — step {}",
                step.approver_role.as_deref().unwrap_or("-"),
                step.id.as_deref().unwrap_or("-")
            ),
        );
    }
    if let Some(reason) = &r.request_reason {
        output::field("Reason", &output::truncate(reason, 66));
    }
}

/// Paste-ready next action, so a queue does not dead-end in two more lookups.
fn hint_decide(r: &ApprovalRequest) {
    if let (Some(id), Some(step)) = (r.id.as_deref(), r.active_step())
        && let Some(step_id) = step.id.as_deref()
    {
        output::info(&format!(
            "rmsctl approvals decide {id} {step_id} --decision approve"
        ));
    }
}

// ── list ────────────────────────────────────────────────────────────────

async fn list(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let query = ctx.paging_limit();

    if ctx.json {
        return output::json_out(&client.get_value_query("/approvals", &query).await?);
    }

    let resp: ApprovalListResponse = client.get_query("/approvals", &query).await?;

    output::section("◈", "Approval Requests");

    if resp.requests.is_empty() {
        output::info("No approval requests.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("ID", output::ID_W),
        ("REQUEST", 16),
        ("ENTITY", 26),
        ("PRI", 9),
        ("STATUS", 14),
    ]);
    for r in &resp.requests {
        println!(
            "    {} {} {} {} {}",
            output::pad(r.id.as_deref().unwrap_or("-"), output::ID_W),
            output::pad(r.request_number.as_deref().unwrap_or("-"), 16),
            output::pad(
                &output::truncate(r.entity_type.as_deref().unwrap_or("-"), 26),
                26
            ),
            output::pad(&priority_tag(r.priority.as_deref()).to_string(), 9),
            output::pad(
                &output::status_indicator(r.status.as_deref().unwrap_or("unknown")),
                14
            ),
        );
    }

    // The envelope reports total and limit but no page count; derive it so the
    // footer can point at --page.
    let total_pages = match (resp.total, resp.limit.filter(|l| *l > 0)) {
        (Some(total), Some(limit)) => Some(total.div_ceil(limit)),
        _ => None,
    };
    output::count_paged(
        "approval request(s)",
        resp.requests.len(),
        resp.total,
        total_pages,
    );
    println!();
    Ok(())
}

// ── show ────────────────────────────────────────────────────────────────

async fn show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/approvals/{id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let r: ApprovalRequest = client.get(&path).await?;

    output::section("◈", &format!("Approval {}", r.handle(id).bold()));

    output::field("ID", r.id.as_deref().unwrap_or(id));
    output::field(
        "Status",
        &output::status_indicator(r.status.as_deref().unwrap_or("unknown")),
    );
    output::field_colored("Priority", priority_tag(r.priority.as_deref()));
    output::field("Entity type", r.entity_type.as_deref().unwrap_or("-"));
    output::field("Entity", r.entity_id.as_deref().unwrap_or("-"));
    if let Some(t) = &r.approval_type {
        output::field("Approval type", t);
    }
    if let Some(robot) = &r.affected_robot_id {
        output::field("Robot", robot);
    }
    if let Some(user) = &r.affected_user_id {
        output::field("Affected user", user);
    }
    output::field("Requested by", r.requested_by.as_deref().unwrap_or("-"));
    if let Some(reason) = &r.request_reason {
        output::field("Reason", reason);
    }
    if let Some(h) = r.sla_hours {
        output::field(
            "SLA",
            &format!(
                "{}h — due {}",
                num(h),
                r.sla_deadline.as_deref().unwrap_or("-")
            ),
        );
    } else if let Some(d) = &r.sla_deadline {
        output::field("Deadline", d);
    }
    if r.blocks_execution == Some(true) {
        output::field_colored("Blocks", "blocks execution".yellow());
    }
    if let Some(level) = r.escalation_level.filter(|l| *l > 0) {
        output::field_colored(
            "Escalation",
            format!(
                "level {level}{}",
                r.escalated_at
                    .as_deref()
                    .map(|a| format!(" at {a}"))
                    .unwrap_or_default()
            )
            .yellow(),
        );
    }
    if let Some(c) = &r.created_at {
        output::field("Created", c);
    }
    if let Some(c) = &r.completed_at {
        output::field("Completed", c);
    }

    if let Some(data) = r.entity_data.as_ref().filter(|v| !v.is_null()) {
        output::subsection("Entity data");
        render_object(data);
    }

    if let Some(chain) = &r.chain {
        output::subsection("Chain");
        if let Some(n) = &chain.name {
            output::field("Name", n);
        }
        // currentStepIndex is zero-based on the wire; show it as a human step
        // number against requiredSteps.
        if let Some(required) = chain.required_steps {
            let at = chain.current_step_index.map(|i| i + 1).unwrap_or(0);
            output::field("Progress", &format!("step {at} of {required}"));
        }
        output::field(
            "Order",
            match chain.is_sequential {
                Some(true) => "sequential",
                Some(false) => "parallel",
                None => "-",
            },
        );
    }

    if !r.steps.is_empty() {
        output::subsection("Steps");
        output::table_header(&[
            ("#", 3),
            ("STEP ID", output::ID_W),
            ("ROLE", 16),
            ("STATUS", 14),
            ("DECIDED BY", 16),
        ]);
        for s in &r.steps {
            println!(
                "    {} {} {} {} {}",
                output::pad(
                    &s.step_order
                        .map(|o| (o + 1).to_string())
                        .unwrap_or_default(),
                    3
                ),
                output::pad(s.id.as_deref().unwrap_or("-"), output::ID_W),
                output::pad(s.approver_role.as_deref().unwrap_or("-"), 16),
                output::pad(
                    &output::status_indicator(s.status.as_deref().unwrap_or("unknown")),
                    14
                ),
                output::pad(
                    &output::truncate(s.decided_by.as_deref().unwrap_or("-"), 16),
                    16
                ),
            );
            if let Some(notes) = &s.decision_notes {
                println!("        {} {}", "└".bright_black(), notes.bright_black());
            }
            // Art. 14 "meaningful engagement" evidence: a rubber-stamped step
            // is one with no review time and no competence check.
            if s.decided_at.is_some() {
                println!(
                    "        {} decided {}  review {}  engaged {}  competence {}",
                    "·".bright_black(),
                    s.decided_at.as_deref().unwrap_or("-"),
                    s.review_duration_sec
                        .map(|v| format!("{}s", num(v)))
                        .unwrap_or_else(|| "-".into()),
                    output::bool_indicator(s.active_engagement),
                    output::bool_indicator(s.competence_verified),
                );
            }
        }
    }

    if let Some(plan) = &r.rollback_plan {
        output::subsection("Rollback plan");
        if let Some(d) = &plan.description {
            output::field("Description", d);
        }
        for step in &plan.steps {
            let mins = step
                .estimated_duration_minutes
                .map(|m| format!(" [{}min]", num(m)))
                .unwrap_or_default();
            output::field(
                &format!(
                    "Step {}",
                    step.order.map(|o| o.to_string()).unwrap_or_default()
                ),
                &format!(
                    "{}{} {}",
                    step.action.as_deref().unwrap_or("-"),
                    mins,
                    if step.automated == Some(true) {
                        "automated".green()
                    } else {
                        "manual".yellow()
                    }
                ),
            );
        }
        if let Some(t) = &plan.test_procedure {
            output::field("Test", t);
        }
        if let Some(by) = &plan.approved_by {
            output::field(
                "Plan approved",
                &format!("{by} {}", plan.approved_at.as_deref().unwrap_or("")),
            );
        }
    }

    if !r.status_history.is_empty() {
        output::subsection("History");
        for h in &r.status_history {
            println!(
                "    {} {}  {} → {}  {}",
                "·".bright_black(),
                output::pad(h.timestamp.as_deref().unwrap_or("-"), 26),
                h.from_status.as_deref().unwrap_or("-").bright_black(),
                output::status_indicator(h.to_status.as_deref().unwrap_or("-")),
                h.reason.as_deref().unwrap_or("").bright_black(),
            );
            if let Some(by) = &h.changed_by {
                println!("        {} by {}", "└".bright_black(), by.bright_black());
            }
        }
    }

    println!();
    hint_decide(&r);
    println!();
    Ok(())
}

// ── pending / overdue / nearing ─────────────────────────────────────────

async fn pending(client: &ApiClient, ctx: &Ctx, role: Option<&str>) -> RmsResult<()> {
    let (path, title) = match role {
        Some(role) => {
            let role = role.trim().to_lowercase();
            if !ROLES.contains(&role.as_str()) {
                return Err(RmsError::InvalidInput(format!(
                    "--role must be one of: {}",
                    ROLES.join(", ")
                )));
            }
            (
                format!("/approvals/pending/role/{role}"),
                format!("Pending for role: {role}"),
            )
        }
        None => (
            "/approvals/pending/me".to_string(),
            "Pending for me".to_string(),
        ),
    };

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    // Both pending routes return a bare array, not a {requests:[...]} envelope.
    let requests: Vec<ApprovalRequest> = client.get(&path).await?;

    output::section("◈", &title);

    if requests.is_empty() {
        output::info("Nothing waiting.");
        if role.is_none() {
            // The server still resolves "me" from a userId query param it
            // defaults to 'system', so a tokenless dev session sees nothing.
            output::info("pending/me resolves the caller server-side; try --role safety_officer.");
        }
        println!();
        return Ok(());
    }

    for r in &requests {
        render_queue_item(r);
    }

    output::count_paged("approval(s) waiting", requests.len(), None, None);
    if let Some(first) = requests.first() {
        hint_decide(first);
    }
    println!();
    Ok(())
}

async fn overdue(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/approvals/overdue").await?);
    }

    let requests: Vec<ApprovalRequest> = client.get("/approvals/overdue").await?;

    output::section("◈", "Overdue Approvals");

    if requests.is_empty() {
        output::info("No approvals past their SLA deadline.");
        println!();
        return Ok(());
    }

    output::warn(&format!(
        "{} approval(s) past the SLA deadline",
        requests.len()
    ));
    for r in &requests {
        render_queue_item(r);
    }

    output::count_paged("overdue approval(s)", requests.len(), None, None);
    if let Some(first) = requests.first() {
        hint_decide(first);
    }
    println!();
    Ok(())
}

async fn nearing(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/approvals/nearing-deadline").await?);
    }

    let requests: Vec<ApprovalRequest> = client.get("/approvals/nearing-deadline").await?;

    output::section("◈", "Approvals Nearing Deadline");

    if requests.is_empty() {
        // The window is a server-side default (withinHours=4) and the CLI has
        // no flag for it, so say what "nearing" means here.
        output::info("Nothing due within the server's 4-hour window.");
        println!();
        return Ok(());
    }

    for r in &requests {
        render_queue_item(r);
    }

    output::count_paged("approval(s) due soon", requests.len(), None, None);
    if let Some(first) = requests.first() {
        hint_decide(first);
    }
    println!();
    Ok(())
}

// ── decide / cancel / escalate ──────────────────────────────────────────

/// Best-effort read of the request so a confirm prompt can name what is being
/// decided. A failure here must not block the action: fall back to bare ids.
async fn peek(client: &ApiClient, id: &str) -> Option<ApprovalRequest> {
    client
        .get::<ApprovalRequest>(&format!("/approvals/{id}"))
        .await
        .ok()
}

async fn decide(
    client: &ApiClient,
    ctx: &Ctx,
    id: &str,
    step_id: &str,
    decision: &str,
    notes: Option<&str>,
) -> RmsResult<()> {
    let decision = decision.trim().to_lowercase();
    if !DECISIONS.contains(&decision.as_str()) {
        return Err(RmsError::InvalidInput(format!(
            "--decision must be one of: {}",
            DECISIONS.join(", ")
        )));
    }
    let notes = notes.map(str::trim).filter(|n| !n.is_empty());

    let request = peek(client, id).await;

    // Catch a mistyped step id before it becomes a POST: the ids are UUIDs and
    // the server would answer with a generic 500 from the service layer.
    let step = match &request {
        Some(r) if !r.steps.is_empty() => {
            match r.steps.iter().find(|s| s.id.as_deref() == Some(step_id)) {
                Some(s) => Some(s),
                None => {
                    let known: Vec<String> = r
                        .steps
                        .iter()
                        .map(|s| format!("{} ({})", s.id.as_deref().unwrap_or("-"), s.label()))
                        .collect();
                    return Err(RmsError::InvalidInput(format!(
                        "step {step_id} is not part of {}. Steps: {}",
                        r.handle(id),
                        known.join(", ")
                    )));
                }
            }
        }
        _ => None,
    };

    let mut what = format!(
        "{} on {}",
        decision.to_uppercase(),
        request
            .as_ref()
            .map(|r| r.describe(id))
            .unwrap_or_else(|| format!("request {id}"))
    );
    if let Some(s) = step {
        what = format!(
            "{} on {} of {}",
            decision.to_uppercase(),
            s.label(),
            request
                .as_ref()
                .map(|r| r.describe(id))
                .unwrap_or_else(|| format!("request {id}"))
        );
        if !s.is_open() {
            output::warn(&format!(
                "{} is already {} (decision: {})",
                s.label(),
                s.status.as_deref().unwrap_or("decided"),
                s.decision.as_deref().unwrap_or("-")
            ));
        }
    }

    if let Some(data) = request
        .as_ref()
        .and_then(|r| r.entity_data.as_ref())
        .filter(|v| !v.is_null())
    {
        output::section("◈", "Deciding");
        render_object(data);
        if let Some(reason) = request.as_ref().and_then(|r| r.request_reason.as_deref()) {
            output::field("Reason", &output::truncate(reason, 66));
        }
    }

    if notes.is_none() && matches!(decision.as_str(), "reject" | "request_info") {
        output::warn("No --notes given; a rejection with no recorded reason is hard to audit.");
    }
    output::info("The decision is attributed to your authenticated session user.");

    let detail = notes.map(|n| format!(" — notes: {n}")).unwrap_or_default();
    if !output::confirm(&format!("This will record {what}{detail}"), ctx.yes) {
        return Ok(());
    }

    // The server reads `decisionNotes` (the CLI flag is --notes) and refuses a
    // body `decidedBy` outright — the actor comes from the session.
    let mut body = serde_json::json!({ "decision": decision });
    if let Some(n) = notes {
        body["decisionNotes"] = serde_json::Value::String(n.to_string());
    }

    let resp = client
        .post_value(&format!("/approvals/{id}/steps/{step_id}/decide"), &body)
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    output::success(&format!("Recorded {decision} on step {step_id}"));
    if let Some(status) = resp.get("status").and_then(|s| s.as_str()) {
        output::field("Request status", &output::status_indicator(status));
    }
    println!();
    Ok(())
}

async fn cancel(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    // `reason` is mandatory (400 otherwise) but the CLI exposes no --reason for
    // cancel, so record a fixed, honest marker instead of a fake reason.
    const REASON: &str = "Cancelled via rmsctl";

    let request = peek(client, id).await;
    let what = request
        .as_ref()
        .map(|r| r.describe(id))
        .unwrap_or_else(|| format!("request {id}"));

    if !output::confirm(
        &format!(
            "This will CANCEL approval {what} — recorded reason: \"{REASON}\". Any work it blocks stays blocked"
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let resp = client
        .post_value(
            &format!("/approvals/{id}/cancel"),
            &serde_json::json!({ "reason": REASON }),
        )
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    output::success(&format!("Cancelled {what}"));
    println!();
    Ok(())
}

async fn escalate(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let request = peek(client, id).await;
    let what = request
        .as_ref()
        .map(|r| r.describe(id))
        .unwrap_or_else(|| format!("request {id}"));
    let level = request
        .as_ref()
        .and_then(|r| r.escalation_level)
        .unwrap_or(0);

    if !output::confirm(
        &format!(
            "This will ESCALATE approval {what} from level {level} to level {}, notifying the next approver tier",
            level + 1
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    // `reason` is optional on this route; nothing in the CLI supplies one.
    let resp = client
        .post_value(&format!("/approvals/{id}/escalate"), &serde_json::json!({}))
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    output::success(&format!("Escalated {what}"));
    if let Some(l) = resp.get("escalationLevel").and_then(|l| l.as_i64()) {
        output::field("Escalation level", &l.to_string());
    }
    println!();
    Ok(())
}

// ── metrics / sla-report ────────────────────────────────────────────────

/// A `{name: count}` breakdown, busiest first, with empty buckets dimmed
/// rather than hidden.
fn render_counts(title: &str, counts: &BTreeMap<String, i64>) {
    output::subsection(title);
    let mut rows: Vec<(&String, &i64)> = counts.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (name, count) in rows {
        output::stat_line(
            "│",
            name,
            &count.to_string(),
            if *count > 0 { "orange" } else { "dim" },
        );
    }
}

fn rate_color(rate: f64) -> &'static str {
    if rate >= 95.0 {
        "green"
    } else if rate >= 80.0 {
        "yellow"
    } else {
        "red"
    }
}

async fn metrics(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/approvals/metrics").await?);
    }

    let m: ApprovalMetrics = client.get("/approvals/metrics").await?;

    output::section("◈", "Approval Metrics");

    output::stat_line(
        "│",
        "Total requests",
        &m.total_requests.unwrap_or(0).to_string(),
        "orange",
    );
    output::stat_line(
        "│",
        "Pending",
        &m.pending_requests.unwrap_or(0).to_string(),
        if m.pending_requests.unwrap_or(0) > 0 {
            "yellow"
        } else {
            "dim"
        },
    );
    output::stat_line(
        "│",
        "In progress",
        &m.in_progress_requests.unwrap_or(0).to_string(),
        "dim",
    );
    let overdue = m.overdue_requests.unwrap_or(0);
    output::stat_line(
        if overdue > 0 { "■" } else { "│" },
        "Overdue",
        &overdue.to_string(),
        if overdue > 0 { "red" } else { "dim" },
    );
    output::stat_line(
        "│",
        "Nearing deadline",
        &m.nearing_deadline_requests.unwrap_or(0).to_string(),
        if m.nearing_deadline_requests.unwrap_or(0) > 0 {
            "yellow"
        } else {
            "dim"
        },
    );
    if let Some(avg) = m.avg_response_time_hours {
        output::stat_line("│", "Avg response", &format!("{}h", num(avg)), "dim");
    }
    output::stat_line(
        "│",
        "Active contests",
        &m.active_contests.unwrap_or(0).to_string(),
        if m.active_contests.unwrap_or(0) > 0 {
            "yellow"
        } else {
            "dim"
        },
    );
    output::stat_line(
        "│",
        "Pending viewpoints",
        &m.pending_viewpoints.unwrap_or(0).to_string(),
        "dim",
    );

    if let Some(rate) = m.sla_compliance_rate {
        println!();
        output::meter("SLA compliance", rate, 100.0, rate_color(rate));
        if overdue > 0 && rate >= 100.0 {
            // The service counts only *completed* requests in the rate, so a
            // still-open overdue request does not dent it.
            output::info(
                "Rate covers completed requests only — overdue items are not counted yet.",
            );
        }
    }

    if let Some(by) = &m.requests_by_status {
        render_counts("By status", by);
    }
    if let Some(by) = &m.requests_by_type {
        render_counts("By entity type", by);
    }
    if let Some(by) = &m.approvals_by_role {
        render_counts("Decisions by role", by);
    }

    println!();
    Ok(())
}

async fn sla_report(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/approvals/sla-report").await?);
    }

    let r: SlaReport = client.get("/approvals/sla-report").await?;

    output::section("◈", "Approval SLA Report");

    output::stat_line(
        "│",
        "Total requests",
        &r.total_requests.unwrap_or(0).to_string(),
        "orange",
    );
    output::stat_line(
        "│",
        "On time",
        &r.on_time_requests.unwrap_or(0).to_string(),
        "green",
    );
    let overdue = r.overdue_requests.unwrap_or(0);
    output::stat_line(
        if overdue > 0 { "■" } else { "│" },
        "Overdue",
        &overdue.to_string(),
        if overdue > 0 { "red" } else { "dim" },
    );
    output::stat_line(
        "│",
        "Nearing deadline",
        &r.nearing_deadline.unwrap_or(0).to_string(),
        if r.nearing_deadline.unwrap_or(0) > 0 {
            "yellow"
        } else {
            "dim"
        },
    );
    output::stat_line(
        "│",
        "Escalated",
        &r.escalated_count.unwrap_or(0).to_string(),
        if r.escalated_count.unwrap_or(0) > 0 {
            "yellow"
        } else {
            "dim"
        },
    );
    if let Some(avg) = r.avg_response_time_hours {
        output::stat_line("│", "Avg response", &format!("{}h", num(avg)), "dim");
    }

    if let Some(rate) = r.compliance_rate {
        println!();
        output::meter("Compliance", rate, 100.0, rate_color(rate));
    }

    if let Some(by) = &r.by_entity_type {
        // Most entity types have no requests at all; listing them all buries
        // the two rows that matter.
        let rows: Vec<(&String, &SlaEntityRow)> = by
            .iter()
            .filter(|(_, v)| v.total.unwrap_or(0) > 0)
            .collect();

        output::subsection("By entity type");
        if rows.is_empty() {
            output::info("No requests recorded for any entity type.");
        } else {
            output::table_header(&[
                ("ENTITY TYPE", 34),
                ("TOTAL", 7),
                ("ON TIME", 9),
                ("OVERDUE", 9),
                ("AVG H", 7),
            ]);
            for (name, row) in &rows {
                let od = row.overdue.unwrap_or(0);
                println!(
                    "    {} {} {} {} {}",
                    output::pad(&output::truncate(name, 34), 34),
                    output::pad(&row.total.unwrap_or(0).to_string(), 7),
                    output::pad(&row.on_time.unwrap_or(0).to_string(), 9),
                    output::pad(
                        &if od > 0 {
                            od.to_string().red().bold().to_string()
                        } else {
                            od.to_string()
                        },
                        9
                    ),
                    output::pad(&num(row.avg_hours.unwrap_or(0.0)), 7),
                );
            }
            let empty = by.len() - rows.len();
            if empty > 0 {
                output::info(&format!("{empty} entity type(s) with no requests hidden."));
            }
        }
    }

    println!();
    Ok(())
}
