use crate::cli::RobotsCommand;
use crate::client::ApiClient;
use crate::error::{RmsError, RmsResult};
use crate::models::*;
use crate::output;
use colored::*;

pub async fn run(cmd: &RobotsCommand, client: &ApiClient, json: bool) -> RmsResult<()> {
    match cmd {
        RobotsCommand::List => list(client, json).await,
        RobotsCommand::Show { id } => show(client, id, json).await,
        RobotsCommand::Register { url } => register(client, url, json).await,
        RobotsCommand::Remove { id } => remove(client, id, json).await,
        RobotsCommand::Telemetry { id } => telemetry(client, id, json).await,
        RobotsCommand::Command {
            id,
            cmd_type,
            payload,
        } => command(client, id, cmd_type, payload.as_deref(), json).await,
    }
}

async fn list(client: &ApiClient, json: bool) -> RmsResult<()> {
    let resp: RobotsResponse = client.get("/robots/").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    if resp.robots.is_empty() {
        output::info("No robots registered. Use 'rmsctl robots register <url>' to add one.");
        return Ok(());
    }

    output::section("◈", "Registered Robots");

    output::table_header(&[("NAME", 18), ("STATUS", 14), ("MODEL", 14), ("ZONE", 16), ("ID", 18)]);

    for r in &resp.robots {
        let name = r.name.as_deref().unwrap_or("-");
        let status = r.status.as_deref().unwrap_or("unknown");
        let model = r.model.as_deref().unwrap_or("-");
        let zone = r
            .location
            .as_ref()
            .and_then(|l| l.get("zone"))
            .and_then(|v| v.as_str())
            .unwrap_or("-");

        println!(
            "    {} {} {} {} {}",
            output::pad(&name.bold().to_string(), 18),
            output::pad(&output::status_indicator(status), 14),
            output::pad(model, 14),
            output::pad(&zone.bright_black().to_string(), 16),
            output::truncate(&r.id, 18).bright_black()
        );
    }

    output::count_label(
        "unit(s) in fleet",
        resp.pagination.as_ref().map(|p| p.total).unwrap_or(resp.robots.len() as u32),
    );
    println!();

    Ok(())
}

async fn show(client: &ApiClient, id: &str, json: bool) -> RmsResult<()> {
    let robot: Robot = client.get(&format!("/robots/{id}")).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&robot)?);
        return Ok(());
    }

    let name = robot.name.as_deref().unwrap_or(&robot.id);
    output::section("◈", &format!("Unit: {}", name));

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
        output::field_colored("Status", output::status_indicator(v).normal());
    }
    if let Some(v) = &robot.battery_level {
        let color = if *v > 50.0 {
            "green"
        } else if *v > 20.0 {
            "yellow"
        } else {
            "red"
        };
        output::meter("Battery", *v, 100.0, color);
    }
    if let Some(loc) = &robot.location {
        let zone = loc.get("zone").and_then(|v| v.as_str()).unwrap_or("-");
        let floor = loc.get("floor").and_then(|v| v.as_str()).unwrap_or("-");
        output::field("Location", &format!("{zone}, Floor {floor}"));
    }

    println!();
    println!(
        "    {} {}",
        "│".bright_black(),
        "Connection".bold().bright_black()
    );
    if let Some(v) = &robot.ip_address {
        output::field("IP Address", v);
    }
    if let Some(v) = &robot.base_url {
        output::field("Agent URL", v);
    }
    if let Some(v) = robot.is_connected {
        let val = if v {
            "connected".green().bold()
        } else {
            "disconnected".red()
        };
        output::field_colored("Link", val);
    }

    println!();
    println!(
        "    {} {}",
        "│".bright_black(),
        "System".bold().bright_black()
    );
    if let Some(v) = &robot.firmware {
        output::field("Firmware", v);
    }
    if let Some(v) = &robot.last_seen {
        output::field("Last Seen", v);
    }

    println!();

    Ok(())
}

async fn register(client: &ApiClient, url: &str, json: bool) -> RmsResult<()> {
    let body = RegisterRequest {
        robot_url: url.to_string(),
    };
    let resp: RegisterResponse = client.post("/robots/register", &body).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    println!();
    output::success(&format!(
        "Robot registered successfully: {} ({})",
        resp.robot.name.as_deref().unwrap_or("-").bold(),
        resp.robot.id.bright_black()
    ));
    println!();

    Ok(())
}

async fn remove(client: &ApiClient, id: &str, json: bool) -> RmsResult<()> {
    let resp: DeleteResponse = client.delete(&format!("/robots/{id}")).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    println!();
    output::success(&format!("Robot {} removed from fleet", id.bold()));
    println!();

    Ok(())
}

async fn telemetry(client: &ApiClient, id: &str, json: bool) -> RmsResult<()> {
    let data: serde_json::Value = client.get(&format!("/robots/{id}/telemetry")).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }

    output::section("◈", &format!("Telemetry: {}", id.bold()));

    if let Some(obj) = data.as_object() {
        // System metrics first
        let system_keys = [
            "cpuUsage",
            "memoryUsage",
            "diskUsage",
            "temperature",
        ];
        let has_system = system_keys.iter().any(|k| {
            obj.get(*k)
                .is_some_and(|v| !v.is_null())
        });

        if has_system {
            println!(
                "    {} {}",
                "│".bright_black(),
                "System Metrics".bold().bright_black()
            );
            for key in &system_keys {
                if let Some(val) = obj.get(*key) {
                    if let Some(n) = val.as_f64() {
                        let label = match *key {
                            "cpuUsage" => "CPU",
                            "memoryUsage" => "Memory",
                            "diskUsage" => "Disk",
                            "temperature" => "Temperature",
                            _ => key,
                        };
                        if *key == "temperature" {
                            output::field(label, &format!("{n:.1} C"));
                        } else {
                            let color = if n > 80.0 {
                                "red"
                            } else if n > 60.0 {
                                "yellow"
                            } else {
                                "green"
                            };
                            output::meter(label, n, 100.0, color);
                        }
                    }
                }
            }
        }

        // Power
        let power_keys = ["batteryLevel", "batteryVoltage", "powerSource"];
        let has_power = power_keys.iter().any(|k| {
            obj.get(*k)
                .is_some_and(|v| !v.is_null())
        });
        if has_power {
            println!();
            println!(
                "    {} {}",
                "│".bright_black(),
                "Power".bold().bright_black()
            );
            for key in &power_keys {
                if let Some(val) = obj.get(*key) {
                    if val.is_null() {
                        continue;
                    }
                    let label = match *key {
                        "batteryLevel" => "Battery",
                        "batteryVoltage" => "Voltage",
                        "powerSource" => "Source",
                        _ => key,
                    };
                    let display = match val {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => format!("{n}"),
                        _ => continue,
                    };
                    output::field(label, &display);
                }
            }
        }

        // Joint states
        if let Some(joints) = obj.get("jointStates").and_then(|v| v.as_array()) {
            if !joints.is_empty() {
                println!();
                println!(
                    "    {} {}",
                    "│".bright_black(),
                    "Joint States".bold().bright_black()
                );
                println!(
                    "    {} {:<18} {:>8} {:>8} {:>8}",
                    "│".bright_black(),
                    "Joint".bright_black(),
                    "Pos".bright_black(),
                    "Vel".bright_black(),
                    "Temp".bright_black()
                );
                for j in joints {
                    let name = j.get("name").and_then(|v| v.as_str()).unwrap_or("-");
                    let pos = j
                        .get("position")
                        .and_then(|v| v.as_f64())
                        .map(|v| format!("{v:>8.2}"))
                        .unwrap_or_else(|| format!("{:>8}", "-"));
                    let vel = j
                        .get("velocity")
                        .and_then(|v| v.as_f64())
                        .map(|v| format!("{v:>8.2}"))
                        .unwrap_or_else(|| format!("{:>8}", "-"));
                    let temp = j
                        .get("temperature")
                        .and_then(|v| v.as_f64())
                        .map(|v| format!("{v:>8.1}"))
                        .unwrap_or_else(|| format!("{:>8}", "-"));
                    println!(
                        "    {} {:<18} {} {} {}",
                        "│".bright_black(),
                        name,
                        pos,
                        vel,
                        temp
                    );
                }
            }
        }

        // Sensors
        if let Some(sensors) = obj.get("sensors") {
            if sensors.is_object() && !sensors.as_object().unwrap().is_empty() {
                println!();
                println!(
                    "    {} {}",
                    "│".bright_black(),
                    "Sensors".bold().bright_black()
                );
                if let Some(sensor_map) = sensors.as_object() {
                    for (key, val) in sensor_map {
                        let display = match val {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Number(n) => n.to_string(),
                            serde_json::Value::Bool(b) => {
                                if *b { "active".to_string() } else { "inactive".to_string() }
                            }
                            serde_json::Value::Null => "-".to_string(),
                            other => serde_json::to_string(other).unwrap_or_default(),
                        };
                        output::field(key, &display);
                    }
                }
            }
        }

        // Errors / Warnings
        if let Some(errors) = obj.get("errors").and_then(|v| v.as_array()) {
            if !errors.is_empty() {
                println!();
                println!(
                    "    {} {}",
                    "▲".red(),
                    "Errors".red().bold()
                );
                for e in errors {
                    if let Some(s) = e.as_str() {
                        println!("    {} {}", "│".bright_black(), s.red());
                    }
                }
            }
        }

        if let Some(warnings) = obj.get("warnings").and_then(|v| v.as_array()) {
            if !warnings.is_empty() {
                println!();
                println!(
                    "    {} {}",
                    "◆".yellow(),
                    "Warnings".yellow().bold()
                );
                for w in warnings {
                    if let Some(s) = w.as_str() {
                        println!("    {} {}", "│".bright_black(), s.yellow());
                    }
                }
            }
        }

        // Timestamp
        if let Some(ts) = obj.get("timestamp").and_then(|v| v.as_str()) {
            println!();
            println!(
                "    {} {} {}",
                "│".bright_black(),
                "Sampled at".bright_black(),
                ts.bright_black()
            );
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&data)?);
    }

    println!();

    Ok(())
}

async fn command(
    client: &ApiClient,
    id: &str,
    cmd_type: &str,
    payload: Option<&str>,
    json: bool,
) -> RmsResult<()> {
    let payload_value = match payload {
        Some(p) => Some(serde_json::from_str(p).map_err(|e| {
            RmsError::InvalidInput(format!("Invalid JSON payload: {e}"))
        })?),
        None => None,
    };

    let body = CommandRequest {
        r#type: cmd_type.to_string(),
        payload: payload_value,
    };

    let resp: serde_json::Value = client.post(&format!("/robots/{id}/command"), &body).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    println!();
    output::success(&format!(
        "Command '{}' dispatched to {}",
        cmd_type.truecolor(255, 103, 0),
        id.bold()
    ));
    println!();

    Ok(())
}
