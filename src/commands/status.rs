use crate::client::ApiClient;
use crate::error::RmsResult;
use crate::models::*;
use crate::output;
use colored::*;

pub async fn run(client: &ApiClient, json: bool) -> RmsResult<()> {
    let (robots_resp, counts_resp) = tokio::join!(
        client.get::<RobotsResponse>("/robots/"),
        client.get::<AlertCountsResponse>("/alerts/counts")
    );

    let robots = robots_resp?;
    let counts = counts_resp?;

    if json {
        let combined = serde_json::json!({
            "robots": robots,
            "alertCounts": counts,
        });
        println!("{}", serde_json::to_string_pretty(&combined)?);
        return Ok(());
    }

    let mut online = 0u32;
    let mut offline = 0u32;
    let mut error = 0u32;
    let mut other = 0u32;

    for r in &robots.robots {
        match r.status.as_deref().unwrap_or("unknown") {
            "online" | "active" | "connected" => online += 1,
            "offline" | "disconnected" => offline += 1,
            "error" | "failed" => error += 1,
            _ => other += 1,
        }
    }

    let total = robots.robots.len() as u32;

    output::banner();

    // Fleet overview
    output::section("◈", "Fleet Overview");

    output::stat_line("│", "Total Units", &total.to_string(), "orange");
    output::stat_line(
        "│",
        "Online",
        &format!("{online}"),
        if online > 0 { "green" } else { "dim" },
    );
    output::stat_line(
        "│",
        "Error",
        &format!("{error}"),
        if error > 0 { "red" } else { "dim" },
    );
    output::stat_line(
        "│",
        "Offline",
        &format!("{offline}"),
        if offline > 0 { "red" } else { "dim" },
    );
    if other > 0 {
        output::stat_line("│", "Other", &format!("{other}"), "yellow");
    }

    if total > 0 {
        println!();
        output::meter(
            "Availability",
            online as f64,
            total as f64,
            if online == total { "green" } else { "yellow" },
        );
    }

    // Alerts
    output::section("◈", "Alert Status");

    let c = &counts.counts;
    let total_alerts = c.critical + c.error + c.warning + c.info;

    if total_alerts == 0 {
        println!(
            "    {} {}",
            "│".bright_black(),
            "All clear — no active alerts".bright_black()
        );
    } else {
        output::stat_line(
            "▲",
            "Critical",
            &c.critical.to_string(),
            if c.critical > 0 { "red" } else { "dim" },
        );
        output::stat_line(
            "▲",
            "Error",
            &c.error.to_string(),
            if c.error > 0 { "red" } else { "dim" },
        );
        output::stat_line(
            "◆",
            "Warning",
            &c.warning.to_string(),
            if c.warning > 0 { "yellow" } else { "dim" },
        );
        output::stat_line(
            "●",
            "Info",
            &c.info.to_string(),
            if c.info > 0 { "dim" } else { "dim" },
        );
    }

    // Quick robot list
    if !robots.robots.is_empty() {
        output::section("◈", "Registered Units");

        for r in &robots.robots {
            let name = r.name.as_deref().unwrap_or(&r.id);
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
                "│".bright_black(),
                output::pad(&name.bold().to_string(), 18),
                output::pad(&output::status_indicator(status), 14),
                output::pad(&model.bright_black().to_string(), 14),
                zone.bright_black()
            );
        }
    }

    println!();
    println!(
        "  {}",
        "━".repeat(50).bright_black()
    );
    println!();

    Ok(())
}
