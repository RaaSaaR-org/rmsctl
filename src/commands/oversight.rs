use crate::cli::OversightCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Identity the CLI writes into the oversight audit log. The server defaults
/// `operatorId` to "system" when the body omits it, which makes every CLI
/// action indistinguishable from an automated one.
const OPERATOR: &str = "rmsctl";

pub async fn run(cmd: &OversightCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        OversightCommand::Dashboard => dashboard(client, ctx).await,
        OversightCommand::Fleet => fleet(client, ctx).await,
        OversightCommand::Anomalies { active, unacked } => {
            anomalies(client, ctx, *active, *unacked).await
        }
        OversightCommand::AckAnomaly { id } => ack_anomaly(client, ctx, id).await,
        OversightCommand::ResolveAnomaly { id, notes } => {
            resolve_anomaly(client, ctx, id, notes.as_deref()).await
        }
        OversightCommand::ManualSessions => manual_sessions(client, ctx).await,
        OversightCommand::ManualMode {
            robot_id,
            reason,
            off,
        } => manual_mode(client, ctx, robot_id, reason.as_deref(), *off).await,
        OversightCommand::Verifications { due } => verifications(client, ctx, *due).await,
        OversightCommand::Capabilities { robot_id } => capabilities(client, ctx, robot_id).await,
        OversightCommand::Logs => logs(client, ctx).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SeverityCounts {
    #[serde(default)]
    critical: Option<u32>,
    #[serde(default)]
    high: Option<u32>,
    #[serde(default)]
    medium: Option<u32>,
    #[serde(default)]
    low: Option<u32>,
}

impl SeverityCounts {
    /// Worst first — alphabetical order from a map would put "critical" next to
    /// "high" and bury nothing, but "low" would lead on a plain sort.
    fn rows(&self) -> [(&'static str, u32); 4] {
        [
            ("critical", self.critical.unwrap_or(0)),
            ("high", self.high.unwrap_or(0)),
            ("medium", self.medium.unwrap_or(0)),
            ("low", self.low.unwrap_or(0)),
        ]
    }

    fn total(&self) -> u32 {
        self.rows().iter().map(|(_, n)| n).sum()
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Anomaly {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    robot_name: Option<String>,
    #[serde(default)]
    anomaly_type: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    detected_at: Option<String>,
    #[serde(default)]
    acknowledged_at: Option<String>,
    #[serde(default)]
    acknowledged_by: Option<String>,
    #[serde(default)]
    resolved_at: Option<String>,
    #[serde(default)]
    resolution: Option<String>,
    #[serde(default)]
    is_active: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnomalyListResponse {
    #[serde(default)]
    anomalies: Vec<Anomaly>,
    #[serde(default)]
    total: Option<u32>,
    #[serde(default)]
    total_pages: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct OversightLog {
    #[serde(default)]
    action_type: Option<String>,
    #[serde(default)]
    operator_id: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    robot_name: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct OversightLogsResponse {
    #[serde(default)]
    logs: Vec<OversightLog>,
    #[serde(default)]
    total: Option<u32>,
    #[serde(default)]
    total_pages: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardStats {
    #[serde(default)]
    active_manual_sessions: Option<u32>,
    #[serde(default)]
    manual_sessions_today: Option<u32>,
    #[serde(default)]
    active_anomalies: Option<u32>,
    #[serde(default)]
    unacknowledged_anomalies: Option<u32>,
    #[serde(default)]
    anomalies_by_severity: Option<SeverityCounts>,
    #[serde(default)]
    anomalies_by_type: Option<BTreeMap<String, u32>>,
    #[serde(default)]
    total_verification_schedules: Option<u32>,
    #[serde(default)]
    overdue_verifications: Option<u32>,
    #[serde(default)]
    completed_verifications_today: Option<u32>,
    #[serde(default)]
    verification_compliance_rate: Option<f64>,
    #[serde(default)]
    recent_logs: Vec<OversightLog>,
    #[serde(default)]
    recent_anomalies: Vec<Anomaly>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FleetRobot {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    operating_mode: Option<String>,
    #[serde(default)]
    has_anomalies: Option<bool>,
    #[serde(default)]
    battery_level: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FleetOverview {
    #[serde(default)]
    total_robots: Option<u32>,
    #[serde(default)]
    online_robots: Option<u32>,
    #[serde(default)]
    robots_in_manual_mode: Option<u32>,
    #[serde(default)]
    robots_with_anomalies: Option<u32>,
    #[serde(default)]
    average_confidence: Option<f64>,
    #[serde(default)]
    average_battery_level: Option<f64>,
    #[serde(default)]
    status_breakdown: Option<BTreeMap<String, u32>>,
    #[serde(default)]
    mode_breakdown: Option<BTreeMap<String, u32>>,
    #[serde(default)]
    total_active_anomalies: Option<u32>,
    #[serde(default)]
    anomalies_by_severity: Option<SeverityCounts>,
    #[serde(default)]
    overdue_verifications: Option<u32>,
    #[serde(default)]
    due_verifications_today: Option<u32>,
    #[serde(default)]
    robots: Vec<FleetRobot>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManualSession {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    robot_name: Option<String>,
    #[serde(default)]
    operator_id: Option<String>,
    #[serde(default)]
    operator_name: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    ended_at: Option<String>,
    #[serde(default)]
    is_active: Option<bool>,
    #[serde(default)]
    speed_limit_mm_per_sec: Option<f64>,
    #[serde(default)]
    force_limit_n: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManualModeRobot {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    previous_mode: Option<String>,
    #[serde(default)]
    new_mode: Option<String>,
}

/// POST /robots/:id/manual-mode answers `{session, robot}`; the DELETE answers
/// a bare session, so the release path parses `ManualSession` directly.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManualModeResult {
    #[serde(default)]
    session: Option<ManualSession>,
    #[serde(default)]
    robot: Option<ManualModeRobot>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerificationSchedule {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    interval_minutes: Option<i64>,
    #[serde(default)]
    robot_scope: Option<String>,
    #[serde(default)]
    scope_id: Option<String>,
    #[serde(default)]
    is_active: Option<bool>,
    #[serde(default)]
    last_completed_at: Option<String>,
    #[serde(default)]
    next_due_at: Option<String>,
    #[serde(default)]
    is_overdue: Option<bool>,
    #[serde(default)]
    completion_rate: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct VerificationCompletion {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    operator_id: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DueVerification {
    #[serde(default)]
    schedule: Option<VerificationSchedule>,
    #[serde(default)]
    last_completion: Option<VerificationCompletion>,
    #[serde(default)]
    due_at: Option<String>,
    #[serde(default)]
    overdue_since_minutes: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Capability {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    is_available: Option<bool>,
    #[serde(default)]
    confidence_level: Option<f64>,
    #[serde(default)]
    limitations: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitiesSummary {
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    robot_name: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    firmware: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    operating_mode: Option<String>,
    #[serde(default)]
    is_in_manual_mode: Option<bool>,
    #[serde(default)]
    manual_session: Option<ManualSession>,
    #[serde(default)]
    capabilities: Vec<Capability>,
    #[serde(default)]
    limitations: Vec<String>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    errors: Vec<String>,
    #[serde(default)]
    battery_level: Option<f64>,
    #[serde(default)]
    cpu_usage: Option<f64>,
    #[serde(default)]
    memory_usage: Option<f64>,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    overall_confidence: Option<f64>,
    #[serde(default)]
    recent_decision_accuracy: Option<f64>,
    #[serde(default)]
    active_anomalies: Vec<Anomaly>,
}

// ── Formatting helpers ──────────────────────────────────────────────────

fn num(v: Option<u32>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "-".to_string())
}

/// Timestamps arrive as `2026-09-14T10:22:33.123Z`; a table column only has
/// room for minute precision.
fn when(ts: Option<&str>) -> String {
    match ts {
        Some(t) => {
            let short: String = t.chars().take(16).collect();
            short.replace('T', " ")
        }
        None => "-".to_string(),
    }
}

fn minutes(m: Option<i64>) -> String {
    match m {
        None => "-".to_string(),
        Some(m) if m <= 0 => "now".to_string(),
        Some(m) if m < 60 => format!("{m}m"),
        Some(m) if m < 1440 => match m % 60 {
            0 => format!("{}h", m / 60),
            rem => format!("{}h {rem}m", m / 60),
        },
        Some(m) => match (m % 1440) / 60 {
            0 => format!("{}d", m / 1440),
            h => format!("{}d {h}h", m / 1440),
        },
    }
}

/// `output::severity_indicator` only knows the alert vocabulary
/// (critical/error/warning/info). Anomalies are graded critical/high/medium/low,
/// so the middle two would come back with no colour at all.
fn anomaly_severity(s: Option<&str>) -> String {
    let raw = s.unwrap_or("unknown");
    match raw.to_lowercase().as_str() {
        "high" => format!("▲ {}", raw.to_uppercase().red()),
        "medium" => format!("◆ {}", raw.to_uppercase().yellow()),
        "low" => format!("● {}", raw.to_uppercase().blue()),
        _ => output::severity_indicator(raw),
    }
}

fn pct(v: Option<f64>) -> String {
    v.map(|n| format!("{n:.0}%"))
        .unwrap_or_else(|| "-".to_string())
}

fn severity_rank(s: Option<&str>) -> u8 {
    match s.map(str::to_lowercase).as_deref() {
        Some("critical") => 0,
        Some("high") => 1,
        Some("medium") => 2,
        Some("low") => 3,
        _ => 4,
    }
}

fn print_severity_rows(counts: &SeverityCounts) {
    for (name, n) in counts.rows() {
        output::stat_line(
            if n > 0 { "■" } else { "│" },
            name,
            &n.to_string(),
            match (name, n) {
                (_, 0) => "dim",
                ("critical", _) | ("high", _) => "red",
                ("medium", _) => "yellow",
                _ => "orange",
            },
        );
    }
}

/// Breakdown keys run to 20 characters (`manual_reduced_speed`), which shoves
/// the count out of `stat_line`'s 18-wide label. Pad ANSI-aware instead.
fn breakdown_row(key: &str, n: u32) {
    println!(
        "    {} {} {}",
        "│".bright_black(),
        output::pad(key, 22),
        if n > 0 {
            n.to_string().normal()
        } else {
            n.to_string().bright_black()
        }
    );
}

/// One dim-bulleted line inside a detail block.
fn bullet(value: ColoredString) {
    println!(
        "    {} {} {}",
        "│".bright_black(),
        "•".bright_black(),
        value
    );
}

fn anomaly_table_header() {
    output::table_header(&[
        ("ID", output::ID_W),
        ("DETECTED", 17),
        ("SEVERITY", 11),
        ("TYPE", 22),
        ("ROBOT", 18),
        ("ACK", 4),
        ("DESCRIPTION", 30),
    ]);
}

fn anomaly_row(a: &Anomaly) {
    println!(
        "    {} {} {} {} {} {} {}",
        output::pad(a.id.as_deref().unwrap_or("-"), output::ID_W),
        output::pad(&when(a.detected_at.as_deref()), 17),
        output::pad(&anomaly_severity(a.severity.as_deref()), 11),
        output::pad(
            &output::truncate(a.anomaly_type.as_deref().unwrap_or("-"), 22),
            22
        ),
        output::pad(
            &output::truncate(
                a.robot_name
                    .as_deref()
                    .or(a.robot_id.as_deref())
                    .unwrap_or("-"),
                18
            ),
            18
        ),
        output::pad(
            &output::bool_indicator(Some(a.acknowledged_at.is_some())),
            4
        ),
        output::truncate(a.description.as_deref().unwrap_or("-"), 30)
    );
}

fn log_table_header() {
    output::table_header(&[
        ("WHEN", 17),
        ("ACTION", 26),
        ("OPERATOR", 16),
        ("ROBOT", 18),
        ("REASON", 34),
    ]);
}

fn log_row(l: &OversightLog) {
    println!(
        "    {} {} {} {} {}",
        output::pad(&when(l.timestamp.as_deref()), 17),
        output::pad(
            &output::truncate(l.action_type.as_deref().unwrap_or("-"), 26),
            26
        ),
        output::pad(
            &output::truncate(l.operator_id.as_deref().unwrap_or("-"), 16),
            16
        ),
        output::pad(
            &output::truncate(
                l.robot_name
                    .as_deref()
                    .or(l.robot_id.as_deref())
                    .unwrap_or("-"),
                18
            ),
            18
        ),
        output::truncate(l.reason.as_deref().unwrap_or("-"), 34)
    );
}

fn render_anomaly_detail(a: &Anomaly) {
    if a.severity.is_some() {
        output::field("Severity", &anomaly_severity(a.severity.as_deref()));
    }
    if let Some(t) = &a.anomaly_type {
        output::field("Type", t);
    }
    if let Some(robot) = a.robot_name.as_deref().or(a.robot_id.as_deref()) {
        output::field("Robot", robot);
    }
    if let Some(d) = &a.description {
        output::field("Description", d);
    }
    if let Some(at) = &a.detected_at {
        output::field("Detected", at);
    }
    if let Some(at) = &a.acknowledged_at {
        output::field("Acknowledged", at);
    }
    if let Some(by) = &a.acknowledged_by {
        output::field("Acked by", by);
    }
    if let Some(at) = &a.resolved_at {
        output::field("Resolved", at);
    }
    if let Some(r) = &a.resolution {
        output::field("Resolution", r);
    }
    output::field_colored(
        "Active",
        match a.is_active {
            Some(true) => "yes".red().bold(),
            Some(false) => "no".green(),
            None => "-".normal(),
        },
    );
}

/// The ISO 10218-1 limits the session was opened under matter as much as who
/// is driving: 250 mm/s reduced speed vs 1000 mm/s full speed.
fn render_session_limits(s: &ManualSession) {
    if let Some(op) = s.operator_name.as_deref().or(s.operator_id.as_deref()) {
        output::field("Operator", op);
    }
    if let Some(r) = &s.reason {
        output::field("Reason", r);
    }
    if let Some(at) = &s.started_at {
        output::field("Started", at);
    }
    if let Some(at) = &s.ended_at {
        output::field("Ended", at);
    }
    if let Some(v) = s.speed_limit_mm_per_sec {
        output::field("Speed limit", &format!("{v:.0} mm/s"));
    }
    if let Some(v) = s.force_limit_n {
        output::field("Force limit", &format!("{v:.0} N"));
    }
}

// ── dashboard ───────────────────────────────────────────────────────────

async fn dashboard(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/oversight/dashboard").await?);
    }

    let d: DashboardStats = client.get("/oversight/dashboard").await?;

    output::section("◈", "Human Oversight");

    output::subsection("Manual control");
    let manual = d.active_manual_sessions.unwrap_or(0);
    output::stat_line(
        if manual > 0 { "◐" } else { "│" },
        "Active sessions",
        &num(d.active_manual_sessions),
        if manual > 0 { "orange" } else { "dim" },
    );
    output::stat_line("│", "Sessions today", &num(d.manual_sessions_today), "dim");

    output::subsection("Anomalies");
    let active = d.active_anomalies.unwrap_or(0);
    let unacked = d.unacknowledged_anomalies.unwrap_or(0);
    output::stat_line(
        if active > 0 { "■" } else { "│" },
        "Active",
        &num(d.active_anomalies),
        if active > 0 { "red" } else { "dim" },
    );
    output::stat_line(
        if unacked > 0 { "!" } else { "│" },
        "Unacknowledged",
        &num(d.unacknowledged_anomalies),
        if unacked > 0 { "yellow" } else { "dim" },
    );
    if let Some(sev) = &d.anomalies_by_severity
        && sev.total() > 0
    {
        print_severity_rows(sev);
    }
    for (t, n) in d
        .anomalies_by_type
        .iter()
        .flatten()
        .filter(|(_, n)| **n > 0)
    {
        output::stat_line("│", t, &n.to_string(), "orange");
    }

    output::subsection("Verifications");
    let overdue = d.overdue_verifications.unwrap_or(0);
    output::stat_line(
        "│",
        "Schedules",
        &num(d.total_verification_schedules),
        "dim",
    );
    output::stat_line(
        if overdue > 0 { "■" } else { "│" },
        "Overdue",
        &num(d.overdue_verifications),
        if overdue > 0 { "red" } else { "dim" },
    );
    output::stat_line(
        "│",
        "Completed today",
        &num(d.completed_verifications_today),
        "dim",
    );
    if let Some(rate) = d.verification_compliance_rate {
        output::meter(
            "Compliance",
            rate,
            100.0,
            if rate >= 95.0 {
                "green"
            } else if rate >= 80.0 {
                "yellow"
            } else {
                "red"
            },
        );
    }

    if !d.recent_anomalies.is_empty() {
        output::subsection("Recent anomalies");
        anomaly_table_header();
        let mut recent: Vec<&Anomaly> = d.recent_anomalies.iter().collect();
        recent.sort_by_key(|a| severity_rank(a.severity.as_deref()));
        for a in recent {
            anomaly_row(a);
        }
    }

    if !d.recent_logs.is_empty() {
        output::subsection("Recent oversight actions");
        log_table_header();
        for l in &d.recent_logs {
            log_row(l);
        }
    }

    if d.recent_anomalies.is_empty() && d.recent_logs.is_empty() {
        println!();
        output::info("No anomalies or oversight actions recorded yet.");
    }

    println!();
    Ok(())
}

// ── fleet ───────────────────────────────────────────────────────────────

async fn fleet(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/oversight/fleet/overview").await?);
    }

    // Flat object, no wrapper key.
    let f: FleetOverview = client.get("/oversight/fleet/overview").await?;

    output::section("◈", "Fleet Oversight");

    let manual = f.robots_in_manual_mode.unwrap_or(0);
    let with_anom = f.robots_with_anomalies.unwrap_or(0);
    let overdue = f.overdue_verifications.unwrap_or(0);

    output::stat_line("│", "Robots", &num(f.total_robots), "orange");
    output::stat_line("│", "Online", &num(f.online_robots), "green");
    output::stat_line(
        if manual > 0 { "◐" } else { "│" },
        "In manual mode",
        &num(f.robots_in_manual_mode),
        if manual > 0 { "orange" } else { "dim" },
    );
    output::stat_line(
        if with_anom > 0 { "■" } else { "│" },
        "With anomalies",
        &num(f.robots_with_anomalies),
        if with_anom > 0 { "red" } else { "dim" },
    );
    output::stat_line(
        "│",
        "Active anomalies",
        &num(f.total_active_anomalies),
        if f.total_active_anomalies.unwrap_or(0) > 0 {
            "red"
        } else {
            "dim"
        },
    );
    output::stat_line(
        if overdue > 0 { "■" } else { "│" },
        "Overdue verif.",
        &num(f.overdue_verifications),
        if overdue > 0 { "red" } else { "dim" },
    );
    output::stat_line("│", "Due today", &num(f.due_verifications_today), "dim");

    if let Some(c) = f.average_confidence {
        output::meter(
            "Avg confidence",
            c,
            100.0,
            if c >= 80.0 {
                "green"
            } else if c >= 50.0 {
                "yellow"
            } else {
                "red"
            },
        );
    }
    if let Some(b) = f.average_battery_level {
        output::meter(
            "Avg battery",
            b,
            100.0,
            if b >= 50.0 {
                "green"
            } else if b >= 20.0 {
                "yellow"
            } else {
                "red"
            },
        );
    }

    if let Some(status) = f.status_breakdown.as_ref().filter(|m| !m.is_empty()) {
        output::subsection("Status");
        for (k, n) in status {
            breakdown_row(&output::status_indicator(k), *n);
        }
    }
    if let Some(modes) = f.mode_breakdown.as_ref().filter(|m| !m.is_empty()) {
        output::subsection("Operating mode");
        for (k, n) in modes {
            breakdown_row(&k.bright_black().to_string(), *n);
        }
    }
    if let Some(sev) = f.anomalies_by_severity.as_ref().filter(|s| s.total() > 0) {
        output::subsection("Anomalies by severity");
        print_severity_rows(sev);
    }

    if f.robots.is_empty() {
        println!();
        output::info("No robots in the oversight overview.");
        println!();
        return Ok(());
    }

    output::subsection("Robots");
    output::table_header(&[
        ("ROBOT", output::ID_W),
        ("NAME", 24),
        ("STATUS", 14),
        ("MODE", 22),
        ("BATTERY", 8),
        ("ANOM", 5),
    ]);
    for r in &f.robots {
        println!(
            "    {} {} {} {} {} {}",
            output::pad(r.id.as_deref().unwrap_or("-"), output::ID_W),
            output::pad(&output::truncate(r.name.as_deref().unwrap_or("-"), 24), 24),
            output::pad(
                &output::status_indicator(r.status.as_deref().unwrap_or("unknown")),
                14
            ),
            output::pad(
                &output::truncate(r.operating_mode.as_deref().unwrap_or("-"), 22),
                22
            ),
            output::pad(&pct(r.battery_level), 8),
            output::pad(&output::bool_indicator(r.has_anomalies), 5)
        );
    }

    output::count_paged("robot(s)", f.robots.len(), f.total_robots, None);
    println!();
    Ok(())
}

// ── anomalies ───────────────────────────────────────────────────────────

async fn anomalies(client: &ApiClient, ctx: &Ctx, active: bool, unacked: bool) -> RmsResult<()> {
    if active && unacked {
        return Err(RmsError::InvalidInput(
            "--active and --unacked are separate server views; pass one or neither".into(),
        ));
    }

    // The two filtered views are bare JSON arrays served whole; only the
    // unfiltered list is paginated, and it spells the page size `limit`.
    if active || unacked {
        let (path, label) = if active {
            ("/oversight/anomalies/active", "active anomaly/-ies")
        } else {
            (
                "/oversight/anomalies/unacknowledged",
                "unacknowledged anomaly/-ies",
            )
        };

        if ctx.json {
            return output::json_out(&client.get_value(path).await?);
        }

        let mut rows: Vec<Anomaly> = client.get(path).await?;
        rows.sort_by_key(|a| severity_rank(a.severity.as_deref()));

        output::section(
            "◈",
            if active {
                "Active Anomalies"
            } else {
                "Unacknowledged Anomalies"
            },
        );

        if ctx.has_paging() {
            output::info("This view is unpaginated server-side; --page/--limit are ignored.");
        }

        if rows.is_empty() {
            output::info("Nothing to review — no matching anomalies.");
            println!();
            return Ok(());
        }

        anomaly_table_header();
        for a in &rows {
            anomaly_row(a);
        }
        output::count_paged(label, rows.len(), None, None);
        println!();
        return Ok(());
    }

    let query = ctx.paging_limit();

    if ctx.json {
        return output::json_out(
            &client
                .get_value_query("/oversight/anomalies", &query)
                .await?,
        );
    }

    let resp: AnomalyListResponse = client.get_query("/oversight/anomalies", &query).await?;

    output::section("◈", "Anomalies");

    if resp.anomalies.is_empty() {
        output::info("No anomalies recorded.");
        println!();
        return Ok(());
    }

    let mut rows: Vec<&Anomaly> = resp.anomalies.iter().collect();
    rows.sort_by_key(|a| severity_rank(a.severity.as_deref()));

    anomaly_table_header();
    for a in rows {
        anomaly_row(a);
    }

    output::count_paged(
        "anomaly/-ies",
        resp.anomalies.len(),
        resp.total,
        resp.total_pages,
    );
    println!();
    Ok(())
}

async fn ack_anomaly(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    if !output::confirm(
        &format!("This will acknowledge anomaly {id} as seen, under operator '{OPERATOR}'"),
        ctx.yes,
    ) {
        return Ok(());
    }

    let body = serde_json::json!({ "operatorId": OPERATOR });
    let resp = client
        .post_value(&format!("/oversight/anomalies/{id}/acknowledge"), &body)
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    output::success(&format!("Anomaly {id} acknowledged"));
    // Acknowledging does not clear the anomaly: isActive stays true until it is
    // resolved, so show the state that came back rather than implying closure.
    if let Ok(a) = serde_json::from_value::<Anomaly>(resp) {
        render_anomaly_detail(&a);
    }
    println!();
    Ok(())
}

async fn resolve_anomaly(
    client: &ApiClient,
    ctx: &Ctx,
    id: &str,
    notes: Option<&str>,
) -> RmsResult<()> {
    // The server calls this field `resolution` and rejects the request without
    // it (400 "Resolution is required").
    let resolution = match notes.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => {
            return Err(RmsError::InvalidInput(
                "--notes is required: the server stores it as the anomaly's resolution".into(),
            ));
        }
    };

    if !output::confirm(
        &format!("This will RESOLVE anomaly {id} and close it out — resolution: {resolution}"),
        ctx.yes,
    ) {
        return Ok(());
    }

    let body = serde_json::json!({ "resolution": resolution, "operatorId": OPERATOR });
    let resp = client
        .post_value(&format!("/oversight/anomalies/{id}/resolve"), &body)
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    output::success(&format!("Anomaly {id} resolved"));
    if let Ok(a) = serde_json::from_value::<Anomaly>(resp) {
        render_anomaly_detail(&a);
    }
    println!();
    Ok(())
}

// ── manual control ──────────────────────────────────────────────────────

async fn manual_sessions(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/oversight/manual-sessions").await?);
    }

    // Bare array, and it is history: ended sessions come back alongside live ones.
    let rows: Vec<ManualSession> = client.get("/oversight/manual-sessions").await?;

    output::section("◈", "Manual Control Sessions");

    if ctx.has_paging() {
        output::info("This route is unpaginated server-side; --page/--limit are ignored.");
    }

    if rows.is_empty() {
        output::info("No manual-control sessions recorded — the fleet is fully autonomous.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("ID", output::ID_W),
        ("ROBOT", 20),
        ("OPERATOR", 16),
        ("STARTED", 17),
        ("ENDED", 17),
        ("LIVE", 5),
        ("REASON", 26),
    ]);
    for s in &rows {
        println!(
            "    {} {} {} {} {} {} {}",
            output::pad(s.id.as_deref().unwrap_or("-"), output::ID_W),
            output::pad(
                &output::truncate(
                    s.robot_name
                        .as_deref()
                        .or(s.robot_id.as_deref())
                        .unwrap_or("-"),
                    20
                ),
                20
            ),
            output::pad(
                &output::truncate(
                    s.operator_name
                        .as_deref()
                        .or(s.operator_id.as_deref())
                        .unwrap_or("-"),
                    16
                ),
                16
            ),
            output::pad(&when(s.started_at.as_deref()), 17),
            output::pad(&when(s.ended_at.as_deref()), 17),
            output::pad(&output::bool_indicator(s.is_active), 5),
            output::truncate(s.reason.as_deref().unwrap_or("-"), 26)
        );
    }

    for s in rows.iter().filter(|s| s.is_active == Some(true)) {
        output::subsection(&format!(
            "LIVE: {} under manual control",
            s.robot_name
                .as_deref()
                .or(s.robot_id.as_deref())
                .unwrap_or("-")
        ));
        render_session_limits(s);
    }

    output::count_paged("manual session(s)", rows.len(), None, None);
    println!();
    Ok(())
}

async fn manual_mode(
    client: &ApiClient,
    ctx: &Ctx,
    robot_id: &str,
    reason: Option<&str>,
    off: bool,
) -> RmsResult<()> {
    let path = format!("/oversight/robots/{robot_id}/manual-mode");

    if off {
        if reason.is_some() {
            output::warn(
                "--reason is ignored when releasing control; the DELETE route takes no body.",
            );
        }

        if !output::confirm(
            &format!(
                "This will RELEASE manual control of {robot_id}, returning it to autonomous operation"
            ),
            ctx.yes,
        ) {
            return Ok(());
        }

        // No body on DELETE, so the server logs the release under operatorId
        // "system" rather than rmsctl.
        let resp = client.delete_value(&path).await?;

        if ctx.json {
            return output::json_out(&resp);
        }

        output::success(&format!("Manual control released on {robot_id}"));
        if let Ok(s) = serde_json::from_value::<ManualSession>(resp) {
            render_session_limits(&s);
        }
        println!();
        return Ok(());
    }

    let reason = match reason.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => r.to_string(),
        None => {
            return Err(RmsError::InvalidInput(
                "--reason is required: the server rejects taking manual control without one".into(),
            ));
        }
    };

    if !output::confirm(
        &format!(
            "This will TAKE MANUAL CONTROL of {robot_id}, overriding autonomous operation at reduced speed — reason: {reason}"
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    // Omitting `mode` puts the robot in manual_reduced_speed (ISO 10218-1,
    // 250 mm/s); the full-speed variant is deliberately not exposed here.
    let body = serde_json::json!({ "reason": reason, "operatorId": OPERATOR });
    let resp = client.post_value(&path, &body).await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    output::success(&format!("Manual control engaged on {robot_id}"));
    if let Ok(result) = serde_json::from_value::<ManualModeResult>(resp) {
        if let Some(r) = &result.robot {
            if let Some(name) = &r.name {
                output::field("Robot", name);
            }
            if let (Some(prev), Some(new)) = (&r.previous_mode, &r.new_mode) {
                output::field("Mode", &format!("{prev} → {new}"));
            }
        }
        if let Some(s) = &result.session {
            if let Some(id) = &s.id {
                output::field("Session", id);
            }
            render_session_limits(s);
        }
    }
    output::info("Release it with: rmsctl oversight manual-mode <robot-id> --off --yes");
    println!();
    Ok(())
}

// ── verifications ───────────────────────────────────────────────────────

async fn verifications(client: &ApiClient, ctx: &Ctx, due: bool) -> RmsResult<()> {
    let path = if due {
        "/oversight/verifications/due"
    } else {
        "/oversight/verifications"
    };

    if ctx.json {
        return output::json_out(&client.get_value(path).await?);
    }

    if due {
        // Bare array, but each row wraps the schedule: {schedule, lastCompletion,
        // dueAt, overdueSinceMinutes}.
        let mut rows: Vec<DueVerification> = client.get(path).await?;
        rows.sort_by_key(|r| -r.overdue_since_minutes.unwrap_or(0));

        output::section("◈", "Due Verifications");

        if rows.is_empty() {
            output::info("Nothing due — every verification schedule is current.");
            println!();
            return Ok(());
        }

        output::table_header(&[
            ("SCHEDULE", output::ID_W),
            ("NAME", 26),
            ("DUE AT", 17),
            ("OVERDUE", 10),
            ("LAST DONE", 17),
            ("LAST STATUS", 12),
        ]);
        for r in &rows {
            let s = r.schedule.as_ref();
            let overdue = r.overdue_since_minutes.unwrap_or(0);
            let overdue_cell = if overdue > 0 {
                minutes(Some(overdue)).red().bold().to_string()
            } else {
                "due".yellow().to_string()
            };
            println!(
                "    {} {} {} {} {} {}",
                output::pad(s.and_then(|s| s.id.as_deref()).unwrap_or("-"), output::ID_W),
                output::pad(
                    &output::truncate(s.and_then(|s| s.name.as_deref()).unwrap_or("-"), 26),
                    26
                ),
                output::pad(&when(r.due_at.as_deref()), 17),
                output::pad(&overdue_cell, 10),
                output::pad(
                    &when(
                        r.last_completion
                            .as_ref()
                            .and_then(|c| c.completed_at.as_deref())
                    ),
                    17
                ),
                output::pad(
                    &output::status_indicator(
                        r.last_completion
                            .as_ref()
                            .and_then(|c| c.status.as_deref())
                            .unwrap_or("never"),
                    ),
                    12
                )
            );
        }

        let overdue_count = rows
            .iter()
            .filter(|r| r.overdue_since_minutes.unwrap_or(0) > 0)
            .count();
        if overdue_count > 0 {
            println!();
            output::warn(&format!(
                "{overdue_count} verification(s) are past their due time — AI Act Art. 14(3) automation-bias checks."
            ));
        }

        output::count_paged("due verification(s)", rows.len(), None, None);
        println!();
        return Ok(());
    }

    let rows: Vec<VerificationSchedule> = client.get(path).await?;

    output::section("◈", "Verification Schedules");

    if rows.is_empty() {
        output::info("No verification schedules configured.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("ID", output::ID_W),
        ("NAME", 26),
        ("EVERY", 10),
        ("SCOPE", 16),
        ("NEXT DUE", 17),
        ("ACTIVE", 7),
        ("OVERDUE", 7),
    ]);
    for s in &rows {
        let scope = match (s.robot_scope.as_deref(), s.scope_id.as_deref()) {
            (Some(kind), Some(id)) => format!("{kind}:{id}"),
            (Some(kind), None) => kind.to_string(),
            _ => "-".to_string(),
        };
        println!(
            "    {} {} {} {} {} {} {}",
            output::pad(s.id.as_deref().unwrap_or("-"), output::ID_W),
            output::pad(&output::truncate(s.name.as_deref().unwrap_or("-"), 26), 26),
            output::pad(&minutes(s.interval_minutes), 10),
            output::pad(&output::truncate(&scope, 16), 16),
            output::pad(&when(s.next_due_at.as_deref()), 17),
            output::pad(&output::bool_indicator(s.is_active), 7),
            output::pad(
                &match s.is_overdue {
                    Some(true) => "■ yes".red().bold().to_string(),
                    other => output::bool_indicator(other),
                },
                7
            )
        );
    }

    output::count_paged("verification schedule(s)", rows.len(), None, None);
    println!();
    Ok(())
}

// ── capabilities ────────────────────────────────────────────────────────

async fn capabilities(client: &ApiClient, ctx: &Ctx, robot_id: &str) -> RmsResult<()> {
    let path = format!("/oversight/robots/{robot_id}/capabilities");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let c: CapabilitiesSummary = client.get(&path).await?;

    output::section(
        "◈",
        &format!(
            "Oversight: {}",
            c.robot_name
                .as_deref()
                .or(c.robot_id.as_deref())
                .unwrap_or(robot_id)
                .bold()
        ),
    );

    output::field("Robot", c.robot_id.as_deref().unwrap_or(robot_id));
    if let Some(m) = &c.model {
        output::field("Model", m);
    }
    output::field("Firmware", c.firmware.as_deref().unwrap_or("unknown"));
    output::field(
        "Status",
        &output::status_indicator(c.status.as_deref().unwrap_or("unknown")),
    );
    output::field(
        "Mode",
        &output::status_indicator(c.operating_mode.as_deref().unwrap_or("unknown")),
    );
    output::field_colored(
        "Manual mode",
        match c.is_in_manual_mode {
            Some(true) => "HUMAN IN CONTROL".truecolor(255, 103, 0).bold(),
            Some(false) => "autonomous".green(),
            None => "-".normal(),
        },
    );
    if let Some(b) = c.battery_level {
        output::meter(
            "Battery",
            b,
            100.0,
            if b >= 50.0 {
                "green"
            } else if b >= 20.0 {
                "yellow"
            } else {
                "red"
            },
        );
    }
    if let Some(v) = c.overall_confidence {
        output::meter(
            "Confidence",
            v,
            100.0,
            if v >= 80.0 {
                "green"
            } else if v >= 50.0 {
                "yellow"
            } else {
                "red"
            },
        );
    }
    if let Some(v) = c.recent_decision_accuracy {
        output::field("Decision acc.", &pct(Some(v)));
    }
    // Host metrics are null unless the robot agent reports them.
    if let Some(v) = c.cpu_usage {
        output::field("CPU", &pct(Some(v)));
    }
    if let Some(v) = c.memory_usage {
        output::field("Memory", &pct(Some(v)));
    }
    if let Some(v) = c.temperature {
        output::field("Temperature", &format!("{v:.1} °C"));
    }

    if let Some(s) = &c.manual_session {
        output::subsection("Manual control session");
        if let Some(id) = &s.id {
            output::field("Session", id);
        }
        render_session_limits(s);
    }

    if c.capabilities.is_empty() {
        output::subsection("Capabilities");
        output::info("The server reports no capability list for this robot.");
    } else {
        output::subsection("Capabilities");
        output::table_header(&[
            ("OK", 3),
            ("CAPABILITY", 20),
            ("CONF", 6),
            ("DESCRIPTION", 44),
        ]);
        for cap in &c.capabilities {
            println!(
                "    {} {} {} {}",
                output::pad(&output::bool_indicator(cap.is_available), 3),
                output::pad(
                    &output::truncate(cap.name.as_deref().unwrap_or("-"), 20),
                    20
                ),
                output::pad(&pct(cap.confidence_level), 6),
                output::truncate(cap.description.as_deref().unwrap_or("-"), 44)
            );
            for l in cap.limitations.iter().flatten() {
                println!("        {} {}", "└".bright_black(), l.yellow());
            }
        }
    }

    if !c.limitations.is_empty() {
        output::subsection("Limitations");
        for l in &c.limitations {
            bullet(l.yellow());
        }
    }
    if !c.warnings.is_empty() {
        output::subsection("Warnings");
        for w in &c.warnings {
            bullet(w.yellow().bold());
        }
    }
    if !c.errors.is_empty() {
        output::subsection("Errors");
        for e in &c.errors {
            bullet(e.red().bold());
        }
    }

    if c.active_anomalies.is_empty() {
        output::subsection("Active anomalies");
        output::info("None.");
    } else {
        output::subsection("Active anomalies");
        anomaly_table_header();
        let mut rows: Vec<&Anomaly> = c.active_anomalies.iter().collect();
        rows.sort_by_key(|a| severity_rank(a.severity.as_deref()));
        for a in rows {
            anomaly_row(a);
        }
    }

    println!();
    Ok(())
}

// ── logs ────────────────────────────────────────────────────────────────

async fn logs(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    // This route spells the page size `limit`, not `pageSize`.
    let query = ctx.paging_limit();

    if ctx.json {
        return output::json_out(&client.get_value_query("/oversight/logs", &query).await?);
    }

    let resp: OversightLogsResponse = client.get_query("/oversight/logs", &query).await?;

    output::section("◈", "Oversight Audit Log");

    if resp.logs.is_empty() {
        output::info("No oversight actions logged.");
        println!();
        return Ok(());
    }

    log_table_header();
    for l in &resp.logs {
        log_row(l);
    }

    output::count_paged(
        "oversight action(s)",
        resp.logs.len(),
        resp.total,
        resp.total_pages,
    );
    println!();
    Ok(())
}
