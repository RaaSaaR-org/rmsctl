use crate::cli::PatrolCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};

pub async fn run(cmd: &PatrolCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        PatrolCommand::Routes => routes(client, ctx).await,
        PatrolCommand::Route { id } => route_show(client, ctx, id).await,
        PatrolCommand::Runs { route } => runs(client, ctx, route.as_deref()).await,
        PatrolCommand::Run { run_id } => run_show(client, ctx, run_id).await,
        PatrolCommand::Findings => findings(client, ctx).await,
        PatrolCommand::Finding { id } => finding_show(client, ctx, id).await,
        PatrolCommand::Places => places(client, ctx).await,
        PatrolCommand::Start {
            route_id,
            robot_id,
            mode,
        } => start(client, ctx, route_id, robot_id.as_deref(), mode.as_deref()).await,
        PatrolCommand::Abort {
            route_id,
            robot_id,
            reason,
        } => {
            abort(
                client,
                ctx,
                route_id,
                robot_id.as_deref(),
                reason.as_deref(),
            )
            .await
        }
        PatrolCommand::AckFinding { id } => {
            finding_action(client, ctx, id, FindingAction::Acknowledge).await
        }
        PatrolCommand::NormalFinding { id } => {
            finding_action(client, ctx, id, FindingAction::Normal).await
        }
        PatrolCommand::EscalateFinding { id } => {
            finding_action(client, ctx, id, FindingAction::Escalate).await
        }
        PatrolCommand::PromoteRun { run_id } => promote_run(client, ctx, run_id).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────
//
// Every patrol list route answers with a BARE ARRAY (no `data`/`items`
// wrapper and no pagination envelope), so the list types are plain `Vec<T>`.

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Checkpoint {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    place_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    heading_deg: Option<f64>,
    #[serde(default)]
    actions: Option<Vec<String>>,
    #[serde(default)]
    dwell_ms: Option<u64>,
    #[serde(default)]
    expectations: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimeWindow {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    start_hour: Option<u32>,
    #[serde(default)]
    end_hour: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Route {
    id: String,
    #[serde(default)]
    name: Option<String>,
    /// null when the route is not bound to a robot — `start` then needs
    /// `--robot-id`, exactly as the server's own resolveRobotId does.
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    twin_id: Option<String>,
    #[serde(default)]
    checkpoints: Vec<Checkpoint>,
    #[serde(default)]
    cron_expression: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    time_windows: Vec<TimeWindow>,
    #[serde(default)]
    home_place_id: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    last_fired_at: Option<String>,
    #[serde(default)]
    next_run_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Pose {
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
    #[serde(default)]
    yaw_deg: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Leg {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    checkpoint_id: Option<String>,
    #[serde(default)]
    place_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    inspection: Option<String>,
    #[serde(default)]
    photo_key: Option<String>,
    /// Why no control photo was kept: `person` (someone was in frame) or `error`.
    #[serde(default)]
    photo_dropped: Option<String>,
    #[serde(default)]
    finding_ids: Vec<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    pose: Option<Pose>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    finished_at: Option<String>,
}

/// One patrol run. The identifier is `runId`, not `id`.
///
/// `findings` only arrives on `GET /patrol/runs/{runId}`; the list route omits
/// it, so the same struct serves both.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Run {
    run_id: String,
    #[serde(default)]
    route_id: Option<String>,
    #[serde(default)]
    route_name: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    origin: Option<String>,
    #[serde(default)]
    window: Option<String>,
    #[serde(default)]
    status: Option<String>,
    /// Set on `skipped`/`aborted` runs: `unreachable: …`, `robot rejected
    /// start: …`, the operator's abort reason.
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    finished_at: Option<String>,
    #[serde(default)]
    legs: Vec<Leg>,
    #[serde(default)]
    finding_count: Option<u32>,
    #[serde(default)]
    plan_id: Option<String>,
    #[serde(default)]
    alert_id: Option<String>,
    #[serde(default)]
    promoted_at: Option<String>,
    #[serde(default)]
    findings: Vec<Finding>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Finding {
    id: String,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    route_id: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    checkpoint_id: Option<String>,
    #[serde(default)]
    leg_index: Option<u32>,
    #[serde(rename = "type", default)]
    finding_type: Option<String>,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    place: Option<String>,
    #[serde(default)]
    pose: Option<Pose>,
    #[serde(default)]
    at: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    /// Free-form per-source payload (label diff, checklist diff, photo keys),
    /// so it stays a Value rather than a struct that would drop fields.
    #[serde(default)]
    evidence: Option<serde_json::Value>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    alert_id: Option<String>,
    #[serde(default)]
    incident_id: Option<String>,
    /// Only `POST /findings/{id}/normal` sets this — it reports whether the
    /// robot was told to fold the finding into its baseline.
    #[serde(default)]
    robot_notified: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Place {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    place_type: Option<String>,
    #[serde(default)]
    confidence: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

/// `GET /patrol/places` is the one patrol read that wraps its array.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlacesResponse {
    #[serde(default)]
    places: Vec<Place>,
}

/// `POST /routes/{id}/start`. A robot that refuses a precondition (battery,
/// e-stop, unknown place, already busy) answers HTTP 200 with
/// `accepted: false` — success at the HTTP layer is not a started patrol.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartResult {
    #[serde(default)]
    accepted: Option<bool>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct OkResult {
    #[serde(default)]
    ok: Option<bool>,
    #[serde(default)]
    run_id: Option<String>,
}

// ── Local formatting ────────────────────────────────────────────────────

/// `2026-08-16T14:39:36.854Z` → `2026-08-16 14:39`, char-safe, for table cells.
fn stamp(iso: Option<&str>) -> String {
    match iso {
        Some(s) => s.chars().take(16).collect::<String>().replace('T', " "),
        None => "-".to_string(),
    }
}

/// Patrol grades findings `low|medium|high`; `severity_indicator` knows the
/// alert vocabulary (critical/error/warning/info) and renders all three as a
/// colourless `○`, which loses the only ranking a triage list has.
fn severity_cell(severity: Option<&str>) -> String {
    match severity {
        Some("high") => format!("{} {}", "▲", "HIGH".red().bold()),
        Some("medium") => format!("{} {}", "◆", "MEDIUM".yellow()),
        Some("low") => format!("{} {}", "●", "LOW".bright_black()),
        Some(other) => output::severity_indicator(other),
        None => output::severity_indicator("unknown"),
    }
}

/// `status_indicator` knows `open`, but not patrol's three terminal finding
/// statuses, and an escalated finding must not read as ordinary.
fn finding_status(status: &str) -> String {
    match status {
        "escalated" => format!("{} {}", "▲".bright_black(), status.red().bold()),
        "acknowledged" => format!("{} {}", "●".bright_black(), status.green()),
        "dismissed_normal" => format!("{} {}", "○".bright_black(), status.bright_black()),
        other => output::status_indicator(other),
    }
}

/// Outcome of a leg's checkpoint inspection. `changed` is the one that costs an
/// operator something, `recorded` means the leg wrote the baseline.
fn inspection_cell(inspection: Option<&str>) -> String {
    match inspection {
        Some("changed") => "changed".yellow().bold().to_string(),
        Some("error") => "error".red().to_string(),
        Some("recorded") => "recorded".truecolor(255, 103, 0).to_string(),
        Some("no_baseline") => "no_baseline".bright_black().to_string(),
        Some(other) => other.normal().to_string(),
        None => "-".bright_black().to_string(),
    }
}

/// None of the patrol list routes read `page` — they take `limit` only (runs
/// default 50, findings 100, capped at 1000) — so say so instead of printing
/// page one under a `--page 3` the server ignored.
fn note_limit_only(ctx: &Ctx, label: &str) {
    if ctx.page.is_some_and(|p| p > 1) {
        output::warn(&format!(
            "{label} has no page cursor — --page is ignored; widen the window with --limit N."
        ));
    }
}

// ── routes ──────────────────────────────────────────────────────────────

async fn routes(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/patrol/routes").await?);
    }

    let rows: Vec<Route> = client.get("/patrol/routes").await?;

    output::section("◈", "Patrol Routes");

    if ctx.has_paging() {
        output::warn("patrol routes is unpaginated — --page and --limit are ignored.");
    }

    if rows.is_empty() {
        output::info("No patrol routes defined.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("ROUTE ID", output::ID_W),
        ("NAME", 18),
        ("ROBOT", 17),
        ("CPS", 4),
        ("CRON", 12),
        ("ON", 3),
    ]);
    for r in &rows {
        println!(
            "    {} {} {} {} {} {}",
            output::pad(&r.id, output::ID_W),
            output::pad(&output::truncate(r.name.as_deref().unwrap_or("-"), 18), 18),
            output::pad(
                &output::truncate(r.robot_id.as_deref().unwrap_or("unbound"), 17),
                17
            ),
            output::pad(&r.checkpoints.len().to_string(), 4),
            output::pad(
                &output::truncate(r.cron_expression.as_deref().unwrap_or("manual"), 12),
                12
            ),
            output::bool_indicator(r.enabled),
        );
    }

    output::count_paged("patrol route(s)", rows.len(), None, None);
    println!();
    Ok(())
}

async fn route_show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/patrol/routes/{id}");
    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let r: Route = client.get(&path).await?;

    output::section(
        "◈",
        &format!(
            "Patrol Route: {}",
            r.name.as_deref().unwrap_or(&r.id).bold()
        ),
    );
    output::field("Route ID", &r.id);
    output::field("Robot", r.robot_id.as_deref().unwrap_or("unbound"));
    if let Some(twin) = &r.twin_id {
        output::field("Twin", twin);
    }
    output::field_colored(
        "Enabled",
        match r.enabled {
            Some(true) => "yes".green(),
            Some(false) => "no — cron will not fire".yellow(),
            None => "-".bright_black(),
        },
    );
    output::field("Schedule", r.cron_expression.as_deref().unwrap_or("manual"));
    output::field("Home place", r.home_place_id.as_deref().unwrap_or("-"));
    output::field("Last fired", &stamp(r.last_fired_at.as_deref()));
    output::field("Next run", &stamp(r.next_run_at.as_deref()));
    output::field("Created", &stamp(r.created_at.as_deref()));
    output::field("Updated", &stamp(r.updated_at.as_deref()));

    if r.checkpoints.is_empty() {
        output::subsection("Checkpoints");
        output::info("None — the route has nothing to visit.");
    } else {
        output::subsection(&format!("Checkpoints ({})", r.checkpoints.len()));
        output::table_header(&[
            ("#", 3),
            ("CHECKPOINT", 18),
            ("PLACE", 14),
            ("NAME", 20),
            ("HEAD", 6),
            ("ACTIONS", 20),
        ]);
        for (i, c) in r.checkpoints.iter().enumerate() {
            println!(
                "    {} {} {} {} {} {}",
                output::pad(&i.to_string(), 3),
                output::pad(&output::truncate(c.id.as_deref().unwrap_or("-"), 18), 18),
                output::pad(
                    &output::truncate(c.place_id.as_deref().unwrap_or("-"), 14),
                    14
                ),
                output::pad(&output::truncate(c.name.as_deref().unwrap_or("-"), 20), 20),
                output::pad(
                    &match c.heading_deg {
                        Some(h) => format!("{h:.0}°"),
                        None => "any".to_string(),
                    },
                    6
                ),
                output::truncate(
                    &c.actions
                        .as_ref()
                        .map(|a| a.join(", "))
                        .unwrap_or_else(|| "-".to_string()),
                    20
                ),
            );
            for e in c.expectations.iter().flatten() {
                println!("        {} {}", "·".bright_black(), e.bright_black());
            }
        }
    }

    if !r.time_windows.is_empty() {
        // Baselines are kept per window: a lit lamp is normal at 09:00 and a
        // finding at 03:00, so the windows say which reference a run compares to.
        output::subsection("Time windows");
        for w in &r.time_windows {
            let hours = match (w.start_hour, w.end_hour) {
                (Some(s), Some(e)) => format!("{s:02}:00 – {e:02}:00"),
                _ => "-".to_string(),
            };
            output::field(
                w.name.as_deref().or(w.id.as_deref()).unwrap_or("window"),
                &hours,
            );
        }
    }

    println!();
    Ok(())
}

// ── runs ────────────────────────────────────────────────────────────────

async fn runs(client: &ApiClient, ctx: &Ctx, route: Option<&str>) -> RmsResult<()> {
    let mut own: Vec<(&'static str, String)> = Vec::new();
    if let Some(r) = route {
        own.push(("routeId", r.to_string()));
    }
    let query = ctx.with_paging_limit(own);

    if ctx.json {
        return output::json_out(&client.get_value_query("/patrol/runs", &query).await?);
    }

    let rows: Vec<Run> = client.get_query("/patrol/runs", &query).await?;

    output::section(
        "◈",
        &match route {
            Some(r) => format!("Patrol Runs — route {r}"),
            None => "Patrol Runs".to_string(),
        },
    );
    note_limit_only(ctx, "patrol runs");

    if rows.is_empty() {
        // An unknown routeId is not an error here: the filter simply matches nothing.
        output::info(match route {
            Some(_) => "No runs for that route — check the ID with 'rmsctl patrol routes'.",
            None => "No patrol runs recorded.",
        });
        println!();
        return Ok(());
    }

    let running = rows
        .iter()
        .filter(|r| r.status.as_deref() == Some("running"))
        .count();
    let skipped = rows
        .iter()
        .filter(|r| matches!(r.status.as_deref(), Some("skipped") | Some("failed")))
        .count();

    output::stat_line(
        if running > 0 { "◐" } else { "│" },
        "Running now",
        &running.to_string(),
        if running > 0 { "orange" } else { "dim" },
    );
    output::stat_line(
        if skipped > 0 { "■" } else { "│" },
        "Skipped / failed",
        &skipped.to_string(),
        if skipped > 0 { "yellow" } else { "dim" },
    );
    println!();

    output::table_header(&[
        ("RUN ID", output::ID_W),
        ("ROUTE", 16),
        ("MODE", 8),
        ("STATUS", 10),
        ("FIND", 4),
        ("STARTED", 16),
    ]);
    for r in &rows {
        println!(
            "    {} {} {} {} {} {}",
            output::pad(&r.run_id, output::ID_W),
            output::pad(
                &output::truncate(r.route_name.as_deref().unwrap_or("-"), 16),
                16
            ),
            output::pad(r.mode.as_deref().unwrap_or("-"), 8),
            output::pad(
                &output::status_indicator(r.status.as_deref().unwrap_or("unknown")),
                10
            ),
            output::pad(
                &r.finding_count
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                4
            ),
            stamp(r.started_at.as_deref()),
        );
    }

    // A skipped run carries the only explanation of a patrol that never happened.
    for r in rows
        .iter()
        .filter(|r| matches!(r.status.as_deref(), Some("skipped") | Some("failed")))
    {
        if let Some(reason) = &r.reason {
            println!();
            output::warn(&format!("{} — {reason}", r.run_id));
        }
    }

    output::count_paged("patrol run(s)", rows.len(), None, None);
    println!();
    Ok(())
}

async fn run_show(client: &ApiClient, ctx: &Ctx, run_id: &str) -> RmsResult<()> {
    let path = format!("/patrol/runs/{run_id}");
    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let r: Run = client.get(&path).await?;

    output::section(
        "◈",
        &format!(
            "Patrol Run: {}",
            r.route_name.as_deref().unwrap_or(&r.run_id).bold()
        ),
    );
    output::field("Run ID", &r.run_id);
    output::field("Route ID", r.route_id.as_deref().unwrap_or("-"));
    output::field("Robot", r.robot_id.as_deref().unwrap_or("-"));
    output::field(
        "Status",
        &output::status_indicator(r.status.as_deref().unwrap_or("unknown")),
    );
    output::field("Mode", r.mode.as_deref().unwrap_or("-"));
    output::field("Origin", r.origin.as_deref().unwrap_or("-"));
    output::field("Window", r.window.as_deref().unwrap_or("-"));
    output::field("Started", r.started_at.as_deref().unwrap_or("-"));
    output::field("Finished", r.finished_at.as_deref().unwrap_or("-"));
    if let Some(reason) = &r.reason {
        output::field_colored("Reason", reason.yellow());
    }
    if let Some(plan) = &r.plan_id {
        output::field("Plan", plan);
    }
    if let Some(alert) = &r.alert_id {
        output::field("Alert", alert);
    }
    match &r.promoted_at {
        Some(at) => {
            output::field_colored("Baseline", format!("promoted {}", stamp(Some(at))).green())
        }
        None => output::field("Baseline", "not promoted"),
    }

    if !r.legs.is_empty() {
        output::subsection(&format!("Legs ({})", r.legs.len()));
        output::table_header(&[
            ("#", 3),
            ("CHECKPOINT", 16),
            ("PLACE", 12),
            ("STATUS", 9),
            ("INSPECTION", 12),
            ("FIND", 4),
            ("MESSAGE", 30),
        ]);
        for l in &r.legs {
            println!(
                "    {} {} {} {} {} {} {}",
                output::pad(
                    &l.index
                        .map(|i| i.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    3
                ),
                output::pad(
                    &output::truncate(l.checkpoint_id.as_deref().unwrap_or("-"), 16),
                    16
                ),
                output::pad(
                    &output::truncate(l.place_id.as_deref().unwrap_or("-"), 12),
                    12
                ),
                output::pad(l.status.as_deref().unwrap_or("-"), 9),
                output::pad(&inspection_cell(l.inspection.as_deref()), 12),
                output::pad(&l.finding_ids.len().to_string(), 4),
                output::truncate(l.message.as_deref().unwrap_or("-"), 30),
            );
            if let Some(dropped) = &l.photo_dropped {
                output::warn(&format!(
                    "leg {} kept no control photo ({dropped}) — nothing to compare",
                    l.index.unwrap_or_default()
                ));
            }
        }
    }

    if r.findings.is_empty() {
        output::subsection("Findings");
        output::info(if r.mode.as_deref() == Some("baseline") {
            "None — a baseline run records the reference, it does not compare."
        } else {
            "None — the run matched its baseline."
        });
    } else {
        output::subsection(&format!("Findings ({})", r.findings.len()));
        output::table_header(&[
            ("FINDING ID", output::ID_W),
            ("SEV", 9),
            ("TYPE", 17),
            ("SUMMARY", 29),
        ]);
        for f in &r.findings {
            println!(
                "    {} {} {} {}",
                output::pad(&f.id, output::ID_W),
                output::pad(&severity_cell(f.severity.as_deref()), 9),
                output::pad(
                    &output::truncate(f.finding_type.as_deref().unwrap_or("-"), 17),
                    17
                ),
                output::truncate(f.summary.as_deref().unwrap_or("-"), 29),
            );
        }
    }

    // findingCount is stored on the run; the findings array is rebuilt per
    // request, so a mismatch means findings were pruned behind the run.
    if let Some(count) = r.finding_count
        && count as usize != r.findings.len()
    {
        println!();
        output::warn(&format!(
            "Run records findingCount={count} but {} finding(s) are readable.",
            r.findings.len()
        ));
    }

    println!();
    Ok(())
}

// ── findings ────────────────────────────────────────────────────────────

async fn findings(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let query = ctx.paging_limit();

    if ctx.json {
        return output::json_out(&client.get_value_query("/patrol/findings", &query).await?);
    }

    let rows: Vec<Finding> = client.get_query("/patrol/findings", &query).await?;

    output::section("◈", "Patrol Findings");
    note_limit_only(ctx, "patrol findings");

    if rows.is_empty() {
        output::info("No patrol findings.");
        println!();
        return Ok(());
    }

    let open = rows
        .iter()
        .filter(|f| matches!(f.status.as_deref(), Some("open") | Some("candidate")))
        .count();
    let escalated = rows
        .iter()
        .filter(|f| f.status.as_deref() == Some("escalated"))
        .count();

    output::stat_line(
        if open > 0 { "◆" } else { "│" },
        "Open",
        &open.to_string(),
        if open > 0 { "yellow" } else { "dim" },
    );
    output::stat_line(
        if escalated > 0 { "▲" } else { "│" },
        "Escalated",
        &escalated.to_string(),
        if escalated > 0 { "red" } else { "dim" },
    );
    println!();

    output::table_header(&[
        ("FINDING ID", output::ID_W),
        ("SEV", 9),
        ("TYPE", 17),
        ("STATUS", 17),
        ("SUMMARY", 23),
    ]);
    for f in &rows {
        println!(
            "    {} {} {} {} {}",
            output::pad(&f.id, output::ID_W),
            output::pad(&severity_cell(f.severity.as_deref()), 9),
            output::pad(
                &output::truncate(f.finding_type.as_deref().unwrap_or("-"), 17),
                17
            ),
            output::pad(
                &finding_status(f.status.as_deref().unwrap_or("unknown")),
                17
            ),
            output::truncate(f.summary.as_deref().unwrap_or("-"), 23),
        );
    }

    output::count_paged("patrol finding(s)", rows.len(), None, None);
    if open > 0 {
        output::info("Triage: patrol ack-finding | normal-finding | escalate-finding <id>");
    }
    println!();
    Ok(())
}

async fn finding_show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/patrol/findings/{id}");
    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let f: Finding = client.get(&path).await?;
    output::section("◈", &format!("Patrol Finding: {}", id.bold()));
    render_finding(&f);
    println!();
    Ok(())
}

fn render_finding(f: &Finding) {
    output::field("Finding ID", &f.id);
    output::field("Severity", &severity_cell(f.severity.as_deref()));
    output::field("Type", f.finding_type.as_deref().unwrap_or("-"));
    output::field(
        "Status",
        &finding_status(f.status.as_deref().unwrap_or("unknown")),
    );
    output::field("Place", f.place.as_deref().unwrap_or("-"));
    output::field("Detected", f.at.as_deref().unwrap_or("-"));
    output::field("Robot", f.robot_id.as_deref().unwrap_or("-"));
    output::field("Run ID", f.run_id.as_deref().unwrap_or("-"));
    output::field("Route ID", f.route_id.as_deref().unwrap_or("-"));
    // `checkpoint` findings come from the per-checkpoint checklist diff;
    // `enroute_semantic` ones are seen between checkpoints and carry no
    // checkpoint id, only the leg they happened on.
    output::field("Source", f.source.as_deref().unwrap_or("-"));
    output::field(
        "Checkpoint",
        f.checkpoint_id.as_deref().unwrap_or("(en route)"),
    );
    if let Some(i) = f.leg_index {
        output::field("Leg", &i.to_string());
    }
    if let Some(c) = f.confidence {
        output::field("Confidence", &format!("{:.0}%", c * 100.0));
    }
    if let Some(m) = &f.model {
        output::field("Model", m);
    }
    if let Some(p) = &f.pose {
        output::field(
            "Pose",
            &format!(
                "x={:.2} y={:.2} yaw={:.0}°",
                p.x.unwrap_or_default(),
                p.y.unwrap_or_default(),
                p.yaw_deg.unwrap_or_default()
            ),
        );
    }
    if let Some(alert) = &f.alert_id {
        output::field("Alert", alert);
    }
    match &f.incident_id {
        Some(inc) => output::field_colored("Incident", inc.red()),
        None => output::field("Incident", "-"),
    }
    if let Some(notified) = f.robot_notified {
        output::field_colored(
            "Robot taught",
            if notified {
                "yes — folded into the robot's baseline".green()
            } else {
                "no — the robot was not reachable, so it will report this again".yellow()
            },
        );
    }

    if let Some(summary) = &f.summary {
        output::subsection("Summary");
        println!("    {} {}", "│".bright_black(), summary);
    }

    if let Some(ev) = &f.evidence {
        output::subsection("Evidence");
        render_evidence(ev);
    }
}

/// Evidence is shaped by the comparator that produced the finding: a label diff
/// (`enroute_semantic`) or a checklist diff plus the two photo keys
/// (`checkpoint`). Render what is present; `--json` carries the rest.
fn render_evidence(ev: &serde_json::Value) {
    if let Some(labels) = ev.get("labels") {
        let joined = |key: &str| -> String {
            labels
                .get(key)
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        };
        let added = joined("added");
        let missing = joined("missing");
        if !added.is_empty() {
            output::field_colored("Appeared", added.yellow());
        }
        if !missing.is_empty() {
            output::field_colored("Gone", missing.yellow());
        }
    }

    for d in ev
        .get("checklistDiff")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let cell = |key: &str| -> String {
            d.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string()
        };
        output::field(
            &cell("item"),
            &format!("{} → {}", cell("baseline"), cell("current").bold()),
        );
    }

    if let Some(n) = ev.get("observations").and_then(|v| v.as_u64()) {
        output::field("Observations", &n.to_string());
    }
    for (label, key) in [
        ("Baseline photo", "baselinePhotoKey"),
        ("Current photo", "currentPhotoKey"),
    ] {
        if let Some(k) = ev.get(key).and_then(|v| v.as_str()) {
            output::field(label, k);
        }
    }
}

// ── places ──────────────────────────────────────────────────────────────

/// `GET /patrol/places` is a proxy to one robot's place graph and answers 400
/// without `robotId`, but `patrol places` takes no robot argument — so resolve
/// the robots the patrol routes are bound to and ask each of them.
async fn places(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let routes: Vec<Route> = client.get("/patrol/routes").await?;
    let mut robot_ids: Vec<String> = Vec::new();
    for r in &routes {
        if let Some(id) = r.robot_id.as_deref()
            && !robot_ids.iter().any(|k| k == id)
        {
            robot_ids.push(id.to_string());
        }
    }

    if robot_ids.is_empty() {
        return Err(RmsError::InvalidInput(
            "no patrol route is bound to a robot, and the places route needs one — \
             bind a robot to a route, or read the graph with 'rmsctl robots show <id>'"
                .into(),
        ));
    }

    if ctx.json {
        // This subcommand fans one request out per robot, so there is no single
        // server payload to pass through. Key each robot's untouched body (or
        // its error) by robot id — a stable shape however many routes exist.
        let mut out = serde_json::Map::new();
        for id in &robot_ids {
            let entry = match client
                .get_value_query("/patrol/places", &[("robotId", id.as_str())])
                .await
            {
                Ok(v) => v,
                Err(RmsError::Api { status, message }) => serde_json::json!({
                    "error": message,
                    "status": status,
                }),
                Err(e) => return Err(e),
            };
            out.insert(id.clone(), entry);
        }
        return output::json_out(&serde_json::Value::Object(out));
    }

    output::section("◈", "Patrol Places");
    output::info(&format!(
        "Places live on the robot, not the server — asking {} robot(s) bound to patrol routes.",
        robot_ids.len()
    ));

    let mut total = 0usize;
    for id in &robot_ids {
        output::subsection(id);
        match client
            .get_query::<PlacesResponse, _>("/patrol/places", &[("robotId", id.as_str())])
            .await
        {
            Ok(resp) if resp.places.is_empty() => {
                output::info("Robot answered with no places.");
            }
            Ok(resp) => {
                total += resp.places.len();
                output::table_header(&[
                    ("PLACE ID", 20),
                    ("NAME", 20),
                    ("TYPE", 14),
                    ("CONF", 10),
                    ("SOURCE", 14),
                ]);
                for p in &resp.places {
                    println!(
                        "    {} {} {} {} {}",
                        output::pad(&output::truncate(p.id.as_deref().unwrap_or("-"), 20), 20),
                        output::pad(&output::truncate(p.name.as_deref().unwrap_or("-"), 20), 20),
                        output::pad(
                            &output::truncate(p.place_type.as_deref().unwrap_or("-"), 14),
                            14
                        ),
                        output::pad(
                            &output::truncate(p.confidence.as_deref().unwrap_or("-"), 10),
                            10
                        ),
                        output::truncate(p.source.as_deref().unwrap_or("-"), 14),
                    );
                }
            }
            // One unreachable robot must not hide the others' graphs.
            Err(RmsError::Api { status, message }) => {
                output::warn(&format!("{message} (HTTP {status})"));
            }
            Err(e) => return Err(e),
        }
    }

    output::count_paged("place(s)", total, None, None);
    println!();
    Ok(())
}

// ── start / abort ───────────────────────────────────────────────────────

/// `baseline` RECORDS the reference photos, `patrol` compares against them, so
/// a typo here either overwrites what "normal" means or silently compares
/// against nothing. The server 400s on anything else; refuse before sending.
fn validate_mode(mode: Option<&str>) -> RmsResult<&str> {
    match mode {
        None => Ok("patrol"),
        Some(m) if m == "baseline" || m == "patrol" => Ok(m),
        Some(other) => Err(RmsError::InvalidInput(format!(
            "--mode must be 'baseline' or 'patrol', not '{other}' \
             (baseline records the reference photos, patrol compares against them)"
        ))),
    }
}

/// The robot the route is bound to, unless the operator named another one.
fn resolve_robot(route: &Route, robot_id: Option<&str>) -> RmsResult<String> {
    match robot_id
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .or(route.robot_id.as_deref())
    {
        Some(id) => Ok(id.to_string()),
        None => Err(RmsError::InvalidInput(format!(
            "--robot-id is required: route '{}' is not bound to a robot",
            route.name.as_deref().unwrap_or(&route.id)
        ))),
    }
}

/// start/abort/promote are proxies to the robot agent: a robot that never
/// answers turns into HTTP 502 `ROBOT_UNREACHABLE`. The hint layer already
/// covers 502 generically, so name the robot and say what did not happen.
fn unreachable(err: RmsError, robot: &str, what: &str, tail: &str) -> RmsError {
    match err {
        RmsError::Api {
            status: 502,
            message,
        } => RmsError::Api {
            status: 502,
            message: format!(
                "robot {robot} did not answer, so {what} did not happen — {message}{tail}"
            ),
        },
        other => other,
    }
}

async fn start(
    client: &ApiClient,
    ctx: &Ctx,
    route_id: &str,
    robot_id: Option<&str>,
    mode: Option<&str>,
) -> RmsResult<()> {
    let mode = validate_mode(mode)?;

    // Read the route first: it names the robot for the gate, and a bad route
    // ID fails here as a 404 instead of after a confirmed start.
    let route: Route = client.get(&format!("/patrol/routes/{route_id}")).await?;
    let robot = resolve_robot(&route, robot_id)?;
    let name = route.name.as_deref().unwrap_or(&route.id);

    if route.enabled == Some(false) {
        output::warn("This route is disabled for cron; an operator start still runs it.");
    }

    let what = if mode == "baseline" {
        format!(
            "This will DRIVE robot {robot} through patrol '{name}' ({} checkpoints) in BASELINE mode, \
             REPLACING the reference photos every later patrol is compared against",
            route.checkpoints.len()
        )
    } else {
        format!(
            "This will DRIVE robot {robot} through patrol '{name}' ({} checkpoints)",
            route.checkpoints.len()
        )
    };
    if !output::confirm(&what, ctx.yes) {
        return Ok(());
    }

    // origin is always `operator` from a terminal — `scheduled` is the cron
    // path, and the initiative gate treats the two differently.
    let body = serde_json::json!({ "robotId": robot, "mode": mode, "origin": "operator" });
    let resp = client
        .post_value(&format!("/patrol/routes/{route_id}/start"), &body)
        .await
        .map_err(|e| {
            unreachable(
                e,
                &robot,
                "the patrol",
                ". The server recorded a skipped run and raised a warning alert — see 'rmsctl patrol runs'",
            )
        })?;

    if ctx.json {
        return output::json_out(&resp);
    }

    let result: StartResult = serde_json::from_value(resp)?;
    match result.accepted {
        Some(true) => {
            output::success(&format!(
                "Patrol '{name}' started on {robot} in {mode} mode"
            ));
            if let Some(run) = &result.run_id {
                output::field("Run ID", run);
            }
            if let Some(msg) = &result.message {
                output::info(msg);
            }
        }
        // HTTP 200 with accepted:false — the robot refused a precondition.
        _ => {
            output::warn(&format!(
                "Robot {robot} REFUSED the start — the patrol is not running"
            ));
            if let Some(reason) = &result.reason {
                output::field_colored("Reason", reason.yellow());
            }
            if let Some(msg) = &result.message {
                output::field("Robot said", msg);
            }
        }
    }
    println!();
    Ok(())
}

async fn abort(
    client: &ApiClient,
    ctx: &Ctx,
    route_id: &str,
    robot_id: Option<&str>,
    reason: Option<&str>,
) -> RmsResult<()> {
    let route: Route = client.get(&format!("/patrol/routes/{route_id}")).await?;
    let robot = resolve_robot(&route, robot_id)?;
    let name = route.name.as_deref().unwrap_or(&route.id);
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());

    if !output::confirm(
        &format!(
            "This will ABORT patrol '{name}' on robot {robot} mid-route — reason: {}",
            reason.unwrap_or("operator abort")
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let mut body = serde_json::Map::new();
    body.insert("robotId".into(), serde_json::Value::String(robot.clone()));
    if let Some(r) = reason {
        body.insert("reason".into(), serde_json::Value::String(r.to_string()));
    }
    let resp = client
        .post_value(
            &format!("/patrol/routes/{route_id}/abort"),
            &serde_json::Value::Object(body),
        )
        .await
        .map_err(|e| unreachable(e, &robot, "the abort", ""))?;

    if ctx.json {
        return output::json_out(&resp);
    }

    let result: OkResult = serde_json::from_value(resp)?;
    if result.ok == Some(true) {
        output::success(&format!("Patrol '{name}' aborted on {robot}"));
        if let Some(run) = &result.run_id {
            output::field("Run ID", run);
        }
    } else {
        // The robot answered, but not with ok:true — usually nothing was running.
        output::warn(&format!(
            "Robot {robot} did not confirm an abort — it may not have been patrolling"
        ));
    }
    println!();
    Ok(())
}

// ── run promotion ───────────────────────────────────────────────────────

async fn promote_run(client: &ApiClient, ctx: &Ctx, run_id: &str) -> RmsResult<()> {
    let r: Run = client.get(&format!("/patrol/runs/{run_id}")).await?;
    let robot = r.robot_id.clone().unwrap_or_else(|| "-".to_string());
    let name = r.route_name.as_deref().unwrap_or(&r.run_id);

    if r.promoted_at.is_some() {
        output::warn("This run is already promoted; promoting again re-sends its photos.");
    }
    if r.status.as_deref() != Some("done") {
        output::warn(&format!(
            "This run ended as '{}' — its photos may be incomplete.",
            r.status.as_deref().unwrap_or("unknown")
        ));
    }

    if !output::confirm(
        &format!(
            "This will make run {run_id} of '{name}' the BASELINE on robot {robot} \
             ({} finding(s) in it become the new normal, for the '{}' window)",
            r.finding_count.unwrap_or_default(),
            r.window.as_deref().unwrap_or("default")
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let resp = client
        .post_value(
            &format!("/patrol/runs/{run_id}/promote"),
            &serde_json::json!({}),
        )
        .await
        .map_err(|e| unreachable(e, &robot, "the promotion", ""))?;

    if ctx.json {
        return output::json_out(&resp);
    }

    let result: OkResult = serde_json::from_value(resp)?;
    if result.ok == Some(true) {
        output::success(&format!(
            "Run {run_id} is now the baseline for '{name}' on {robot}"
        ));
    } else {
        // The server only records the promotion when the robot confirms it.
        output::warn(&format!(
            "Robot {robot} did not confirm the promotion — the baseline is unchanged"
        ));
    }
    println!();
    Ok(())
}

// ── finding triage ──────────────────────────────────────────────────────

#[derive(Copy, Clone)]
enum FindingAction {
    Acknowledge,
    Normal,
    Escalate,
}

impl FindingAction {
    fn segment(self) -> &'static str {
        match self {
            FindingAction::Acknowledge => "acknowledge",
            FindingAction::Normal => "normal",
            FindingAction::Escalate => "escalate",
        }
    }

    fn gate(self, f: &Finding) -> String {
        let what = format!(
            "finding {} ({} in {})",
            f.id,
            f.finding_type.as_deref().unwrap_or("unknown"),
            f.place.as_deref().unwrap_or("unknown place")
        );
        match self {
            FindingAction::Acknowledge => {
                format!("This will ACKNOWLEDGE {what} and acknowledge its alert")
            }
            FindingAction::Normal => format!(
                "This will mark {what} NORMAL — it is dismissed AND the robot folds it into its \
                 baseline, so later patrols stop reporting it"
            ),
            FindingAction::Escalate => {
                format!("This will ESCALATE {what} into a tracked incident")
            }
        }
    }

    fn done(self) -> &'static str {
        match self {
            FindingAction::Acknowledge => "Finding acknowledged",
            FindingAction::Normal => "Finding marked normal",
            FindingAction::Escalate => "Finding escalated",
        }
    }
}

async fn finding_action(
    client: &ApiClient,
    ctx: &Ctx,
    id: &str,
    action: FindingAction,
) -> RmsResult<()> {
    // Read first so the gate names what is being triaged, not just an ID.
    let current: Finding = client.get(&format!("/patrol/findings/{id}")).await?;

    if let Some(status) = current.status.as_deref()
        && (status == "dismissed_normal" || status == "escalated")
    {
        output::warn(&format!(
            "This finding is already '{status}'; the new status replaces it."
        ));
    }

    if !output::confirm(&action.gate(&current), ctx.yes) {
        return Ok(());
    }

    let resp = client
        .post_value(
            &format!("/patrol/findings/{id}/{}", action.segment()),
            &serde_json::json!({}),
        )
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    let updated: Finding = serde_json::from_value(resp)?;
    output::success(action.done());
    output::section("◈", &format!("Patrol Finding: {}", id.bold()));
    render_finding(&updated);

    // The incident is best effort on the server: status flips to escalated
    // whether or not the incident service answered.
    if matches!(action, FindingAction::Escalate) && updated.incident_id.is_none() {
        output::warn(
            "No incident ID came back — the finding is escalated but no incident was created.",
        );
    }
    println!();
    Ok(())
}
