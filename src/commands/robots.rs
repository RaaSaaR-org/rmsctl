use crate::cli::RobotsCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::models::{Robot, RobotsResponse};
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub async fn run(cmd: &RobotsCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        RobotsCommand::List => list(client, ctx).await,
        RobotsCommand::Show { id } => show(client, ctx, id).await,
        RobotsCommand::Register { url } => register(client, ctx, url).await,
        RobotsCommand::Remove { id } => remove(client, ctx, id).await,
        RobotsCommand::Telemetry { id, history } => {
            if *history {
                telemetry_history(client, ctx, id).await
            } else {
                telemetry(client, ctx, id).await
            }
        }
        RobotsCommand::AgentMode { id } => agent_mode(client, ctx, id).await,
        RobotsCommand::Cameras { id } => cameras(client, ctx, id).await,
        RobotsCommand::Peers { id } => peers(client, ctx, id).await,
        RobotsCommand::Command {
            id,
            cmd_type,
            payload,
            priority,
        } => {
            command(
                client,
                ctx,
                id,
                cmd_type,
                payload.as_deref(),
                priority.as_deref(),
            )
            .await
        }
    }
}

/// The server's closed `CommandType` union (RobotManager.ts). A type outside it
/// is accepted by the route and then rejected by the robot agent, so it is
/// cheaper to refuse it here.
const COMMAND_TYPES: [&str; 8] = [
    "move",
    "stop",
    "pickup",
    "drop",
    "charge",
    "return_home",
    "emergency_stop",
    "custom",
];

const PRIORITIES: [&str; 4] = ["low", "normal", "high", "critical"];

/// `GET /telemetry/history` defaults to 500 rows of the last hour, which is a
/// screenful per second. Ask for a readable slice unless `--limit` says more.
const HISTORY_DEFAULT_LIMIT: u32 = 20;

// ── Response shapes ─────────────────────────────────────────────────────

/// `POST /robots/register` answers `{robot, endpoints, agentCard}` — note the
/// camelCase `agentCard`, which is why this struct needs `rename_all`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisterResponse {
    robot: Robot,
    #[serde(default)]
    endpoints: Option<RobotEndpoints>,
    #[serde(default)]
    agent_card: Option<AgentCard>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RobotEndpoints {
    #[serde(default)]
    robot: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    telemetry: Option<String>,
    #[serde(default)]
    telemetry_ws: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentCard {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    skills: Option<Vec<AgentSkill>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentSkill {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// `GET /robots/{id}/telemetry/history` -> `{telemetry: [...]}`.
///
/// Rows are read as `Value` on purpose: they carry the same free-form frame the
/// live telemetry route sends (sensors, joints, imu, odometry) and a typed
/// mirror of that would be a second place to keep the server contract.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TelemetryHistory {
    #[serde(default)]
    telemetry: Vec<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentModeState {
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    control_owner: Option<String>,
    #[serde(default)]
    estop_active: Option<bool>,
    #[serde(default)]
    estop_source: Option<String>,
    #[serde(default)]
    estop_reason: Option<String>,
    #[serde(default)]
    fsm_id: Option<Value>,
    #[serde(default)]
    damped: Option<bool>,
    #[serde(default)]
    mirrored_at: Option<String>,
    #[serde(default)]
    state_mirrored_at: Option<String>,
    #[serde(default)]
    server_now: Option<String>,
}

/// `GET /robots/{id}/cameras` always answers 200 — an unreachable agent comes
/// back as an empty list plus a `detail` saying why.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CamerasResponse {
    #[serde(default)]
    cameras: Vec<Value>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    detail: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PeersResponse {
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    peers: Vec<Peer>,
    #[serde(default)]
    generated_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Peer {
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
    #[serde(default)]
    heading_deg: Option<f64>,
    #[serde(default)]
    zone: Option<String>,
    #[serde(default)]
    place: Option<String>,
    #[serde(default)]
    pose_age_ms: Option<f64>,
    #[serde(default)]
    footprint_radius_m: Option<f64>,
    #[serde(default)]
    updated_at: Option<String>,
}

/// `POST /robots/{id}/command` returns the agent's `RobotCommand`, not an ack:
/// a 200 can carry `status: "failed"`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CommandResult {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default, rename = "type")]
    command_type: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
}

// ── list ────────────────────────────────────────────────────────────────

async fn list(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let query = ctx.paging();

    if ctx.json {
        return output::json_out(&client.get_value_query("/robots/", &query).await?);
    }

    let resp: RobotsResponse = client.get_query("/robots/", &query).await?;

    output::section("◈", "Registered Robots");

    if resp.robots.is_empty() {
        output::info("No robots registered. Use 'rmsctl robots register <url>' to add one.");
        println!();
        return Ok(());
    }

    // The route builds its pagination envelope from the full result set
    // (page 1, pageSize = robots.length) and ignores the query entirely.
    if ctx.has_paging() {
        output::info("GET /robots returns the whole fleet — --page/--limit are ignored here.");
    }

    output::table_header(&[
        ("NAME", 16),
        ("STATUS", 17),
        ("BATT", 5),
        ("ZONE", 13),
        ("TASK", 14),
        ("ID", output::ID_W),
    ]);

    for r in &resp.robots {
        println!(
            "    {} {} {} {} {} {}",
            output::pad(
                &output::truncate(r.name.as_deref().unwrap_or("-"), 16)
                    .bold()
                    .to_string(),
                16
            ),
            output::pad(
                &output::status_indicator(r.status.as_deref().unwrap_or("unknown")),
                17
            ),
            output::pad(&battery_cell(r.battery_level), 5),
            output::pad(
                &output::truncate(r.zone().unwrap_or("-"), 13)
                    .bright_black()
                    .to_string(),
                13
            ),
            output::pad(
                &output::truncate(r.current_task_name.as_deref().unwrap_or("-"), 14),
                14
            ),
            r.id.bright_black()
        );
    }

    let pag = resp.pagination.as_ref();
    output::count_paged(
        "unit(s) in fleet",
        resp.robots.len(),
        pag.and_then(|p| p.total()),
        pag.and_then(|p| p.total_pages()),
    );
    println!();
    Ok(())
}

// ── show ────────────────────────────────────────────────────────────────

async fn show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/robots/{id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    // This route answers a BARE robot object, not a `{robot: ...}` wrapper.
    let robot: Robot = client.get(&path).await?;

    let name = robot.name.as_deref().unwrap_or(&robot.id);
    output::section("◈", &format!("Unit: {}", name.bold()));

    output::field("ID", &robot.id);
    if let Some(v) = &robot.name {
        output::field("Name", v);
    }
    if let Some(v) = &robot.model {
        output::field("Model", v);
    }
    if let Some(v) = &robot.serial_number {
        output::field("Serial", v);
    }
    if let Some(v) = &robot.status {
        output::field("Status", &output::status_indicator(v));
    }
    // The most operationally interesting field on a stopped robot: a
    // protective stop reports itself here and nowhere else on the entity.
    if let Some(task) = &robot.current_task_name {
        output::field_colored("Task", task.truecolor(255, 103, 0));
    }
    if let Some(tid) = &robot.current_task_id {
        output::field("Task ID", tid);
    }
    if let Some(v) = robot.battery_level {
        output::meter("Battery", v, 100.0, battery_color(v));
    }
    if let Some(caps) = capability_list(robot.capabilities.as_ref()) {
        output::field("Capabilities", &caps);
    }

    if let Some(loc) = &robot.location {
        output::subsection("Location");
        output::field(
            "Zone",
            &format!(
                "{} / floor {}",
                robot.zone().unwrap_or("-"),
                robot.floor().unwrap_or("-")
            ),
        );
        let x = loc.get("x").and_then(|v| v.as_f64());
        let y = loc.get("y").and_then(|v| v.as_f64());
        if let (Some(x), Some(y)) = (x, y) {
            output::field("Position", &format!("x {x:.2}  y {y:.2}"));
        }
        if let Some(h) = loc.get("heading").and_then(|v| v.as_f64()) {
            output::field("Heading", &format!("{h:.1}°"));
        }
        if let Some(p) = loc.get("place").and_then(|v| v.as_str()) {
            output::field("Place", p);
        }
    }

    output::subsection("Connection");
    if let Some(v) = &robot.ip_address {
        output::field("IP Address", v);
    }
    // `a2aAgentUrl` on the list/detail routes, `baseUrl` on register — the
    // model aliases both. There is deliberately no "Link" row: `isConnected`
    // exists only on the register response, so a connectivity claim here would
    // be invented. `status` is the server's own answer to that question.
    if let Some(v) = &robot.a2a_agent_url {
        output::field("Agent URL", v);
    }
    if let Some(v) = robot.a2a_enabled {
        output::field_colored(
            "A2A",
            if v {
                "enabled".green()
            } else {
                "disabled".bright_black()
            },
        );
    }

    if let Some(meta) = robot.metadata.as_ref().and_then(|m| m.as_object()) {
        output::subsection("Hardware");
        if let Some(t) = text(meta, "robotType") {
            output::field("Type", t);
        }
        if let Some(c) = text(meta, "robotClass") {
            output::field("Class", c);
        }
        if let Some(true) = flag(meta, "isSimulated") {
            output::field_colored("Simulated", "yes — values are generated".yellow());
        }
        if let Some(hw) = flag(meta, "hardwareConnected") {
            output::field_colored(
                "Hardware link",
                if hw {
                    "connected".green()
                } else {
                    "not connected".bright_black()
                },
            );
        }
        if let Some(kg) = num(meta, "maxPayloadKg") {
            output::field("Max payload", &format!("{kg} kg"));
        }
        if let Some(r) = num(meta, "footprintRadiusM") {
            output::field("Footprint", &format!("{r} m radius"));
        }
        if let Some(p) = text(meta, "powerSource") {
            output::field("Power source", p);
        }
        if let Some(d) = text(meta, "description") {
            output::field("Description", &output::truncate(d, 60));
        }
    }

    output::subsection("System");
    if let Some(v) = &robot.firmware {
        output::field("Firmware", v);
    }
    if let Some(v) = &robot.last_seen {
        output::field("Last Seen", v);
    }
    if let Some(v) = &robot.created_at {
        output::field("Registered", v);
    }
    if let Some(v) = &robot.updated_at {
        output::field("Updated", v);
    }

    println!();
    Ok(())
}

// ── register / remove ───────────────────────────────────────────────────

async fn register(client: &ApiClient, ctx: &Ctx, url: &str) -> RmsResult<()> {
    let url = url.trim();
    if url.is_empty() {
        return Err(RmsError::InvalidInput(
            "give the robot agent URL, e.g. rmsctl robots register http://192.168.1.45:41243"
                .into(),
        ));
    }
    // The route runs `new URL(robotUrl)` and answers 400 on anything else.
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(RmsError::InvalidInput(format!(
            "'{url}' is not an absolute URL — the server needs a scheme, e.g. http://{url}"
        )));
    }

    if !output::confirm(
        &format!("This will REGISTER the robot agent at {url} into the fleet"),
        ctx.yes,
    ) {
        return Ok(());
    }

    let body = serde_json::json!({ "robotUrl": url });
    let resp = client.post_value("/robots/register", &body).await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    let parsed: RegisterResponse = serde_json::from_value(resp)?;
    output::success(&format!(
        "Registered {} ({})",
        parsed.robot.name.as_deref().unwrap_or("-").bold(),
        parsed.robot.id.bright_black()
    ));

    output::section("◈", "Registration");
    output::field("ID", &parsed.robot.id);
    if let Some(v) = &parsed.robot.status {
        output::field("Status", &output::status_indicator(v));
    }
    if let Some(v) = &parsed.robot.a2a_agent_url {
        output::field("Agent URL", v);
    }

    if let Some(e) = &parsed.endpoints {
        output::subsection("Endpoints");
        if let Some(v) = &e.robot {
            output::field("Robot", v);
        }
        if let Some(v) = &e.command {
            output::field("Command", v);
        }
        if let Some(v) = &e.telemetry {
            output::field("Telemetry", v);
        }
        if let Some(v) = &e.telemetry_ws {
            output::field("Telemetry WS", v);
        }
    }

    if let Some(card) = &parsed.agent_card {
        output::subsection("A2A Agent Card");
        if let Some(v) = &card.name {
            output::field("Name", v);
        }
        if let Some(v) = &card.version {
            output::field("Version", v);
        }
        if let Some(v) = &card.url {
            output::field("URL", v);
        }
        if let Some(v) = &card.description {
            output::field("Description", &output::truncate(v, 60));
        }
        for s in card.skills.iter().flatten() {
            let label = s.name.as_deref().or(s.id.as_deref()).unwrap_or("-");
            match &s.description {
                Some(d) => {
                    output::field("Skill", &format!("{label} — {}", output::truncate(d, 44)))
                }
                None => output::field("Skill", label),
            }
        }
    }

    println!();
    Ok(())
}

async fn remove(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    if !output::confirm(
        &format!("This will UNREGISTER robot {id} — the server drops it from the fleet"),
        ctx.yes,
    ) {
        return Ok(());
    }

    let resp = client.delete_value(&format!("/robots/{id}")).await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    output::success(&format!("Robot {} removed from fleet", id.bold()));
    if resp.get("success").and_then(|v| v.as_bool()) == Some(false) {
        output::warn("The server answered success:false — re-check with 'rmsctl robots list'.");
    }
    println!();
    Ok(())
}

// ── telemetry ───────────────────────────────────────────────────────────

async fn telemetry(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/robots/{id}/telemetry");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let data = client.get_value(&path).await?;

    output::section("◈", &format!("Telemetry: {}", id.bold()));

    let Some(obj) = data.as_object() else {
        println!("{}", serde_json::to_string_pretty(&data)?);
        println!();
        return Ok(());
    };

    let sim = sim_groups(obj);
    let hardware = flag(obj, "hardwareConnected");

    // The server marks every field group it fabricated for this frame. Printing
    // the numbers without that marker presents synthetic joints, poses and
    // battery readings as hardware measurements.
    if !sim.is_empty() {
        output::warn(&format!("SIMULATED values this frame: {}", sim.join(", ")));
    }
    if hardware == Some(false) {
        output::info("hardwareConnected: false — no hardware is backing these readings.");
    }

    if let Some(t) = text(obj, "robotType") {
        output::field("Type", t);
    }

    render_system(obj);
    render_power(obj, &sim);
    render_environment(obj);
    render_joints(obj, &sim);
    render_imu(obj, &sim);
    render_odometry(obj, &sim);
    render_touch(obj, &sim);
    render_sensors(obj, &sim);
    render_faults(obj);

    println!();
    if let Some(ts) = text(obj, "timestamp") {
        println!(
            "    {} {} {}",
            "│".bright_black(),
            "Sampled at".bright_black(),
            ts.bright_black()
        );
    }
    println!(
        "    {} {} {}",
        "│".bright_black(),
        "Hardware".bright_black(),
        match hardware {
            Some(true) => "connected".green(),
            Some(false) => "NOT connected — simulated frame".yellow(),
            None => "unreported".bright_black(),
        }
    );

    println!();
    Ok(())
}

async fn telemetry_history(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/robots/{id}/telemetry/history");
    let mut query = ctx.paging_limit();
    if ctx.limit.is_none() {
        query.push(("limit", HISTORY_DEFAULT_LIMIT.to_string()));
    }

    if ctx.json {
        return output::json_out(&client.get_value_query(&path, &query).await?);
    }

    let resp: TelemetryHistory = client.get_query(&path, &query).await?;

    output::section("◈", &format!("Telemetry History: {}", id.bold()));

    if resp.telemetry.is_empty() {
        output::info("No persisted telemetry in the window (the default window is the last hour).");
        println!();
        return Ok(());
    }

    if ctx.limit.is_none() {
        output::info(&format!(
            "Newest {HISTORY_DEFAULT_LIMIT} frames of the last hour — use --limit N (max 2000) for more."
        ));
    }

    output::table_header(&[
        ("WHEN", 26),
        ("BATT", 6),
        ("CPU", 6),
        ("MEM", 6),
        ("TEMP", 7),
        ("SPEED", 7),
        ("HW", 3),
        ("SIMULATED", 34),
    ]);

    for frame in &resp.telemetry {
        let Some(f) = frame.as_object() else { continue };
        println!(
            "    {} {} {} {} {} {} {} {}",
            output::pad(text(f, "timestamp").unwrap_or("-"), 26),
            output::pad(&pct_cell(num(f, "batteryLevel")), 6),
            output::pad(&pct_cell(num(f, "cpuUsage")), 6),
            output::pad(&pct_cell(num(f, "memoryUsage")), 6),
            output::pad(&opt_num(num(f, "temperature"), 1, "°"), 7),
            output::pad(&opt_num(num(f, "speed"), 2, ""), 7),
            output::pad(&output::bool_indicator(flag(f, "hardwareConnected")), 3),
            output::truncate(
                &match sim_groups(f) {
                    g if g.is_empty() => "-".to_string(),
                    g => g.join(","),
                },
                34
            )
            .yellow()
        );
    }

    // Rows come back ascending by timestamp; the envelope carries no total.
    output::count_paged(
        "telemetry frame(s), oldest first",
        resp.telemetry.len(),
        None,
        None,
    );
    println!();
    Ok(())
}

fn render_system(obj: &Map<String, Value>) {
    let keys: [(&str, &str); 3] = [
        ("cpuUsage", "CPU"),
        ("memoryUsage", "Memory"),
        ("diskUsage", "Disk"),
    ];
    if !keys.iter().any(|(k, _)| num(obj, k).is_some()) && num(obj, "temperature").is_none() {
        return;
    }

    block("System Metrics", false);
    for (key, label) in keys {
        if let Some(n) = num(obj, key) {
            output::meter(label, n, 100.0, load_color(n));
        }
    }
    if let Some(n) = num(obj, "temperature") {
        output::field("Temperature", &format!("{n:.1} °C"));
    }
}

fn render_power(obj: &Map<String, Value>, sim: &[&str]) {
    let battery = obj.get("battery").and_then(|v| v.as_object());
    if num(obj, "batteryLevel").is_none()
        && num(obj, "batteryVoltage").is_none()
        && num(obj, "batteryTemperature").is_none()
        && text(obj, "powerSource").is_none()
        && battery.is_none()
    {
        return;
    }

    block("Power", sim.contains(&"battery"));
    if let Some(n) = num(obj, "batteryLevel") {
        output::meter("Battery", n, 100.0, battery_color(n));
    }
    if let Some(n) = num(obj, "batteryVoltage") {
        output::field("Voltage", &format!("{n:.2} V"));
    }
    if let Some(n) = num(obj, "batteryTemperature") {
        output::field("Batt temp", &format!("{n:.1} °C"));
    }
    if let Some(s) = text(obj, "powerSource") {
        output::field("Source", s);
    }
    // The richer BMS block only appears on hardware frames.
    if let Some(b) = battery {
        if let Some(n) = num(b, "soc") {
            output::field("BMS charge", &format!("{n:.1} %"));
        }
        if let Some(n) = num(b, "current") {
            output::field("Current", &format!("{n:.2} A"));
        }
        if let Some(n) = num(b, "soh") {
            output::field("Health", &format!("{n:.1} %"));
        }
        if let Some(n) = num(b, "cycles") {
            output::field("Cycles", &format!("{n:.0}"));
        }
    }
}

fn render_environment(obj: &Map<String, Value>) {
    if num(obj, "humidity").is_none() && num(obj, "speed").is_none() {
        return;
    }
    block("Environment & Motion", false);
    if let Some(n) = num(obj, "humidity") {
        output::field("Humidity", &format!("{n:.1} %"));
    }
    if let Some(n) = num(obj, "speed") {
        output::field("Speed", &format!("{n:.2} m/s"));
    }
}

fn render_joints(obj: &Map<String, Value>, sim: &[&str]) {
    let Some(joints) = obj.get("jointStates").and_then(|v| v.as_array()) else {
        return;
    };
    if joints.is_empty() {
        return;
    }

    let motor_temps = obj.get("motorTemperatures").and_then(|v| v.as_object());
    block(
        "Joint States",
        sim.contains(&"joints") || sim.contains(&"motorTemperatures"),
    );
    println!(
        "    {} {:<26} {:>9} {:>9} {:>9} {:>8}",
        "│".bright_black(),
        "Joint".bright_black(),
        "Pos".bright_black(),
        "Vel".bright_black(),
        "Effort".bright_black(),
        "Temp".bright_black()
    );
    for j in joints {
        let name = j.get("name").and_then(|v| v.as_str()).unwrap_or("-");
        // On real hardware the per-joint temperature arrives in the frame-level
        // `motorTemperatures` map, not on the joint itself.
        let temp = j
            .get("temperature")
            .and_then(|v| v.as_f64())
            .or_else(|| motor_temps?.get(name)?.as_f64());
        println!(
            "    {} {:<26} {:>9} {:>9} {:>9} {:>8}",
            "│".bright_black(),
            output::truncate(name, 26),
            fixed(j.get("position").and_then(|v| v.as_f64()), 3),
            fixed(j.get("velocity").and_then(|v| v.as_f64()), 3),
            fixed(j.get("effort").and_then(|v| v.as_f64()), 2),
            fixed(temp, 1)
        );
    }
}

fn render_imu(obj: &Map<String, Value>, sim: &[&str]) {
    let Some(imu) = obj.get("imu").and_then(|v| v.as_object()) else {
        return;
    };
    block("IMU", sim.contains(&"imu"));
    if let Some(v) = imu.get("rpy").and_then(vec3) {
        output::field("Roll/Pitch/Yaw", &v);
    }
    if let Some(v) = imu.get("gyro").and_then(vec3) {
        output::field("Gyro", &v);
    }
    if let Some(v) = imu.get("accel").and_then(vec3) {
        output::field("Accel", &v);
    }
    if let Some(n) = num(imu, "temperature") {
        output::field("Temperature", &format!("{n:.1} °C"));
    }
}

fn render_odometry(obj: &Map<String, Value>, sim: &[&str]) {
    let Some(odo) = obj.get("odometry").and_then(|v| v.as_object()) else {
        return;
    };
    block(
        "Odometry",
        sim.contains(&"odometry") || sim.contains(&"position"),
    );
    if let Some(v) = odo.get("position").and_then(vec3) {
        output::field("Position", &v);
    }
    if let Some(v) = odo.get("rpy").and_then(vec3) {
        output::field("Orientation", &v);
    }
    if let Some(v) = odo.get("velocity").and_then(vec3) {
        output::field("Velocity", &v);
    }
    if let Some(n) = num(odo, "yawSpeed") {
        output::field("Yaw speed", &format!("{n:.3}"));
    }
}

fn render_touch(obj: &Map<String, Value>, sim: &[&str]) {
    let Some(touch) = obj.get("touch").and_then(|v| v.as_object()) else {
        return;
    };
    block("Hand Touch", sim.contains(&"touch"));
    for hand in ["left", "right"] {
        let Some(pads) = touch.get(hand).and_then(|v| v.as_array()) else {
            continue;
        };
        let peak = pads
            .iter()
            .filter_map(|p| p.get("pressure")?.as_array())
            .flatten()
            .filter_map(|v| v.as_f64())
            .fold(f64::NEG_INFINITY, f64::max);
        let label = format!("{} hand", hand[..1].to_uppercase());
        if peak.is_finite() {
            output::field(
                &label,
                &format!("{} pad(s), peak pressure {peak:.1}", pads.len()),
            );
        } else {
            output::field(&label, &format!("{} pad(s)", pads.len()));
        }
    }
}

fn render_sensors(obj: &Map<String, Value>, sim: &[&str]) {
    let Some(sensors) = obj.get("sensors").and_then(|v| v.as_object()) else {
        return;
    };
    if sensors.is_empty() {
        return;
    }
    block("Sensors", sim.contains(&"sensors"));
    for (key, val) in sensors {
        let display = match val {
            Value::String(s) => s.clone(),
            // Sonar and current readings arrive with full float precision
            // (171.24457781101916); two places is all a terminal can use.
            Value::Number(n) => match n.as_f64() {
                Some(f) if f.fract() != 0.0 => format!("{f:.2}"),
                Some(f) => format!("{f:.0}"),
                None => n.to_string(),
            },
            Value::Bool(b) => {
                if *b {
                    "active".to_string()
                } else {
                    "inactive".to_string()
                }
            }
            Value::Null => "-".to_string(),
            other => serde_json::to_string(other).unwrap_or_default(),
        };
        output::field(key, &display);
    }
}

fn render_faults(obj: &Map<String, Value>) {
    if let Some(errors) = obj.get("errors").and_then(|v| v.as_array())
        && !errors.is_empty()
    {
        println!();
        println!("    {} {}", "▲".red(), "Errors".red().bold());
        for e in errors {
            if let Some(s) = e.as_str() {
                println!("    {} {}", "│".bright_black(), s.red());
            }
        }
    }
    if let Some(warnings) = obj.get("warnings").and_then(|v| v.as_array())
        && !warnings.is_empty()
    {
        println!();
        println!("    {} {}", "◆".yellow(), "Warnings".yellow().bold());
        for w in warnings {
            if let Some(s) = w.as_str() {
                println!("    {} {}", "│".bright_black(), s.yellow());
            }
        }
    }
}

// ── agent-mode ──────────────────────────────────────────────────────────

async fn agent_mode(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/robots/{id}/agent-mode");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    // A 502 here means AGENT_STATE_UNAVAILABLE: the mode and the E-stop latch
    // are UNKNOWN, which is deliberately not the same answer as "mode off".
    // Letting the error propagate keeps that distinction (and the exit code).
    let state: AgentModeState = client.get(&path).await?;

    output::section("◈", &format!("Agent Mode: {}", id.bold()));

    output::field_colored(
        "Agent mode",
        match state.enabled {
            Some(true) => "ENABLED".green().bold(),
            Some(false) => "disabled".bright_black(),
            None => "unreported".bright_black(),
        },
    );
    if let Some(owner) = &state.control_owner {
        output::field("Control owner", &output::status_indicator(owner));
    }

    match state.estop_active {
        Some(true) => {
            output::field_colored("E-stop", "■ LATCHED".red().bold());
            if let Some(s) = &state.estop_source {
                output::field("Source", s);
            }
            if let Some(r) = &state.estop_reason {
                output::field("Reason", r);
            }
        }
        Some(false) => output::field_colored("E-stop", "clear".green()),
        None => output::field("E-stop", "unreported"),
    }

    if let Some(true) = state.damped {
        output::field_colored("Motors", "damped".yellow());
    }
    if let Some(fsm) = state.fsm_id.as_ref().filter(|v| !v.is_null()) {
        output::field("FSM", &value_text(fsm));
    }

    output::subsection("Mirror");
    // The mirror is in-memory and event-driven, so the age of THIS body is
    // stateMirroredAt — mirroredAt moves on any event, snapshot or not.
    output::field(
        "State mirrored",
        state.state_mirrored_at.as_deref().unwrap_or("never"),
    );
    if let Some(v) = &state.mirrored_at {
        output::field("Last event", v);
    }
    if let Some(v) = &state.server_now {
        output::field("Server now", v);
    }
    if let Some(v) = &state.robot_id {
        output::field("Robot ID", v);
    }

    println!();
    Ok(())
}

// ── cameras ─────────────────────────────────────────────────────────────

async fn cameras(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/robots/{id}/cameras");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp: CamerasResponse = client.get(&path).await?;

    output::section("◈", &format!("Cameras: {}", id.bold()));

    if let Some(src) = &resp.source {
        output::field("Source", src);
    }

    if resp.cameras.is_empty() {
        // Not an error path: the route answers 200 with an empty list and puts
        // the reason (unreachable agent, non-JSON answer) in `detail`.
        output::warn(
            resp.detail
                .as_deref()
                .unwrap_or("The agent served no cameras."),
        );
        println!();
        return Ok(());
    }

    if let Some(detail) = &resp.detail {
        output::info(detail);
    }

    output::table_header(&[("CAMERA", 24)]);
    for cam in &resp.cameras {
        let name = cam
            .as_str()
            .map(str::to_string)
            .or_else(|| cam.get("name").and_then(|v| v.as_str()).map(str::to_string))
            .unwrap_or_else(|| value_text(cam));
        println!("    {}", output::truncate(&name, 24));
    }

    output::count_paged("camera(s)", resp.cameras.len(), None, None);
    println!();
    Ok(())
}

// ── peers ───────────────────────────────────────────────────────────────

async fn peers(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/robots/{id}/peers");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp: PeersResponse = client.get(&path).await?;

    output::section("◈", &format!("A2A Peers: {}", id.bold()));

    if resp.peers.is_empty() {
        // Only CONNECTED robots other than this one are listed.
        output::info("No connected peers besides this robot.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("NAME", 16),
        ("ZONE", 14),
        ("X", 9),
        ("Y", 9),
        ("HDG", 8),
        ("AGE", 8),
        ("ID", output::ID_W),
    ]);

    for p in &resp.peers {
        println!(
            "    {} {} {} {} {} {} {}",
            output::pad(
                &output::truncate(p.name.as_deref().unwrap_or("-"), 16)
                    .bold()
                    .to_string(),
                16
            ),
            output::pad(
                &output::truncate(p.zone.as_deref().or(p.place.as_deref()).unwrap_or("-"), 14)
                    .bright_black()
                    .to_string(),
                14
            ),
            output::pad(&fixed(p.x, 2), 9),
            output::pad(&fixed(p.y, 2), 9),
            output::pad(&fixed(p.heading_deg, 1), 8),
            output::pad(&pose_age(p.pose_age_ms), 8),
            p.robot_id.as_deref().unwrap_or("-").bright_black()
        );
    }

    output::count_paged("peer(s)", resp.peers.len(), None, None);
    if let Some(ts) = &resp.generated_at {
        println!("  {} {}", "[--]".bright_black(), ts.bright_black());
    }
    println!();
    Ok(())
}

// ── command ─────────────────────────────────────────────────────────────

async fn command(
    client: &ApiClient,
    ctx: &Ctx,
    id: &str,
    cmd_type: &str,
    payload: Option<&str>,
    priority: Option<&str>,
) -> RmsResult<()> {
    if !COMMAND_TYPES.contains(&cmd_type) {
        return Err(RmsError::InvalidInput(format!(
            "unknown command type '{cmd_type}' — the server accepts: {}",
            COMMAND_TYPES.join(", ")
        )));
    }
    if let Some(p) = priority
        && !PRIORITIES.contains(&p)
    {
        return Err(RmsError::InvalidInput(format!(
            "unknown --priority '{p}' — use one of: {}",
            PRIORITIES.join(", ")
        )));
    }

    let payload_value = match payload {
        Some(p) => {
            let v: Value = serde_json::from_str(p)
                .map_err(|e| RmsError::InvalidInput(format!("--payload is not valid JSON: {e}")))?;
            if !v.is_object() {
                return Err(RmsError::InvalidInput(
                    "--payload must be a JSON object, e.g. --payload '{\"x\":1,\"y\":2}'".into(),
                ));
            }
            Some(v)
        }
        None => None,
    };

    let described = match (&payload_value, priority) {
        (Some(p), Some(pr)) => format!(" with priority {pr} and payload {p}"),
        (Some(p), None) => format!(" with payload {p}"),
        (None, Some(pr)) => format!(" with priority {pr}"),
        (None, None) => String::new(),
    };
    if !output::confirm(
        &format!("This will SEND the '{cmd_type}' command to robot {id}{described}"),
        ctx.yes,
    ) {
        return Ok(());
    }

    let mut body = Map::new();
    body.insert("type".into(), Value::String(cmd_type.to_string()));
    if let Some(p) = payload_value {
        body.insert("payload".into(), p);
    }
    if let Some(p) = priority {
        body.insert("priority".into(), Value::String(p.to_string()));
    }

    let resp = client
        .post_value(&format!("/robots/{id}/command"), &Value::Object(body))
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    // A 200 is only an accepted dispatch, not an executed command: the body is
    // the agent's RobotCommand and can already say failed/cancelled.
    let result: CommandResult = serde_json::from_value(resp)?;
    let status = result.status.as_deref().unwrap_or("unknown");

    output::section("◈", &format!("Command: {}", cmd_type.bold()));
    if let Some(cid) = &result.id {
        output::field("Command ID", cid);
    }
    output::field("Robot", result.robot_id.as_deref().unwrap_or(id));
    output::field("Type", result.command_type.as_deref().unwrap_or(cmd_type));
    output::field("Status", &output::status_indicator(status));
    if let Some(p) = &result.priority {
        output::field("Priority", p);
    }
    if let Some(v) = &result.created_at {
        output::field("Created", v);
    }
    if let Some(v) = &result.completed_at {
        output::field("Completed", v);
    }
    if let Some(r) = result.result.as_ref().filter(|v| !v.is_null()) {
        output::field("Result", &output::truncate(&value_text(r), 60));
    }
    println!();

    match status {
        "failed" | "cancelled" | "canceled" => {
            let msg = result
                .error_message
                .as_deref()
                .unwrap_or("the robot reported no error message");
            Err(RmsError::Other(format!(
                "Command '{cmd_type}' {status} on {id}: {msg}"
            )))
        }
        "pending" | "executing" | "completed" => {
            output::success(&format!(
                "Command '{}' {status} on {}",
                cmd_type.truecolor(255, 103, 0),
                id.bold()
            ));
            if let Some(msg) = &result.error_message {
                output::warn(msg);
            }
            println!();
            Ok(())
        }
        other => {
            output::warn(&format!(
                "The server reported an unrecognised command status '{other}'."
            ));
            println!();
            Ok(())
        }
    }
}

// ── shared rendering helpers ────────────────────────────────────────────

/// Telemetry block header, tagged when the server flagged the group as
/// fabricated. Mirrors `output::subsection`, which cannot carry the tag.
fn block(label: &str, simulated: bool) {
    println!();
    if simulated {
        println!(
            "    {} {} {}",
            "│".bright_black(),
            label.bold().bright_black(),
            "(simulated)".yellow()
        );
    } else {
        println!("    {} {}", "│".bright_black(), label.bold().bright_black());
    }
}

/// Field groups whose values this frame fabricated.
fn sim_groups(obj: &Map<String, Value>) -> Vec<&str> {
    obj.get("simulated")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default()
}

fn num(obj: &Map<String, Value>, key: &str) -> Option<f64> {
    obj.get(key)?.as_f64()
}

fn text<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key)?.as_str()
}

fn flag(obj: &Map<String, Value>, key: &str) -> Option<bool> {
    obj.get(key)?.as_bool()
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn vec3(v: &Value) -> Option<String> {
    let a = v.as_array()?;
    let parts: Vec<String> = a
        .iter()
        .map(|n| match n.as_f64() {
            Some(f) => format!("{f:.3}"),
            None => "-".to_string(),
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn fixed(v: Option<f64>, places: usize) -> String {
    match v {
        Some(n) => format!("{n:.places$}"),
        None => "-".to_string(),
    }
}

fn opt_num(v: Option<f64>, places: usize, suffix: &str) -> String {
    match v {
        Some(n) => format!("{n:.places$}{suffix}"),
        None => "-".to_string(),
    }
}

fn pct_cell(v: Option<f64>) -> String {
    match v {
        Some(n) => format!("{n:.0}%"),
        None => "-".to_string(),
    }
}

fn pose_age(ms: Option<f64>) -> String {
    match ms {
        Some(v) if v < 1000.0 => format!("{v:.0}ms"),
        Some(v) => format!("{:.1}s", v / 1000.0),
        None => "-".to_string(),
    }
}

fn battery_cell(level: Option<f64>) -> String {
    match level {
        Some(v) => {
            let cell = format!("{v:.0}%");
            match battery_color(v) {
                "green" => cell.green().to_string(),
                "yellow" => cell.yellow().to_string(),
                _ => cell.red().to_string(),
            }
        }
        None => "-".bright_black().to_string(),
    }
}

fn battery_color(level: f64) -> &'static str {
    if level > 50.0 {
        "green"
    } else if level > 20.0 {
        "yellow"
    } else {
        "red"
    }
}

fn load_color(pct: f64) -> &'static str {
    if pct > 80.0 {
        "red"
    } else if pct > 60.0 {
        "yellow"
    } else {
        "green"
    }
}

/// `capabilities` is a bare JSON array of strings on every robot the server
/// serves; accept a single string too rather than dropping the row.
fn capability_list(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Array(a) => {
            let items: Vec<&str> = a.iter().filter_map(|c| c.as_str()).collect();
            (!items.is_empty()).then(|| items.join(", "))
        }
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}
