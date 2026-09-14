use crate::cli::IncidentsCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::RmsResult;
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub async fn run(cmd: &IncidentsCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        IncidentsCommand::List {
            status,
            severity,
            robot_id,
        } => {
            list(
                client,
                ctx,
                status.as_deref(),
                severity.as_deref(),
                robot_id.as_deref(),
            )
            .await
        }
        IncidentsCommand::Show { id } => show(client, ctx, id).await,
        IncidentsCommand::Dashboard => dashboard(client, ctx).await,
        IncidentsCommand::Open => open(client, ctx).await,
        IncidentsCommand::Overdue => overdue(client, ctx).await,
        IncidentsCommand::RiskMatrix => risk_matrix(client, ctx).await,
        IncidentsCommand::Evidence { id } => evidence(client, ctx, id).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Incident {
    id: String,
    #[serde(default)]
    incident_number: Option<String>,
    /// The server field is literally `type`.
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    root_cause: Option<String>,
    #[serde(default)]
    resolution: Option<String>,
    /// 0-100. Float rather than integer so a future fractional score still parses.
    #[serde(default)]
    risk_score: Option<f64>,
    #[serde(default)]
    affected_data_subjects: Option<f64>,
    #[serde(default)]
    data_categories: Option<Vec<String>>,
    #[serde(default)]
    detected_at: Option<String>,
    #[serde(default)]
    contained_at: Option<String>,
    #[serde(default)]
    resolved_at: Option<String>,
    #[serde(default)]
    closed_at: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    compliance_log_ids: Option<Vec<String>>,
    #[serde(default)]
    alert_ids: Option<Vec<String>>,
    #[serde(default)]
    clip_key: Option<String>,
    /// Stored as an opaque JSON column and null on every live incident here, so
    /// it stays a `Value` and is rendered field by field — a typed struct would
    /// turn one unexpected key into a hard parse failure.
    #[serde(default)]
    system_snapshot: Option<serde_json::Value>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    created_by: Option<String>,
    #[serde(default)]
    notifications: Option<Vec<Notification>>,
}

/// `GET /incidents` puts its pagination fields flat on the envelope instead of
/// inside a `pagination` object, and `GET /incidents/open` omits them entirely.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct IncidentsResponse {
    #[serde(default)]
    incidents: Vec<Incident>,
    #[serde(default)]
    total: Option<u32>,
    #[serde(default)]
    total_pages: Option<u32>,
}

/// Regulatory notification (AI Act / GDPR / NIS2 / CRA deadline clock).
///
/// The live server has none, so nothing here could be observed — every field
/// including the id is optional.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Notification {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    incident_id: Option<String>,
    #[serde(default)]
    authority: Option<String>,
    #[serde(default)]
    regulation: Option<String>,
    #[serde(default)]
    notification_type: Option<String>,
    #[serde(default)]
    deadline_hours: Option<f64>,
    #[serde(default)]
    due_at: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    sent_at: Option<String>,
    #[serde(default)]
    acknowledged_at: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    sent_by: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverdueResponse {
    #[serde(default)]
    notifications: Vec<Notification>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Dashboard {
    #[serde(default)]
    total_incidents: Option<u32>,
    #[serde(default)]
    open_incidents: Option<u32>,
    #[serde(default)]
    incidents_by_severity: Option<HashMap<String, u32>>,
    #[serde(default)]
    incidents_by_type: Option<HashMap<String, u32>>,
    #[serde(default)]
    incidents_by_status: Option<HashMap<String, u32>>,
    #[serde(default)]
    overdue_notifications: Option<u32>,
    #[serde(default)]
    pending_notifications: Option<u32>,
    #[serde(default)]
    average_resolution_time_hours: Option<f64>,
    #[serde(default)]
    recent_incidents: Vec<Incident>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RiskCell {
    #[serde(default)]
    impact: Option<String>,
    #[serde(default)]
    likelihood: Option<String>,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    severity: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RiskMatrixResponse {
    #[serde(default)]
    matrix: Vec<RiskCell>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EvidenceResponse {
    #[serde(default)]
    compliance_log_ids: Option<Vec<String>>,
    #[serde(default)]
    alert_ids: Option<Vec<String>>,
    #[serde(default)]
    system_snapshot: Option<serde_json::Value>,
}

// ── Shared rendering ────────────────────────────────────────────────────

const SEVERITIES: &[&str] = &["critical", "high", "medium", "low"];
const STATUSES: &[&str] = &[
    "detected",
    "investigating",
    "contained",
    "resolved",
    "closed",
];

/// `output::severity_indicator` knows the alert vocabulary
/// (critical/error/warning/info); incidents grade critical/high/medium/low, so
/// three of the four levels would otherwise render uncoloured.
fn severity_cell(severity: &str) -> String {
    match severity.to_lowercase().as_str() {
        "critical" => format!("▲ {}", severity.to_uppercase().red().bold()),
        "high" => format!("▲ {}", severity.to_uppercase().red()),
        "medium" => format!("◆ {}", severity.to_uppercase().yellow()),
        "low" => format!("● {}", severity.to_uppercase().blue()),
        _ => output::severity_indicator(severity),
    }
}

/// Same gap on the status axis: `output::status_indicator` does not know
/// `detected`, `investigating` or `contained`, so it would grade a fresh
/// incident as neutral/idle. What it does know (resolved, closed) falls through.
fn status_cell(status: &str) -> String {
    match status.to_lowercase().as_str() {
        "detected" => format!("{} {}", "●".bright_black(), status.yellow().bold()),
        "investigating" => format!(
            "{} {}",
            "◐".bright_black(),
            status.truecolor(255, 103, 0).bold()
        ),
        "contained" => format!("{} {}", "◐".bright_black(), status.yellow().bold()),
        _ => output::status_indicator(status),
    }
}

fn count_color(key: &str, count: u32) -> &'static str {
    if count == 0 {
        return "dim";
    }
    match key {
        "critical" | "high" | "overdue" => "red",
        "medium" | "detected" | "investigating" | "pending" => "yellow",
        "resolved" | "closed" => "green",
        _ => "orange",
    }
}

/// Print a `{key: count}` rollup in a fixed, meaningful order — a JSON object
/// deserialised into a map loses the server's ordering, and severity read
/// alphabetically (critical, high, low, medium) is actively misleading.
fn render_breakdown(map: &HashMap<String, u32>, preferred: &[&str]) {
    let mut keys: Vec<String> = preferred
        .iter()
        .filter(|k| map.contains_key(**k))
        .map(|k| (*k).to_string())
        .collect();
    let mut rest: Vec<String> = map
        .keys()
        .filter(|k| !preferred.contains(&k.as_str()))
        .cloned()
        .collect();
    rest.sort();
    keys.extend(rest);

    for k in keys {
        let count = map.get(&k).copied().unwrap_or(0);
        output::stat_line("│", &k, &count.to_string(), count_color(&k, count));
    }
}

fn render_rows(rows: &[Incident]) {
    output::table_header(&[
        ("NUMBER", 15),
        ("SEV", 12),
        ("STATUS", 15),
        ("TITLE", 34),
        ("ID", output::ID_W),
    ]);
    for i in rows {
        println!(
            "    {} {} {} {} {}",
            output::pad(i.incident_number.as_deref().unwrap_or("-"), 15),
            output::pad(&severity_cell(i.severity.as_deref().unwrap_or("-")), 12),
            output::pad(&status_cell(i.status.as_deref().unwrap_or("-")), 15),
            output::pad(&output::truncate(i.title.as_deref().unwrap_or("-"), 34), 34),
            i.id.bright_black()
        );
    }
}

/// Flag which rows still need attention, since the table's colour alone is easy
/// to miss when the queue is long.
fn warn_on_serious(rows: &[Incident]) {
    let serious = rows
        .iter()
        .filter(|i| {
            matches!(
                i.severity.as_deref().map(str::to_lowercase).as_deref(),
                Some("critical") | Some("high")
            )
        })
        .count();
    if serious > 0 {
        output::warn(&format!(
            "{serious} incident(s) at high or critical severity in this page."
        ));
    }
}

fn render_notifications(rows: &[Notification], with_incident: bool) {
    let mut columns: Vec<(&str, usize)> = vec![
        ("DUE", 26),
        ("REGULATION", 12),
        ("AUTHORITY", 20),
        ("TYPE", 16),
        ("STATUS", 14),
    ];
    if with_incident {
        columns.push(("INCIDENT", output::ID_W));
    }
    output::table_header(&columns);

    for n in rows {
        let mut line = format!(
            "    {} {} {} {} {}",
            output::pad(n.due_at.as_deref().unwrap_or("-"), 26),
            output::pad(n.regulation.as_deref().unwrap_or("-"), 12),
            output::pad(
                &output::truncate(n.authority.as_deref().unwrap_or("-"), 20),
                20
            ),
            output::pad(n.notification_type.as_deref().unwrap_or("-"), 16),
            output::pad(
                &output::status_indicator(n.status.as_deref().unwrap_or("-")),
                14
            ),
        );
        if with_incident {
            // The overdue route returns bare notifications, so there is no
            // incidentNumber to show here — only the raw id to feed `show`.
            line.push(' ');
            line.push_str(&n.incident_id.as_deref().unwrap_or("-").bright_black());
        }
        println!("{line}");
    }
}

/// Wrap free text into the field gutter. Incident descriptions are generated by
/// the patrol pipeline and arrive as one several-hundred-character line.
fn print_wrapped(text: &str, width: usize) {
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            println!("    {} {}", "│".bright_black(), line);
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        println!("    {} {}", "│".bright_black(), line);
    }
}

fn render_snapshot(v: &serde_json::Value) {
    if let Some(at) = v.get("capturedAt").and_then(|x| x.as_str()) {
        output::field("Captured", at);
    }
    if let Some(robots) = v.get("robots").and_then(|x| x.as_array()) {
        output::field("Robots", &robots.len().to_string());
        for r in robots.iter().take(10) {
            let id = r.get("id").and_then(|x| x.as_str()).unwrap_or("-");
            let status = r.get("status").and_then(|x| x.as_str()).unwrap_or("-");
            let battery = r
                .get("batteryLevel")
                .and_then(|x| x.as_f64())
                .map(|b| format!(" {b:.0}%"))
                .unwrap_or_default();
            println!(
                "    {}   {} {}{}",
                "│".bright_black(),
                output::pad(&output::truncate(id, 26), 26),
                output::status_indicator(status),
                battery
            );
        }
        if robots.len() > 10 {
            output::info(&format!(
                "{} more robot(s) in the snapshot — use --json for all",
                robots.len() - 10
            ));
        }
    }
    if let Some(alerts) = v.get("activeAlerts").and_then(|x| x.as_array()) {
        output::field("Active alerts", &alerts.len().to_string());
        for a in alerts.iter().take(10) {
            let sev = a.get("severity").and_then(|x| x.as_str()).unwrap_or("-");
            let title = a.get("title").and_then(|x| x.as_str()).unwrap_or("-");
            println!(
                "    {}   {} {}",
                "│".bright_black(),
                output::pad(&output::severity_indicator(sev), 12),
                output::truncate(title, 50)
            );
        }
    }
    if let Some(h) = v.get("systemHealth") {
        if let Some(n) = h.get("connectedRobots").and_then(|x| x.as_f64()) {
            output::field("Connected", &format!("{n:.0}"));
        }
        if let Some(n) = h.get("activeWebSockets").and_then(|x| x.as_f64()) {
            output::field("WebSockets", &format!("{n:.0}"));
        }
        if let Some(n) = h.get("serverUptime").and_then(|x| x.as_f64()) {
            output::field("Server uptime", &format!("{n:.0}s"));
        }
    }
}

// ── list ────────────────────────────────────────────────────────────────

/// Tokens the server does not recognise come back as an empty list rather than a
/// 400, so a typo looks exactly like a quiet queue. Name them instead.
fn unknown_tokens(value: &str, allowed: &[&str]) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter(|t| !allowed.contains(&t.to_lowercase().as_str()))
        .map(str::to_string)
        .collect()
}

async fn list(
    client: &ApiClient,
    ctx: &Ctx,
    status: Option<&str>,
    severity: Option<&str>,
    robot_id: Option<&str>,
) -> RmsResult<()> {
    let mut own: Vec<(&'static str, String)> = Vec::new();
    if let Some(s) = status {
        own.push(("status", s.to_string()));
    }
    if let Some(s) = severity {
        own.push(("severity", s.to_string()));
    }
    if let Some(r) = robot_id {
        own.push(("robotId", r.to_string()));
    }
    // This route spells its page size `limit`.
    let query = ctx.with_paging_limit(own);

    if ctx.json {
        return output::json_out(&client.get_value_query("/incidents", &query).await?);
    }

    let resp: IncidentsResponse = client.get_query("/incidents", &query).await?;

    output::section("◈", "Incidents");

    for (flag, value, allowed) in [
        ("--status", status, STATUSES),
        ("--severity", severity, SEVERITIES),
    ] {
        if let Some(v) = value {
            let bad = unknown_tokens(v, allowed);
            if !bad.is_empty() {
                output::warn(&format!(
                    "{flag} {} is not a known value ({}) — the server will match nothing",
                    bad.join(", "),
                    allowed.join(", ")
                ));
            }
        }
    }

    if resp.incidents.is_empty() {
        output::info("No incidents match that filter.");
        println!();
        return Ok(());
    }

    render_rows(&resp.incidents);
    output::count_paged(
        "incident(s)",
        resp.incidents.len(),
        resp.total,
        resp.total_pages,
    );
    warn_on_serious(&resp.incidents);
    println!();
    Ok(())
}

// ── open ────────────────────────────────────────────────────────────────

async fn open(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/incidents/open").await?);
    }

    // Same `{incidents:[...]}` key as the list route but with no pagination
    // envelope: this route always returns every non-closed incident.
    let resp: IncidentsResponse = client.get("/incidents/open").await?;

    output::section("◈", "Open Incidents");

    if resp.incidents.is_empty() {
        output::info("No open incidents — everything is closed.");
        println!();
        return Ok(());
    }

    render_rows(&resp.incidents);
    output::count_paged("open incident(s)", resp.incidents.len(), None, None);
    warn_on_serious(&resp.incidents);
    println!();
    Ok(())
}

// ── show ────────────────────────────────────────────────────────────────

async fn show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/incidents/{id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let i: Incident = client.get(&path).await?;

    output::section(
        "◈",
        &format!(
            "{} {}",
            i.incident_number.as_deref().unwrap_or("Incident"),
            i.title.as_deref().unwrap_or("-").bold()
        ),
    );

    output::field(
        "Severity",
        &severity_cell(i.severity.as_deref().unwrap_or("-")),
    );
    output::field("Status", &status_cell(i.status.as_deref().unwrap_or("-")));
    output::field("Type", i.kind.as_deref().unwrap_or("-"));
    output::field("Robot", i.robot_id.as_deref().unwrap_or("-"));
    output::field("ID", &i.id);

    if let Some(score) = i.risk_score {
        output::meter(
            "Risk score",
            score,
            100.0,
            if score >= 60.0 { "red" } else { "yellow" },
        );
    }
    if let Some(subjects) = i.affected_data_subjects {
        output::field("Data subjects", &format!("{subjects:.0}"));
    }
    let categories = i.data_categories.as_deref().unwrap_or_default();
    if !categories.is_empty() {
        output::field("Categories", &categories.join(", "));
    }

    output::subsection("Timeline");
    for (label, ts) in [
        ("Detected", &i.detected_at),
        ("Contained", &i.contained_at),
        ("Resolved", &i.resolved_at),
        ("Closed", &i.closed_at),
    ] {
        output::field(label, ts.as_deref().unwrap_or("-"));
    }
    if let Some(created) = &i.created_at {
        output::field("Created", created);
    }
    if let Some(by) = &i.created_by {
        output::field("Created by", by);
    }
    if let Some(updated) = &i.updated_at {
        output::field("Updated", updated);
    }

    if let Some(description) = i.description.as_deref().filter(|d| !d.trim().is_empty()) {
        output::subsection("Description");
        print_wrapped(description, 76);
    }
    if let Some(cause) = i.root_cause.as_deref().filter(|c| !c.trim().is_empty()) {
        output::subsection("Root cause");
        print_wrapped(cause, 76);
    }
    if let Some(resolution) = i.resolution.as_deref().filter(|r| !r.trim().is_empty()) {
        output::subsection("Resolution");
        print_wrapped(resolution, 76);
    }

    let alerts = i.alert_ids.as_deref().unwrap_or_default();
    let logs = i.compliance_log_ids.as_deref().unwrap_or_default();
    if !alerts.is_empty() || !logs.is_empty() || i.clip_key.is_some() {
        output::subsection("Evidence");
        output::field("Alerts", &alerts.len().to_string());
        output::field("Logs", &logs.len().to_string());
        if let Some(clip) = &i.clip_key {
            output::field("Clip", clip);
        }
        output::field_colored(
            "Detail",
            format!("rmsctl incidents evidence {}", i.id).bright_black(),
        );
    }

    let notifications = i.notifications.as_deref().unwrap_or_default();
    if !notifications.is_empty() {
        output::subsection("Regulatory notifications");
        render_notifications(notifications, false);
    }

    println!();
    Ok(())
}

// ── dashboard ───────────────────────────────────────────────────────────

async fn dashboard(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/incidents/dashboard").await?);
    }

    // Flat object, no envelope key.
    let d: Dashboard = client.get("/incidents/dashboard").await?;

    output::section("◈", "Incident Dashboard");

    let open = d.open_incidents.unwrap_or(0);
    output::stat_line(
        "│",
        "Total",
        &d.total_incidents.unwrap_or(0).to_string(),
        "orange",
    );
    output::stat_line(
        if open == 0 { "│" } else { "■" },
        "Open",
        &open.to_string(),
        if open == 0 { "dim" } else { "yellow" },
    );
    let overdue = d.overdue_notifications.unwrap_or(0);
    output::stat_line(
        if overdue == 0 { "│" } else { "■" },
        "Overdue notices",
        &overdue.to_string(),
        if overdue == 0 { "dim" } else { "red" },
    );
    output::stat_line(
        "│",
        "Pending notices",
        &d.pending_notifications.unwrap_or(0).to_string(),
        if d.pending_notifications.unwrap_or(0) > 0 {
            "yellow"
        } else {
            "dim"
        },
    );
    // Null until something has actually been resolved.
    output::stat_line(
        "│",
        "Avg resolution",
        &d.average_resolution_time_hours
            .map(|h| format!("{h:.1}h"))
            .unwrap_or_else(|| "-".to_string()),
        "dim",
    );

    if let Some(map) = &d.incidents_by_severity {
        output::subsection("By severity");
        render_breakdown(map, SEVERITIES);
    }
    if let Some(map) = &d.incidents_by_status {
        output::subsection("By status");
        render_breakdown(map, STATUSES);
    }
    if let Some(map) = &d.incidents_by_type {
        output::subsection("By type");
        render_breakdown(
            map,
            &[
                "safety",
                "security",
                "data_breach",
                "ai_malfunction",
                "vulnerability",
            ],
        );
    }

    if !d.recent_incidents.is_empty() {
        output::subsection("Recent");
        render_rows(&d.recent_incidents);
    }

    if overdue > 0 {
        println!();
        output::warn("Overdue regulatory notifications — see `rmsctl incidents overdue`.");
    }

    println!();
    Ok(())
}

// ── overdue ─────────────────────────────────────────────────────────────

async fn overdue(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/incidents/overdue").await?);
    }

    // Despite sitting under /incidents, this route lists notifications whose
    // regulatory deadline has passed, not incidents.
    let resp: OverdueResponse = client.get("/incidents/overdue").await?;

    output::section("◈", "Overdue Notifications");

    if resp.notifications.is_empty() {
        output::info("No regulatory notification is past its deadline.");
        println!();
        return Ok(());
    }

    render_notifications(&resp.notifications, true);
    output::count_paged(
        "overdue notification(s)",
        resp.notifications.len(),
        None,
        None,
    );
    println!();
    Ok(())
}

// ── risk-matrix ─────────────────────────────────────────────────────────

async fn risk_matrix(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/incidents/risk-matrix").await?);
    }

    let resp: RiskMatrixResponse = client.get("/incidents/risk-matrix").await?;

    output::section("◈", "Risk Matrix");

    if resp.matrix.is_empty() {
        output::info("The server returned no risk matrix.");
        println!();
        return Ok(());
    }

    // The payload is a flat list of cells; rebuild the grid, keeping the
    // server's own axis order (negligible→severe, rare→certain) rather than
    // sorting, which would scramble the ordinal scale.
    let mut impacts: Vec<&str> = Vec::new();
    let mut likelihoods: Vec<&str> = Vec::new();
    for c in &resp.matrix {
        if let Some(i) = c.impact.as_deref()
            && !impacts.contains(&i)
        {
            impacts.push(i);
        }
        if let Some(l) = c.likelihood.as_deref()
            && !likelihoods.contains(&l)
        {
            likelihoods.push(l);
        }
    }

    const CELL: usize = 11;
    let mut header = format!("    {}", output::pad("IMPACT \\ LIKELIHOOD", 22));
    for l in &likelihoods {
        header.push_str(&output::pad(&output::truncate(l, CELL - 1), CELL));
    }
    println!("{}", header.bright_black());
    println!(
        "    {}",
        "─".repeat(22 + CELL * likelihoods.len()).bright_black()
    );

    for imp in &impacts {
        let mut row = format!("    {}", output::pad(imp, 22));
        for lk in &likelihoods {
            let cell = resp.matrix.iter().find(|c| {
                c.impact.as_deref() == Some(*imp) && c.likelihood.as_deref() == Some(*lk)
            });
            let text = match cell {
                Some(c) => {
                    let score = c
                        .score
                        .map(|s| format!("{s:.0}"))
                        .unwrap_or_else(|| "-".into());
                    match c.severity.as_deref().map(str::to_lowercase).as_deref() {
                        Some("critical") => score.red().bold().to_string(),
                        Some("high") => score.red().to_string(),
                        Some("medium") => score.yellow().to_string(),
                        Some("low") => score.green().to_string(),
                        _ => score,
                    }
                }
                None => "·".bright_black().to_string(),
            };
            row.push_str(&output::pad(&text, CELL));
        }
        println!("{row}");
    }

    println!();
    println!(
        "    {} {}  {}  {}  {}",
        "score colour:".bright_black(),
        "low".green(),
        "medium".yellow(),
        "high".red(),
        "critical".red().bold()
    );
    output::count_paged("risk matrix cell(s)", resp.matrix.len(), None, None);
    println!();
    Ok(())
}

// ── evidence ────────────────────────────────────────────────────────────

async fn evidence(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/incidents/{id}/evidence");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let e: EvidenceResponse = client.get(&path).await?;

    output::section("◈", &format!("Evidence: {}", id.bold()));

    let alerts = e.alert_ids.as_deref().unwrap_or_default();
    let logs = e.compliance_log_ids.as_deref().unwrap_or_default();
    let snapshot = e.system_snapshot.as_ref().filter(|v| !v.is_null());

    if alerts.is_empty() && logs.is_empty() && snapshot.is_none() {
        output::info("No evidence linked to this incident.");
        println!();
        return Ok(());
    }

    if !alerts.is_empty() {
        output::subsection(&format!("Linked alerts ({})", alerts.len()));
        for a in alerts {
            output::field("alert", a);
        }
    }
    if !logs.is_empty() {
        output::subsection(&format!("Compliance logs ({})", logs.len()));
        for l in logs {
            output::field("log", l);
        }
    }
    if let Some(v) = snapshot {
        output::subsection("System snapshot");
        render_snapshot(v);
    } else {
        println!();
        output::info("No system snapshot captured for this incident.");
    }

    println!();
    Ok(())
}
