use crate::cli::SafetyCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};

pub async fn run(cmd: &SafetyCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        SafetyCommand::Status { robot } => status(client, ctx, robot.as_deref()).await,
        SafetyCommand::Estop { id, fleet, reason } => {
            estop(client, ctx, id.as_deref(), *fleet, reason.as_deref()).await
        }
        SafetyCommand::Reset { id, fleet } => reset(client, ctx, id.as_deref(), *fleet).await,
        SafetyCommand::ZoneEstop { zone_id, reason } => {
            zone_estop(client, ctx, zone_id, reason.as_deref()).await
        }
        SafetyCommand::Events => events(client, ctx).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────

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
    server_connected: Option<bool>,
    #[serde(default)]
    system_healthy: Option<bool>,
    #[serde(default)]
    current_speed: Option<f64>,
    #[serde(default)]
    active_speed_limit: Option<f64>,
    #[serde(default)]
    active_force_limit: Option<f64>,
    #[serde(default)]
    warnings: Option<Vec<String>>,
    #[serde(default)]
    last_check_timestamp: Option<String>,
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SafetyEvent {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    event_type: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SafetyEventsResponse {
    #[serde(default)]
    events: Vec<SafetyEvent>,
    #[serde(default)]
    count: Option<u32>,
}

// ── status ──────────────────────────────────────────────────────────────

async fn status(client: &ApiClient, ctx: &Ctx, robot: Option<&str>) -> RmsResult<()> {
    match robot {
        Some(id) => {
            let path = format!("/safety/robots/{id}");
            if ctx.json {
                return output::json_out(&client.get_value(&path).await?);
            }
            let s: RobotSafety = client.get(&path).await?;
            output::section("◈", &format!("Safety: {}", id.bold()));
            render_robot(&s, true);
            println!();
            Ok(())
        }
        None => {
            if ctx.json {
                return output::json_out(&client.get_value("/safety/fleet").await?);
            }
            let fleet: FleetSafety = client.get("/safety/fleet").await?;

            output::section("◈", "Fleet Safety");

            if fleet.robots.is_empty() {
                output::info("No robots reporting safety state.");
                println!();
                return Ok(());
            }

            // Count what the robots actually report rather than trusting the
            // envelope: the server has been seen sending triggeredCount 0
            // alongside a robot whose estop.status is "triggered".
            let latched: Vec<&RobotSafety> = fleet
                .robots
                .iter()
                .filter(|r| r.estop.as_ref().is_some_and(|e| e.is_triggered()))
                .collect();
            let unhealthy = fleet
                .robots
                .iter()
                .filter(|r| r.system_healthy == Some(false))
                .count();

            output::stat_line("│", "Robots", &fleet.robots.len().to_string(), "orange");
            output::stat_line(
                if latched.is_empty() { "│" } else { "■" },
                "E-stop latched",
                &latched.len().to_string(),
                if latched.is_empty() { "dim" } else { "red" },
            );
            output::stat_line(
                "│",
                "Unhealthy",
                &unhealthy.to_string(),
                if unhealthy > 0 { "yellow" } else { "dim" },
            );

            if let (Some(reported), false) = (fleet.triggered_count, latched.is_empty())
                && reported as usize != latched.len()
            {
                println!();
                output::warn(&format!(
                    "Server reports triggeredCount={reported} but {} robot(s) report a latched E-stop.",
                    latched.len()
                ));
            }

            for r in &fleet.robots {
                println!();
                let name = r
                    .robot_name
                    .as_deref()
                    .or(r.robot_id.as_deref())
                    .unwrap_or("-");
                println!("    {} {}", "│".bright_black(), name.bold());
                render_robot(r, false);
            }

            println!();
            Ok(())
        }
    }
}

fn render_robot(s: &RobotSafety, with_id: bool) {
    if with_id && let Some(name) = &s.robot_name {
        output::field("Name", name);
    }

    match &s.estop {
        Some(e) if e.is_triggered() => {
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
                output::field("Triggered at", at);
            }
            if let Some(cat) = e.stop_category {
                output::field("Stop category", &cat.to_string());
            }
            if e.requires_manual_reset == Some(true) {
                output::field_colored("Reset", "manual reset required".yellow());
            }
        }
        Some(e) => output::field(
            "E-stop",
            &output::status_indicator(e.status.as_deref().unwrap_or("clear")),
        ),
        None => output::field("E-stop", "-"),
    }

    if let Some(mode) = &s.operating_mode {
        output::field("Mode", &output::status_indicator(mode));
    }
    if let Some(healthy) = s.system_healthy {
        output::field_colored(
            "System",
            if healthy {
                "healthy".green()
            } else {
                "UNHEALTHY".red().bold()
            },
        );
    }
    if let Some(connected) = s.server_connected {
        output::field_colored(
            "Server link",
            if connected {
                "connected".green()
            } else {
                "disconnected".red()
            },
        );
    }
    if let (Some(speed), Some(limit)) = (s.current_speed, s.active_speed_limit) {
        output::field("Speed", &format!("{speed:.0} / {limit:.0} limit"));
    }
    if let Some(force) = s.active_force_limit {
        output::field("Force limit", &format!("{force:.0}"));
    }
    for w in s.warnings.iter().flatten() {
        output::field_colored("Warning", w.yellow());
    }
    if let Some(ts) = &s.last_check_timestamp {
        output::field("Checked", ts);
    }
}

// ── estop / reset ───────────────────────────────────────────────────────

fn require_reason(reason: Option<&str>) -> RmsResult<String> {
    match reason.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => Ok(r.to_string()),
        None => Err(RmsError::InvalidInput(
            "--reason is required: the server rejects an E-stop without one".into(),
        )),
    }
}

async fn estop(
    client: &ApiClient,
    ctx: &Ctx,
    id: Option<&str>,
    fleet: bool,
    reason: Option<&str>,
) -> RmsResult<()> {
    let reason = require_reason(reason)?;

    let (path, what) = match (fleet, id) {
        (true, _) => (
            "/safety/fleet/estop".to_string(),
            "EVERY ROBOT IN THE FLEET".to_string(),
        ),
        (false, Some(id)) => (format!("/safety/robots/{id}/estop"), id.to_string()),
        (false, None) => {
            return Err(RmsError::InvalidInput(
                "give a robot ID, or pass --fleet to stop everything".into(),
            ));
        }
    };

    if !output::confirm(
        &format!("This will ENGAGE EMERGENCY STOP on {what} — reason: {reason}"),
        ctx.yes,
    ) {
        return Ok(());
    }

    let body = serde_json::json!({ "reason": reason, "triggeredBy": "rmsctl" });
    let resp = client.post_value(&path, &body).await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    output::success(&format!("Emergency stop engaged on {what}"));
    if let Some(msg) = resp.get("message").and_then(|m| m.as_str()) {
        output::info(msg);
    }
    println!();
    Ok(())
}

async fn reset(client: &ApiClient, ctx: &Ctx, id: Option<&str>, fleet: bool) -> RmsResult<()> {
    let (path, what) = match (fleet, id) {
        (true, _) => (
            "/safety/fleet/estop/reset".to_string(),
            "EVERY ROBOT IN THE FLEET".to_string(),
        ),
        (false, Some(id)) => (format!("/safety/robots/{id}/estop/reset"), id.to_string()),
        (false, None) => {
            return Err(RmsError::InvalidInput(
                "give a robot ID, or pass --fleet to reset everything".into(),
            ));
        }
    };

    if !output::confirm(
        &format!("This will CLEAR the emergency stop on {what}, allowing motion again"),
        ctx.yes,
    ) {
        return Ok(());
    }

    let resp = client.post_value(&path, &serde_json::json!({})).await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    output::success(&format!("Emergency stop reset on {what}"));
    println!();
    Ok(())
}

async fn zone_estop(
    client: &ApiClient,
    ctx: &Ctx,
    zone_id: &str,
    reason: Option<&str>,
) -> RmsResult<()> {
    let reason = require_reason(reason)?;

    if !output::confirm(
        &format!(
            "This will ENGAGE EMERGENCY STOP on every robot in zone {zone_id} — reason: {reason}"
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let body = serde_json::json!({ "reason": reason, "triggeredBy": "rmsctl" });
    let resp = client
        .post_value(&format!("/safety/zones/{zone_id}/estop"), &body)
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    output::success(&format!("Emergency stop engaged in zone {zone_id}"));
    println!();
    Ok(())
}

// ── events ──────────────────────────────────────────────────────────────

async fn events(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let query = ctx.paging_limit();

    if ctx.json {
        return output::json_out(&client.get_value_query("/safety/events", &query).await?);
    }

    let resp: SafetyEventsResponse = client.get_query("/safety/events", &query).await?;

    output::section("◈", "Safety Events");

    if resp.events.is_empty() {
        output::info("No safety events recorded.");
        println!();
        return Ok(());
    }

    output::table_header(&[("WHEN", 26), ("ROBOT", 22), ("TYPE", 20), ("REASON", 30)]);
    for e in &resp.events {
        println!(
            "    {} {} {} {}",
            output::pad(e.timestamp.as_deref().unwrap_or("-"), 26),
            output::pad(
                &output::truncate(e.robot_id.as_deref().unwrap_or("-"), 22),
                22
            ),
            output::pad(
                &output::status_indicator(e.event_type.as_deref().unwrap_or("-")),
                20
            ),
            output::truncate(e.reason.as_deref().unwrap_or("-"), 30)
        );
    }

    output::count_paged("safety event(s)", resp.events.len(), resp.count, None);
    println!();
    Ok(())
}
