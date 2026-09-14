use crate::cli::ComplianceCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::RmsResult;
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub async fn run(cmd: &ComplianceCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        ComplianceCommand::Dashboard => dashboard(client, ctx).await,
        ComplianceCommand::Deadlines => deadlines(client, ctx).await,
        ComplianceCommand::Gaps { summary } => {
            if *summary {
                gaps_summary(client, ctx).await
            } else {
                gaps(client, ctx).await
            }
        }
        ComplianceCommand::Metrics => metrics(client, ctx).await,
        ComplianceCommand::Documents => documents(client, ctx).await,
        ComplianceCommand::Training { summary } => {
            if *summary {
                training_summary(client, ctx).await
            } else {
                training(client, ctx).await
            }
        }
        ComplianceCommand::Inspections { summary } => {
            if *summary {
                inspections_summary(client, ctx).await
            } else {
                inspections(client, ctx).await
            }
        }
        ComplianceCommand::RiskAssessments => risk_assessments(client, ctx).await,
        ComplianceCommand::Activity => activity(client, ctx).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────
//
// Two different route files back this group: everything under
// /compliance/tracker/ is the tracker service (named-key envelopes, or a flat
// object for /dashboard), while /compliance/metrics is the compliance-log
// service and is flat. Nothing here shares a wrapper with anything else.

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Dashboard {
    #[serde(default)]
    overall_score: Option<f64>,
    #[serde(default)]
    framework_scores: Vec<FrameworkScore>,
    #[serde(default)]
    alerts: Option<ComplianceAlerts>,
    #[serde(default)]
    last_updated: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FrameworkScore {
    #[serde(default)]
    framework: Option<String>,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    open_items: Option<u32>,
    #[serde(default)]
    total_items: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComplianceAlerts {
    #[serde(default)]
    critical_gaps: Option<u32>,
    #[serde(default)]
    expiring_documents: Option<u32>,
    #[serde(default)]
    overdue_training: Option<u32>,
    #[serde(default)]
    overdue_inspections: Option<u32>,
    #[serde(default)]
    upcoming_deadlines: Option<u32>,
    #[serde(default)]
    pending_risk_reviews: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Deadline {
    #[serde(default)]
    framework: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    deadline: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    days_until_deadline: Option<i64>,
    // JSON-encoded strings, not arrays — see json_list_len.
    #[serde(default)]
    requirements: Option<serde_json::Value>,
    #[serde(default)]
    completed_requirements: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeadlinesResponse {
    #[serde(default)]
    deadlines: Vec<Deadline>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Gap {
    #[serde(default)]
    framework: Option<String>,
    #[serde(default)]
    requirement: Option<String>,
    #[serde(default)]
    article_reference: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    due_date: Option<String>,
    #[serde(default)]
    assigned_to: Option<String>,
    #[serde(default)]
    estimated_effort: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GapsResponse {
    #[serde(default)]
    gaps: Vec<Gap>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GapSummaryEntry {
    #[serde(default)]
    total: Option<u32>,
    #[serde(default)]
    critical: Option<u32>,
    #[serde(default)]
    high: Option<u32>,
    #[serde(default)]
    medium: Option<u32>,
    #[serde(default)]
    low: Option<u32>,
}

/// `summary` is keyed by framework, so a map rather than a list. BTreeMap keeps
/// the frameworks in a stable order — a HashMap would reshuffle every run.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GapSummaryResponse {
    #[serde(default)]
    summary: BTreeMap<String, GapSummaryEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Metrics {
    #[serde(default)]
    total_logs: Option<u64>,
    #[serde(default)]
    event_type_counts: Vec<EventTypeCount>,
    #[serde(default)]
    severity_counts: Option<SeverityCounts>,
    #[serde(default)]
    unique_sessions: Option<u64>,
    #[serde(default)]
    unique_robots: Option<u64>,
    #[serde(default)]
    date_range: Option<DateRange>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventTypeCount {
    #[serde(default)]
    event_type: Option<String>,
    #[serde(default)]
    count: Option<u64>,
    #[serde(default)]
    last_occurrence: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SeverityCounts {
    #[serde(default)]
    debug: Option<u64>,
    #[serde(default)]
    info: Option<u64>,
    #[serde(default)]
    warning: Option<u64>,
    #[serde(default)]
    error: Option<u64>,
    #[serde(default)]
    critical: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DateRange {
    #[serde(default)]
    start: Option<String>,
    #[serde(default)]
    end: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpiringDocument {
    #[serde(default)]
    provider_name: Option<String>,
    #[serde(default)]
    model_version: Option<String>,
    #[serde(default)]
    document_type: Option<String>,
    #[serde(default)]
    document_url: Option<String>,
    #[serde(default)]
    valid_to: Option<String>,
    #[serde(default)]
    days_until_expiry: Option<i64>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentsResponse {
    #[serde(default)]
    documents: Vec<ExpiringDocument>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrainingRecord {
    #[serde(default)]
    user_name: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    training_type: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    training_provider: Option<String>,
    #[serde(default)]
    days_until_expiry: Option<i64>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrainingResponse {
    #[serde(default)]
    records: Vec<TrainingRecord>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrainingSummary {
    #[serde(default)]
    total_records: Option<u32>,
    #[serde(default)]
    total_employees: Option<u32>,
    #[serde(default)]
    valid: Option<u32>,
    #[serde(default)]
    expiring_soon: Option<u32>,
    #[serde(default)]
    expired: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Inspection {
    #[serde(default)]
    inspection_type: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    robot_name: Option<String>,
    #[serde(default)]
    last_inspection_date: Option<String>,
    #[serde(default)]
    next_due_date: Option<String>,
    #[serde(default)]
    interval_years: Option<u32>,
    #[serde(default)]
    inspector_name: Option<String>,
    #[serde(default)]
    inspector_company: Option<String>,
    #[serde(default)]
    days_until_due: Option<i64>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectionsResponse {
    #[serde(default)]
    schedules: Vec<Inspection>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectionSummary {
    #[serde(default)]
    total_scheduled: Option<u32>,
    #[serde(default)]
    current: Option<u32>,
    #[serde(default)]
    due_soon: Option<u32>,
    #[serde(default)]
    overdue: Option<u32>,
    // null whenever nothing is scheduled in the future.
    #[serde(default)]
    next_inspection: Option<Inspection>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RiskAssessment {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    assessment_type: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    last_updated: Option<String>,
    #[serde(default)]
    next_review_date: Option<String>,
    #[serde(default)]
    responsible_person: Option<String>,
    #[serde(default)]
    days_until_review: Option<i64>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RiskAssessmentsResponse {
    #[serde(default)]
    assessments: Vec<RiskAssessment>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityEntry {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    framework: Option<String>,
    #[serde(default)]
    entity_type: Option<String>,
    #[serde(default)]
    user_name: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityResponse {
    #[serde(default)]
    activity: Vec<ActivityEntry>,
}

// ── Shared rendering helpers ────────────────────────────────────────────

/// Framework codes are stored verbatim (`machinery_regulation`), which overruns
/// the 14-column label `output::meter` reserves; shorten the seven the server
/// defines and let anything new through truncated.
fn framework_label(framework: &str) -> String {
    match framework {
        "ai_act" => "EU AI Act".to_string(),
        "machinery_regulation" => "Machinery Reg".to_string(),
        "gdpr" => "GDPR".to_string(),
        "nis2" => "NIS2".to_string(),
        "cra" => "CRA".to_string(),
        "red" => "RED".to_string(),
        "dguv" => "DGUV".to_string(),
        other => output::truncate(other, 14),
    }
}

fn score_color(score: f64) -> &'static str {
    if score >= 90.0 {
        "green"
    } else if score >= 70.0 {
        "yellow"
    } else if score >= 50.0 {
        "orange"
    } else {
        "red"
    }
}

/// `output::status_indicator` knows `overdue`/`expired`/`compliant`, but not the
/// tracker's own vocabulary (`due_soon`, `expiring_soon`, `review_needed`,
/// `update_required`, `current`), which would otherwise print colourless.
fn tracker_status(status: &str) -> String {
    match status {
        "current" | "valid" => format!("{} {}", "●".bright_black(), status.green().bold()),
        "due_soon" | "expiring_soon" | "review_needed" => {
            format!("{} {}", "●".bright_black(), status.yellow().bold())
        }
        "update_required" => format!("{} {}", "●".bright_black(), status.red().bold()),
        other => output::status_indicator(other),
    }
}

/// Gap severities are `critical | high | medium | low`, but
/// `output::severity_indicator` only colours the alert vocabulary
/// (critical/error/warning/info), so `high` and below would print grey.
fn gap_severity(severity: &str) -> String {
    match severity {
        "high" => format!("{} {}", "▲", severity.to_uppercase().truecolor(255, 103, 0)),
        "medium" => format!("{} {}", "◆", severity.to_uppercase().yellow()),
        "low" => format!("{} {}", "○", severity.to_uppercase().bright_black()),
        other => output::severity_indicator(other),
    }
}

/// Keep the date, drop the time — the tracker's timestamps are all midnight UTC.
fn short_date(ts: Option<&str>) -> String {
    match ts {
        Some(s) => s.chars().take(10).collect(),
        None => "-".to_string(),
    }
}

/// Minute precision, for logs where the time of day matters.
fn short_datetime(ts: Option<&str>) -> String {
    match ts {
        Some(s) => s
            .chars()
            .take(16)
            .map(|c| if c == 'T' { ' ' } else { c })
            .collect(),
        None => "-".to_string(),
    }
}

/// Every tracker row carries a server-computed day delta that goes NEGATIVE once
/// the date has passed — and most deadlines on this server are already past due,
/// so an overdue cell must never read as an ordinary date.
fn due_cell(days: Option<i64>) -> String {
    match days {
        Some(d) if d < 0 => format!("{}d overdue", -d).red().bold().to_string(),
        Some(d) if d <= 30 => format!("in {d}d").yellow().to_string(),
        Some(d) => format!("in {d}d").normal().to_string(),
        None => "-".bright_black().to_string(),
    }
}

fn date_cell(ts: Option<&str>, days: Option<i64>) -> String {
    let date = short_date(ts);
    if days.is_some_and(|d| d < 0) {
        date.red().to_string()
    } else {
        date.normal().to_string()
    }
}

/// `requirements` and `completedRequirements` arrive as JSON *strings*
/// (`"[\"a\",\"b\"]"`) because the tracker hands the stored column through
/// unparsed. Accept either that or a real array.
fn json_list_len(value: Option<&serde_json::Value>) -> Option<usize> {
    match value? {
        serde_json::Value::Array(items) => Some(items.len()),
        serde_json::Value::String(raw) => serde_json::from_str::<Vec<serde_json::Value>>(raw)
            .ok()
            .map(|items| items.len()),
        _ => None,
    }
}

fn alert_stat(label: &str, count: Option<u32>, hot: &str) {
    let n = count.unwrap_or(0);
    output::stat_line(
        if n > 0 { "■" } else { "│" },
        label,
        &n.to_string(),
        if n > 0 { hot } else { "dim" },
    );
}

// ── dashboard ───────────────────────────────────────────────────────────

async fn dashboard(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    // Flat object, no envelope.
    if ctx.json {
        return output::json_out(&client.get_value("/compliance/tracker/dashboard").await?);
    }

    let d: Dashboard = client.get("/compliance/tracker/dashboard").await?;

    output::section("◈", "Compliance Dashboard");

    let overall = d.overall_score.unwrap_or(0.0);
    output::meter("Overall score", overall, 100.0, score_color(overall));
    if let Some(ts) = &d.last_updated {
        output::field("Scored", &short_datetime(Some(ts)));
    }

    if let Some(a) = &d.alerts {
        output::subsection("Attention");
        alert_stat("Critical gaps", a.critical_gaps, "red");
        alert_stat("Expiring docs", a.expiring_documents, "yellow");
        // Labels stay inside stat_line's 18-column gutter, or the values step out
        // of the column.
        alert_stat("Overdue training", a.overdue_training, "red");
        alert_stat("Inspections late", a.overdue_inspections, "red");
        alert_stat("Deadlines soon", a.upcoming_deadlines, "yellow");
        alert_stat("Risk reviews due", a.pending_risk_reviews, "yellow");
    }

    if d.framework_scores.is_empty() {
        output::info("Server returned no framework scores.");
        println!();
        return Ok(());
    }

    output::subsection("Frameworks");
    for f in &d.framework_scores {
        let raw = f.framework.as_deref().unwrap_or("-");
        let score = f.score.unwrap_or(0.0);
        output::meter(&framework_label(raw), score, 100.0, score_color(score));

        // At 100% with nothing open the meter already says everything; only
        // annotate the frameworks that need work.
        let open = f.open_items.unwrap_or(0);
        let status = f.status.as_deref().unwrap_or("");
        if open > 0 || !matches!(status, "compliant" | "") {
            println!(
                "    {}   {} — {} of {} item(s) open",
                "│".bright_black(),
                tracker_status(status),
                open,
                f.total_items.unwrap_or(0)
            );
        }
    }

    println!();
    Ok(())
}

// ── deadlines ───────────────────────────────────────────────────────────

async fn deadlines(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/compliance/tracker/deadlines").await?);
    }

    let resp: DeadlinesResponse = client.get("/compliance/tracker/deadlines").await?;

    output::section("◈", "Regulatory Deadlines");

    if resp.deadlines.is_empty() {
        output::info("No regulatory deadlines tracked.");
        println!();
        return Ok(());
    }

    let overdue = resp
        .deadlines
        .iter()
        .filter(|d| d.days_until_deadline.is_some_and(|n| n < 0))
        .count();
    if overdue > 0 {
        output::warn(&format!("{overdue} deadline(s) already past due"));
    }

    output::table_header(&[
        ("DEADLINE", 12),
        ("DUE", 14),
        ("FRAMEWORK", 15),
        ("PRIORITY", 10),
        ("REQS", 7),
        ("NAME", 32),
    ]);
    for d in &resp.deadlines {
        let reqs = match (
            json_list_len(d.completed_requirements.as_ref()),
            json_list_len(d.requirements.as_ref()),
        ) {
            (Some(done), Some(total)) => format!("{done}/{total}"),
            (None, Some(total)) => format!("?/{total}"),
            _ => "-".to_string(),
        };
        println!(
            "    {} {} {} {} {} {}",
            output::pad(&date_cell(d.deadline.as_deref(), d.days_until_deadline), 12),
            output::pad(&due_cell(d.days_until_deadline), 14),
            output::pad(
                &output::truncate(&framework_label(d.framework.as_deref().unwrap_or("-")), 15),
                15
            ),
            output::pad(
                &output::truncate(d.priority.as_deref().unwrap_or("-"), 10),
                10
            ),
            output::pad(&reqs, 7),
            output::truncate(d.name.as_deref().unwrap_or("-"), 32)
        );
    }

    output::count_paged("regulatory deadline(s)", resp.deadlines.len(), None, None);
    println!();
    Ok(())
}

// ── gaps ────────────────────────────────────────────────────────────────

async fn gaps(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/compliance/tracker/gaps").await?);
    }

    let resp: GapsResponse = client.get("/compliance/tracker/gaps").await?;

    output::section("◈", "Compliance Gaps");

    if resp.gaps.is_empty() {
        output::info("No compliance gaps recorded.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("SEVERITY", 12),
        ("FRAMEWORK", 15),
        ("ARTICLE", 14),
        ("REQUIREMENT", 28),
        ("DUE", 12),
        ("STATUS", 14),
        ("OWNER", 14),
    ]);
    for g in &resp.gaps {
        println!(
            "    {} {} {} {} {} {} {}",
            output::pad(&gap_severity(g.severity.as_deref().unwrap_or("-")), 12),
            output::pad(
                &output::truncate(&framework_label(g.framework.as_deref().unwrap_or("-")), 15),
                15
            ),
            output::pad(
                &output::truncate(g.article_reference.as_deref().unwrap_or("-"), 14),
                14
            ),
            output::pad(
                &output::truncate(g.requirement.as_deref().unwrap_or("-"), 28),
                28
            ),
            // Gaps carry a plain dueDate with no server-computed day delta, so
            // there is nothing to colour it against.
            output::pad(&short_date(g.due_date.as_deref()), 12),
            output::pad(&tracker_status(g.status.as_deref().unwrap_or("-")), 14),
            output::truncate(g.assigned_to.as_deref().unwrap_or("-"), 14)
        );
    }

    // The route reports `status` per gap but has no closed/open rollup, so show
    // the open count the operator actually has to act on.
    let open = resp
        .gaps
        .iter()
        .filter(|g| g.status.as_deref() != Some("closed"))
        .count();
    output::count_paged("compliance gap(s)", resp.gaps.len(), None, None);
    if open != resp.gaps.len() {
        output::info(&format!("{open} still open"));
    }
    println!();
    Ok(())
}

async fn gaps_summary(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/compliance/tracker/gaps/summary").await?);
    }

    let resp: GapSummaryResponse = client.get("/compliance/tracker/gaps/summary").await?;

    output::section("◈", "Gap Summary by Framework");

    if resp.summary.is_empty() {
        // The rollup only counts gaps whose status is not 'closed', so an empty
        // map means nothing is open — not that the feature is unused.
        output::info("No open gaps in any framework.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("FRAMEWORK", 18),
        ("TOTAL", 7),
        ("CRITICAL", 9),
        ("HIGH", 7),
        ("MEDIUM", 7),
        ("LOW", 7),
    ]);
    let mut totals = (0u32, 0u32, 0u32, 0u32, 0u32);
    for (framework, s) in &resp.summary {
        let (total, critical, high, medium, low) = (
            s.total.unwrap_or(0),
            s.critical.unwrap_or(0),
            s.high.unwrap_or(0),
            s.medium.unwrap_or(0),
            s.low.unwrap_or(0),
        );
        totals = (
            totals.0 + total,
            totals.1 + critical,
            totals.2 + high,
            totals.3 + medium,
            totals.4 + low,
        );
        println!(
            "    {} {} {} {} {} {}",
            output::pad(&output::truncate(&framework_label(framework), 18), 18),
            output::pad(&total.to_string(), 7),
            output::pad(
                &if critical > 0 {
                    critical.to_string().red().bold().to_string()
                } else {
                    "0".bright_black().to_string()
                },
                9
            ),
            output::pad(
                &if high > 0 {
                    high.to_string().yellow().to_string()
                } else {
                    "0".bright_black().to_string()
                },
                7
            ),
            output::pad(&medium.to_string(), 7),
            low
        );
    }

    println!();
    output::stat_line("│", "Open gaps", &totals.0.to_string(), "orange");
    output::stat_line(
        if totals.1 > 0 { "■" } else { "│" },
        "Critical",
        &totals.1.to_string(),
        if totals.1 > 0 { "red" } else { "dim" },
    );

    output::count_paged(
        "framework(s) with open gaps",
        resp.summary.len(),
        None,
        None,
    );
    println!();
    Ok(())
}

// ── metrics ─────────────────────────────────────────────────────────────

async fn metrics(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    // Note the path: metrics is served by the compliance-LOG routes, not the
    // tracker. (GET /compliance/logs itself 500s on this server.)
    if ctx.json {
        return output::json_out(&client.get_value("/compliance/metrics").await?);
    }

    let m: Metrics = client.get("/compliance/metrics").await?;

    output::section("◈", "Compliance Log Metrics");

    let total = m.total_logs.unwrap_or(0);
    output::stat_line("│", "Log entries", &total.to_string(), "orange");
    output::stat_line(
        "│",
        "Unique sessions",
        &m.unique_sessions.unwrap_or(0).to_string(),
        "dim",
    );
    output::stat_line(
        "│",
        "Unique robots",
        &m.unique_robots.unwrap_or(0).to_string(),
        "dim",
    );
    if let Some(r) = &m.date_range {
        output::field(
            "Range",
            &format!(
                "{} → {}",
                short_date(r.start.as_deref()),
                short_date(r.end.as_deref())
            ),
        );
    }

    if let Some(s) = &m.severity_counts {
        output::subsection("By severity");
        severity_stat("critical", s.critical, "red");
        severity_stat("error", s.error, "red");
        severity_stat("warning", s.warning, "yellow");
        severity_stat("info", s.info, "dim");
        severity_stat("debug", s.debug, "dim");
    }

    if m.event_type_counts.is_empty() {
        output::info("No event-type counts reported.");
        println!();
        return Ok(());
    }

    output::subsection("By event type");
    // Server orders these alphabetically; loudest-first is what you want here.
    let mut events: Vec<&EventTypeCount> = m.event_type_counts.iter().collect();
    events.sort_by_key(|a| std::cmp::Reverse(a.count.unwrap_or(0)));

    output::table_header(&[
        ("EVENT TYPE", 22),
        ("COUNT", 10),
        ("SHARE", 8),
        ("LAST SEEN", 16),
    ]);
    for e in events {
        let count = e.count.unwrap_or(0);
        let share = if total > 0 {
            format!("{:.1}%", count as f64 / total as f64 * 100.0)
        } else {
            "-".to_string()
        };
        println!(
            "    {} {} {} {}",
            output::pad(
                &output::truncate(e.event_type.as_deref().unwrap_or("-"), 22),
                22
            ),
            output::pad(&count.to_string(), 10),
            output::pad(&share, 8),
            short_datetime(e.last_occurrence.as_deref())
        );
    }

    println!();
    Ok(())
}

fn severity_stat(label: &str, count: Option<u64>, hot: &str) {
    let n = count.unwrap_or(0);
    output::stat_line("│", label, &n.to_string(), if n > 0 { hot } else { "dim" });
}

// ── documents ───────────────────────────────────────────────────────────

async fn documents(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let path = "/compliance/tracker/documents/expiring";

    if ctx.json {
        return output::json_out(&client.get_value(path).await?);
    }

    let resp: DocumentsResponse = client.get(path).await?;

    output::section("◈", "Expiring Documents");
    // The route takes ?withinDays and defaults to 30; already-expired documents
    // are included, so the list is "needs attention", not "expires later".
    output::info("Window: validTo within the next 30 days (server default).");

    if resp.documents.is_empty() {
        output::info("No provider documentation expiring in that window.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("PROVIDER", 20),
        ("MODEL", 22),
        ("TYPE", 24),
        ("VALID TO", 12),
        ("EXPIRY", 14),
        ("STATUS", 16),
    ]);
    for d in &resp.documents {
        println!(
            "    {} {} {} {} {} {}",
            output::pad(
                &output::truncate(d.provider_name.as_deref().unwrap_or("-"), 20),
                20
            ),
            output::pad(
                &output::truncate(d.model_version.as_deref().unwrap_or("-"), 22),
                22
            ),
            output::pad(
                &output::truncate(d.document_type.as_deref().unwrap_or("-"), 24),
                24
            ),
            output::pad(&date_cell(d.valid_to.as_deref(), d.days_until_expiry), 12),
            output::pad(&due_cell(d.days_until_expiry), 14),
            tracker_status(d.status.as_deref().unwrap_or("-"))
        );
    }

    let with_url = resp
        .documents
        .iter()
        .filter(|d| d.document_url.is_some())
        .count();
    output::count_paged("document(s)", resp.documents.len(), None, None);
    if with_url > 0 {
        output::info(&format!(
            "{with_url} of {} have a documentUrl — use --json for the links",
            resp.documents.len()
        ));
    }
    println!();
    Ok(())
}

// ── training ────────────────────────────────────────────────────────────

async fn training(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let path = "/compliance/tracker/training";

    if ctx.json {
        return output::json_out(&client.get_value(path).await?);
    }

    let resp: TrainingResponse = client.get(path).await?;

    output::section("◈", "Compliance Training");

    if resp.records.is_empty() {
        output::info("No training records on file.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("PERSON", 22),
        ("TYPE", 24),
        ("COMPLETED", 12),
        ("EXPIRES", 12),
        ("EXPIRY", 14),
        ("STATUS", 16),
    ]);
    for r in &resp.records {
        println!(
            "    {} {} {} {} {} {}",
            output::pad(
                &output::truncate(
                    r.user_name
                        .as_deref()
                        .or(r.user_id.as_deref())
                        .unwrap_or("-"),
                    22
                ),
                22
            ),
            output::pad(
                &output::truncate(r.training_type.as_deref().unwrap_or("-"), 24),
                24
            ),
            output::pad(&short_date(r.completed_at.as_deref()), 12),
            output::pad(&date_cell(r.expires_at.as_deref(), r.days_until_expiry), 12),
            output::pad(&due_cell(r.days_until_expiry), 14),
            tracker_status(r.status.as_deref().unwrap_or("-"))
        );
    }

    let expired = resp
        .records
        .iter()
        .filter(|r| r.days_until_expiry.is_some_and(|d| d < 0))
        .count();
    if expired > 0 {
        output::warn(&format!(
            "{expired} record(s) expired — the operator is no longer certified"
        ));
    }
    output::count_paged("training record(s)", resp.records.len(), None, None);
    println!();
    Ok(())
}

async fn training_summary(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let path = "/compliance/tracker/training/summary";

    if ctx.json {
        return output::json_out(&client.get_value(path).await?);
    }

    let s: TrainingSummary = client.get(path).await?;

    output::section("◈", "Training Compliance");

    let total = s.total_records.unwrap_or(0);
    output::stat_line("│", "Records", &total.to_string(), "orange");
    output::stat_line(
        "│",
        "Employees",
        &s.total_employees.unwrap_or(0).to_string(),
        "dim",
    );
    output::stat_line(
        "│",
        "Valid",
        &s.valid.unwrap_or(0).to_string(),
        if s.valid.unwrap_or(0) > 0 {
            "green"
        } else {
            "dim"
        },
    );
    alert_stat("Expiring soon", s.expiring_soon, "yellow");
    alert_stat("Expired", s.expired, "red");

    if total > 0 {
        let valid = s.valid.unwrap_or(0) as f64;
        let pct = valid / total as f64 * 100.0;
        output::meter("Valid share", valid, total as f64, score_color(pct));
    } else {
        output::info("No training records to score.");
    }

    println!();
    Ok(())
}

// ── inspections ─────────────────────────────────────────────────────────

async fn inspections(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let path = "/compliance/tracker/inspections";

    if ctx.json {
        return output::json_out(&client.get_value(path).await?);
    }

    // Named `schedules`, not `inspections`.
    let resp: InspectionsResponse = client.get(path).await?;

    output::section("◈", "Inspection Schedules");

    if resp.schedules.is_empty() {
        output::info("No inspection schedules recorded (DGUV Vorschrift 3).");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("ROBOT", 24),
        ("TYPE", 20),
        ("LAST", 12),
        ("NEXT DUE", 12),
        ("DUE", 14),
        ("EVERY", 7),
        ("STATUS", 14),
    ]);
    for s in &resp.schedules {
        println!(
            "    {} {} {} {} {} {} {}",
            output::pad(&output::truncate(inspection_subject(s), 24), 24),
            output::pad(
                &output::truncate(s.inspection_type.as_deref().unwrap_or("-"), 20),
                20
            ),
            output::pad(&short_date(s.last_inspection_date.as_deref()), 12),
            output::pad(&date_cell(s.next_due_date.as_deref(), s.days_until_due), 12),
            output::pad(&due_cell(s.days_until_due), 14),
            output::pad(
                &s.interval_years
                    .map(|y| format!("{y}y"))
                    .unwrap_or_else(|| "-".to_string()),
                7
            ),
            tracker_status(s.status.as_deref().unwrap_or("-"))
        );
    }

    output::count_paged("inspection schedule(s)", resp.schedules.len(), None, None);
    println!();
    Ok(())
}

/// robotId/robotName are both optional: a schedule can cover the installation
/// rather than one robot.
fn inspection_subject(s: &Inspection) -> &str {
    s.robot_name
        .as_deref()
        .or(s.robot_id.as_deref())
        .unwrap_or("(site-wide)")
}

async fn inspections_summary(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let path = "/compliance/tracker/inspections/summary";

    if ctx.json {
        return output::json_out(&client.get_value(path).await?);
    }

    let s: InspectionSummary = client.get(path).await?;

    output::section("◈", "Inspection Compliance");

    output::stat_line(
        "│",
        "Scheduled",
        &s.total_scheduled.unwrap_or(0).to_string(),
        "orange",
    );
    output::stat_line(
        "│",
        "Current",
        &s.current.unwrap_or(0).to_string(),
        if s.current.unwrap_or(0) > 0 {
            "green"
        } else {
            "dim"
        },
    );
    alert_stat("Due soon", s.due_soon, "yellow");
    alert_stat("Overdue", s.overdue, "red");

    match &s.next_inspection {
        Some(n) => {
            output::subsection("Next inspection");
            output::field("Subject", inspection_subject(n));
            output::field("Type", n.inspection_type.as_deref().unwrap_or("-"));
            output::field(
                "Due",
                &format!(
                    "{} — {}",
                    short_date(n.next_due_date.as_deref()),
                    due_cell(n.days_until_due)
                ),
            );
            if let Some(name) = &n.inspector_name {
                output::field("Inspector", name);
            }
            if let Some(company) = &n.inspector_company {
                output::field("Company", company);
            }
        }
        None => output::info("No inspection scheduled in the future."),
    }

    println!();
    Ok(())
}

// ── risk assessments ────────────────────────────────────────────────────

async fn risk_assessments(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let path = "/compliance/tracker/risk-assessments";

    if ctx.json {
        return output::json_out(&client.get_value(path).await?);
    }

    let resp: RiskAssessmentsResponse = client.get(path).await?;

    output::section("◈", "Risk Assessments");

    if resp.assessments.is_empty() {
        output::info("No risk assessments recorded.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("NAME", 24),
        ("TYPE", 16),
        ("VER", 6),
        ("NEXT REVIEW", 12),
        ("REVIEW", 14),
        ("OWNER", 16),
        ("STATUS", 16),
    ]);
    for a in &resp.assessments {
        println!(
            "    {} {} {} {} {} {} {}",
            output::pad(&output::truncate(a.name.as_deref().unwrap_or("-"), 24), 24),
            output::pad(
                &output::truncate(a.assessment_type.as_deref().unwrap_or("-"), 16),
                16
            ),
            output::pad(&output::truncate(a.version.as_deref().unwrap_or("-"), 6), 6),
            output::pad(
                &date_cell(a.next_review_date.as_deref(), a.days_until_review),
                12
            ),
            output::pad(&due_cell(a.days_until_review), 14),
            output::pad(
                &output::truncate(a.responsible_person.as_deref().unwrap_or("-"), 16),
                16
            ),
            tracker_status(a.status.as_deref().unwrap_or("-"))
        );
    }

    let stale = resp
        .assessments
        .iter()
        .filter(|a| a.days_until_review.is_some_and(|d| d < 0))
        .count();
    if stale > 0 {
        output::warn(&format!("{stale} assessment(s) past their review date"));
    }
    output::count_paged("risk assessment(s)", resp.assessments.len(), None, None);
    println!();
    Ok(())
}

// ── activity ────────────────────────────────────────────────────────────

async fn activity(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let path = "/compliance/tracker/activity";
    // This route reads ?limit only (and defaults to 20); ?page is ignored.
    let query = ctx.paging_limit();

    if ctx.json {
        return output::json_out(&client.get_value_query(path, &query).await?);
    }

    let resp: ActivityResponse = client.get_query(path, &query).await?;

    output::section("◈", "Recent Compliance Activity");

    if resp.activity.is_empty() {
        output::info("No compliance activity logged.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("WHEN", 17),
        ("TYPE", 20),
        ("FRAMEWORK", 15),
        ("WHO", 16),
        ("DESCRIPTION", 34),
    ]);
    for a in &resp.activity {
        println!(
            "    {} {} {} {} {}",
            output::pad(&short_datetime(a.timestamp.as_deref()), 17),
            output::pad(&output::truncate(a.kind.as_deref().unwrap_or("-"), 20), 20),
            output::pad(
                &output::truncate(&framework_label(a.framework.as_deref().unwrap_or("-")), 15),
                15
            ),
            // userName is only set for operator-driven events; the tracker logs
            // its own activity without an actor.
            output::pad(
                &output::truncate(a.user_name.as_deref().unwrap_or("-"), 16),
                16
            ),
            output::truncate(a.description.as_deref().unwrap_or("-"), 34)
        );
    }

    // The server caps an unfiltered request at its own default, and returns no
    // total, so say where the wall came from rather than implying that is all.
    if ctx.limit.is_none() && resp.activity.len() >= 20 {
        output::info("Server returns the 20 most recent by default — use --limit N for more.");
    }
    output::count_paged("activity record(s)", resp.activity.len(), None, None);
    println!();
    Ok(())
}
