use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::models::{AlertCountsResponse, Robot, RobotHealth, RobotsResponse};
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

pub async fn run(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return json_dashboard(client).await;
    }
    dashboard(client, ctx).await
}

// ── Response shapes ─────────────────────────────────────────────────────
//
// Only the safety panel needs local structs: the robot and alert-count
// envelopes are shared with `robots`/`alerts` and live in models.rs.

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EstopState {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    triggered_at: Option<String>,
    #[serde(default)]
    triggered_by: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    stop_category: Option<u32>,
    #[serde(default)]
    requires_manual_reset: Option<bool>,
}

impl EstopState {
    fn is_triggered(&self) -> bool {
        matches!(
            self.status.as_deref().map(str::to_lowercase).as_deref(),
            Some("triggered") | Some("engaged") | Some("active") | Some("latched")
        )
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RobotSafety {
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    robot_name: Option<String>,
    #[serde(default)]
    estop: Option<EstopState>,
    #[serde(default)]
    operating_mode: Option<String>,
    #[serde(default)]
    system_healthy: Option<bool>,
    #[serde(default)]
    server_connected: Option<bool>,
    #[serde(default)]
    warnings: Option<Vec<String>>,
}

impl RobotSafety {
    fn is_latched(&self) -> bool {
        self.estop.as_ref().is_some_and(|e| e.is_triggered())
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FleetSafety {
    #[serde(default)]
    robots: Vec<RobotSafety>,
    #[serde(default)]
    any_triggered: Option<bool>,
    #[serde(default)]
    triggered_count: Option<u32>,
    #[serde(default)]
    timestamp: Option<String>,
}

// ── --json ──────────────────────────────────────────────────────────────

/// `--json` is a passthrough: each panel is the server's own body, verbatim.
///
/// The old dashboard re-serialized the typed `Robot`, which dropped every
/// field the struct lacked (metadata, createdAt, currentTaskName, the a2a
/// wiring) and invented two nulls the server never sent.
async fn json_dashboard(client: &ApiClient) -> RmsResult<()> {
    let (robots, counts, safety) = tokio::join!(
        client.get_value("/robots"),
        client.get_value("/alerts/counts"),
        client.get_value("/safety/fleet"),
    );

    // The fleet roster is the one panel the document is meaningless without;
    // the other two degrade to an inline `error` key so a 500 on the alert
    // service does not empty the whole payload.
    let robots = robots?;

    let mut out = serde_json::Map::new();
    out.insert("robots".to_string(), robots);
    out.insert("alertCounts".to_string(), panel(counts));
    out.insert("safety".to_string(), panel(safety));
    output::json_out(&Value::Object(out))
}

fn panel(result: RmsResult<Value>) -> Value {
    match result {
        Ok(v) => v,
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    }
}

// ── dashboard ───────────────────────────────────────────────────────────

async fn dashboard(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let (robots_res, counts_res, safety_res) = tokio::join!(
        client.get::<RobotsResponse>("/robots"),
        client.get::<AlertCountsResponse>("/alerts/counts"),
        client.get::<FleetSafety>("/safety/fleet"),
    );

    // Partial failure degrades instead of aborting: this is the one command
    // that is explicitly an aggregate, so a 500 from the alert service must
    // not throw away a fleet roster that already arrived.
    let fleet = robots_res.map_err(annotate_robots_failure)?;
    let robots = &fleet.robots;

    output::banner();

    let safety = safety_res;
    let latched: Vec<&RobotSafety> = match &safety {
        Ok(s) => s.robots.iter().filter(|r| r.is_latched()).collect(),
        Err(_) => Vec::new(),
    };
    // /safety/fleet is authoritative for a latched stop, and it disagrees with
    // /robots: SimBot-01 reports status "online" while its E-stop is engaged.
    let latched_ids: HashSet<&str> = latched
        .iter()
        .filter_map(|s| s.robot_id.as_deref())
        .collect();

    if !latched.is_empty() {
        render_estop_headline(&latched, robots, safety.as_ref().ok());
    }

    render_fleet(robots, &latched_ids, &safety);
    render_alerts(&counts_res);
    render_units(robots, &latched_ids);

    if ctx.has_paging() {
        // GET /robots takes `_req` and synthesises its pagination block from
        // the array it just built, so page/pageSize are silently ignored.
        output::info("--page/--limit do not apply: the fleet overview always covers every robot.");
    }
    output::count_paged(
        "robot(s) registered",
        robots.len(),
        fleet.pagination.as_ref().and_then(|p| p.total()),
        fleet.pagination.as_ref().and_then(|p| p.total_pages()),
    );
    println!();
    Ok(())
}

fn annotate_robots_failure(e: RmsError) -> RmsError {
    match e {
        RmsError::Api { status, message } => RmsError::Api {
            status,
            message: format!("{message} (GET /robots — the fleet roster is required)"),
        },
        other => other,
    }
}

// ── E-stop headline ─────────────────────────────────────────────────────

fn render_estop_headline(latched: &[&RobotSafety], robots: &[Robot], fleet: Option<&FleetSafety>) {
    output::section("■", "E-STOP LATCHED");
    output::warn(
        &format!(
            "{} robot(s) are held in emergency stop and cannot move.",
            latched.len()
        )
        .red()
        .bold()
        .to_string(),
    );

    for s in latched {
        println!();
        println!(
            "    {} {}",
            "│".bright_black(),
            display_name(s, robots).red().bold()
        );
        if let Some(id) = &s.robot_id {
            output::field("ID", id);
        }
        if let Some(e) = &s.estop {
            output::field_colored(
                "E-stop",
                format!(
                    "■ {}",
                    e.status.as_deref().unwrap_or("triggered").to_uppercase()
                )
                .red()
                .bold(),
            );
            if let Some(reason) = &e.reason {
                output::field("Reason", reason);
            }
            if let Some(by) = &e.triggered_by {
                output::field("Triggered by", by);
            }
            if let Some(at) = &e.triggered_at {
                output::field("Triggered at", &short_time(at));
            }
            if let Some(cat) = e.stop_category {
                output::field("Stop category", &cat.to_string());
            }
            if e.requires_manual_reset == Some(true) {
                output::field_colored("Reset", "manual reset required".yellow());
            }
        }
        if let Some(mode) = &s.operating_mode {
            output::field("Mode", &output::status_indicator(mode));
        }
        if s.system_healthy == Some(false) {
            output::field_colored("System", "UNHEALTHY".red().bold());
        }
        // The roster's own status is what the old dashboard trusted; print the
        // mismatch rather than hiding it.
        if let Some(r) = s.robot_id.as_deref().and_then(|id| find_robot(robots, id))
            && let Some(st) = &r.status
            && !matches!(r.health(), RobotHealth::Stopped)
        {
            output::field_colored(
                "Roster says",
                format!("{st} — /robots does not reflect the stop").yellow(),
            );
        }
        for w in s.warnings.iter().flatten() {
            output::field_colored("Warning", w.yellow());
        }
    }

    // Live quirk: anyTriggered stays false and triggeredCount stays 0 while a
    // robot entry reports estop.status "triggered", so the envelope's summary
    // cannot be used as the alarm.
    if let Some(f) = fleet {
        let disagrees = f.any_triggered == Some(false)
            || f.triggered_count
                .is_some_and(|c| c as usize != latched.len());
        if disagrees {
            println!();
            output::warn(&format!(
                "Server summary disagrees: anyTriggered={}, triggeredCount={} vs {} latched robot(s) reported.",
                f.any_triggered
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into()),
                f.triggered_count
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into()),
                latched.len()
            ));
        }
    }
    println!();
}

// ── fleet overview ──────────────────────────────────────────────────────

#[derive(Default)]
struct Buckets {
    online: usize,
    working: usize,
    stopped: usize,
    error: usize,
    offline: usize,
    other: usize,
    /// Raw status strings that fell through to `Other`, so a value the CLI
    /// predates (e.g. `maintenance`) is named instead of just counted.
    other_labels: Vec<String>,
    available: usize,
}

/// Bucket a robot, letting a latched E-stop override the roster's status.
fn bucket(r: &Robot, latched_ids: &HashSet<&str>) -> RobotHealth {
    if latched_ids.contains(r.id.as_str()) {
        return RobotHealth::Stopped;
    }
    r.health()
}

fn tally(robots: &[Robot], latched_ids: &HashSet<&str>) -> Buckets {
    let mut b = Buckets::default();
    for r in robots {
        let health = bucket(r, latched_ids);
        match health {
            RobotHealth::Online => b.online += 1,
            RobotHealth::Working => b.working += 1,
            RobotHealth::Stopped => b.stopped += 1,
            RobotHealth::Error => b.error += 1,
            RobotHealth::Offline => b.offline += 1,
            RobotHealth::Other => {
                b.other += 1;
                let raw = r.status.clone().unwrap_or_else(|| "unknown".to_string());
                if !b.other_labels.contains(&raw) {
                    b.other_labels.push(raw);
                }
            }
        }
        if health.is_available() {
            b.available += 1;
        }
    }
    b
}

fn render_fleet(robots: &[Robot], latched_ids: &HashSet<&str>, safety: &RmsResult<FleetSafety>) {
    output::section("◈", "Fleet Overview");

    if robots.is_empty() {
        output::info("No robots registered. Add one with: rmsctl robots register --help");
        println!();
        return;
    }

    let b = tally(robots, latched_ids);
    let total = robots.len();

    output::stat_line("│", "Total units", &total.to_string(), "orange");
    output::stat_line(
        "●",
        "Online",
        &b.online.to_string(),
        dim_if(b.online, "green"),
    );
    output::stat_line(
        "◐",
        "Working",
        &b.working.to_string(),
        dim_if(b.working, "orange"),
    );
    output::stat_line(
        "■",
        "E-stopped",
        &b.stopped.to_string(),
        dim_if(b.stopped, "red"),
    );
    output::stat_line("●", "Error", &b.error.to_string(), dim_if(b.error, "red"));
    output::stat_line(
        "●",
        "Offline",
        &b.offline.to_string(),
        dim_if(b.offline, "yellow"),
    );
    output::stat_line(
        "○",
        "Other",
        &b.other.to_string(),
        dim_if(b.other, "yellow"),
    );

    match safety {
        Ok(f) => output::stat_line(
            "│",
            "Unhealthy (safety)",
            &f.robots
                .iter()
                .filter(|r| r.system_healthy == Some(false))
                .count()
                .to_string(),
            if f.robots.iter().any(|r| r.system_healthy == Some(false)) {
                "yellow"
            } else {
                "dim"
            },
        ),
        Err(_) => output::stat_line("│", "Unhealthy (safety)", "unavailable", "dim"),
    }

    println!();
    if let Err(e) = safety {
        output::warn(&format!(
            "safety state unavailable: {e} — latched E-stops were NOT checked"
        ));
        println!();
    }
    // Availability counts the reachable, unlatched fleet — a stopped robot is
    // not available however healthy its roster entry looks.
    output::meter(
        "Availability",
        b.available as f64,
        total as f64,
        availability_color(b.available, total),
    );

    let batteries: Vec<f64> = robots.iter().filter_map(|r| r.battery_level).collect();
    if !batteries.is_empty() {
        let avg = batteries.iter().sum::<f64>() / batteries.len() as f64;
        output::meter("Avg battery", avg, 100.0, battery_color(avg));
    }

    // A 0%-filled bar has no colour to show, so the outage also gets words.
    if total > 0 && (b.available as f64) / (total as f64) < 0.5 {
        println!();
        output::warn(&match b.available {
            0 => format!("no robot of {total} is available for work."),
            n => format!("only {n} of {total} robot(s) available for work."),
        });
    }

    if b.other > 0 && !b.other_labels.is_empty() {
        println!();
        output::info(&format!(
            "unbucketed status value(s): {} — this rmsctl predates them",
            b.other_labels.join(", ")
        ));
    }
    println!();
}

fn dim_if(count: usize, color: &'static str) -> &'static str {
    if count > 0 { color } else { "dim" }
}

/// Threshold colouring, so a fully down fleet reads red rather than amber.
fn availability_color(available: usize, total: usize) -> &'static str {
    if total == 0 {
        return "dim";
    }
    let pct = available as f64 / total as f64;
    if pct >= 0.9 {
        "green"
    } else if pct >= 0.5 {
        "yellow"
    } else {
        "red"
    }
}

fn battery_color(avg: f64) -> &'static str {
    if avg >= 60.0 {
        "green"
    } else if avg >= 25.0 {
        "yellow"
    } else {
        "red"
    }
}

// ── alerts ──────────────────────────────────────────────────────────────

fn render_alerts(counts: &RmsResult<AlertCountsResponse>) {
    output::section("◈", "Alert Status");

    let c = match counts {
        Ok(resp) => &resp.counts,
        Err(e) => {
            output::warn(&format!("alert counts unavailable: {e}"));
            output::info("Fleet sections above are unaffected. Retry: rmsctl alerts list");
            println!();
            return;
        }
    };

    output::stat_line(
        "▲",
        "Critical",
        &c.critical.to_string(),
        dim_if_u32(c.critical, "red"),
    );
    output::stat_line(
        "▲",
        "Error",
        &c.error.to_string(),
        dim_if_u32(c.error, "red"),
    );
    output::stat_line(
        "◆",
        "Warning",
        &c.warning.to_string(),
        dim_if_u32(c.warning, "yellow"),
    );
    output::stat_line("●", "Info", &c.info.to_string(), "dim");
    output::stat_line("│", "Total", &c.total().to_string(), "orange");

    if c.critical + c.error > 0 {
        println!();
        output::warn(&format!(
            "{} alert(s) at error or above — see: rmsctl alerts list",
            c.critical + c.error
        ));
    }
    println!();
}

fn dim_if_u32(count: u32, color: &'static str) -> &'static str {
    if count > 0 { color } else { "dim" }
}

// ── registered units ────────────────────────────────────────────────────

fn render_units(robots: &[Robot], latched_ids: &HashSet<&str>) {
    if robots.is_empty() {
        return;
    }

    output::section("◈", "Registered Units");
    output::table_header(&[
        ("ID", output::ID_W),
        ("NAME", 20),
        ("STATUS", 20),
        ("BATT", 5),
        ("ZONE", 14),
        ("LAST SEEN", 17),
    ]);

    for r in robots {
        let name = r.name.as_deref().unwrap_or("-");
        let status = if latched_ids.contains(r.id.as_str()) {
            format!("■ {}", "E-STOP".red().bold())
        } else {
            output::status_indicator(r.status.as_deref().unwrap_or("unknown"))
        };
        let batt = match r.battery_level {
            Some(b) => format!("{b:.0}%"),
            None => "-".to_string(),
        };
        println!(
            "    {} {} {} {} {} {}",
            output::pad(&r.id, output::ID_W),
            output::pad(&output::truncate(name, 20), 20),
            output::pad(&status, 20),
            output::pad(&batt, 5),
            output::pad(&output::truncate(r.zone().unwrap_or("-"), 14), 14),
            output::truncate(&short_time(r.last_seen.as_deref().unwrap_or("-")), 17)
        );
        // Only the roster carries the task name, and it is the only clue about
        // what a busy robot is actually doing.
        if let Some(task) = r.current_task_name.as_deref().filter(|t| !t.is_empty()) {
            println!(
                "    {} {} {}",
                " ".repeat(output::ID_W - 1),
                "└".bright_black(),
                format!("task: {}", output::truncate(task, 60)).bright_black()
            );
        }
    }
}

// ── helpers ─────────────────────────────────────────────────────────────

fn find_robot<'a>(robots: &'a [Robot], id: &str) -> Option<&'a Robot> {
    robots.iter().find(|r| r.id == id)
}

fn display_name(s: &RobotSafety, robots: &[Robot]) -> String {
    if let Some(name) = s.robot_name.as_deref().filter(|n| !n.is_empty()) {
        return match s.robot_id.as_deref() {
            Some(id) => format!("{name} ({id})"),
            None => name.to_string(),
        };
    }
    let id = s.robot_id.as_deref().unwrap_or("-");
    match find_robot(robots, id).and_then(|r| r.name.as_deref()) {
        Some(name) => format!("{name} ({id})"),
        None => id.to_string(),
    }
}

/// `2026-09-13T22:00:24.320Z` -> `2026-09-13 22:00`; anything that is not an
/// ISO timestamp is passed through untouched.
fn short_time(ts: &str) -> String {
    match ts.split_once('T') {
        Some((date, time)) if date.len() == 10 => {
            let hm: String = time.chars().take(5).collect();
            format!("{date} {hm}")
        }
        _ => ts.to_string(),
    }
}
