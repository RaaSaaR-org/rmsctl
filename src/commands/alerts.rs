use crate::cli::AlertsCommand;
use crate::client::ApiClient;
use crate::error::RmsResult;
use crate::models::*;
use crate::output;
use colored::*;

pub async fn run(cmd: &AlertsCommand, client: &ApiClient, json: bool) -> RmsResult<()> {
    match cmd {
        AlertsCommand::List { severity, active } => {
            list(client, severity.as_deref(), *active, json).await
        }
        AlertsCommand::Ack { id } => ack(client, id, json).await,
    }
}

async fn list(
    client: &ApiClient,
    severity: Option<&str>,
    active: bool,
    json: bool,
) -> RmsResult<()> {
    let resp: AlertsResponse = if active {
        match severity {
            Some(s) => {
                client
                    .get_with_query("/alerts/active", &[("severity", s)])
                    .await?
            }
            None => client.get("/alerts/active").await?,
        }
    } else {
        match severity {
            Some(s) => {
                client
                    .get_with_query("/alerts/", &[("severity", s)])
                    .await?
            }
            None => client.get("/alerts/").await?,
        }
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    if resp.alerts.is_empty() {
        output::section("◈", "Alerts");
        println!(
            "    {} {}",
            "│".bright_black(),
            "All clear — no alerts to report".bright_black()
        );
        println!();
        return Ok(());
    }

    let label = if active { "Active Alerts" } else { "Alerts" };
    output::section("◈", label);

    output::table_header(&[("SEVERITY", 16), ("TITLE", 28), ("SOURCE", 14), ("ACK", 4)]);

    for a in &resp.alerts {
        let source = a.source.as_deref().unwrap_or("-");
        let ack = if a.acknowledged == Some(true) {
            "✓".green().to_string()
        } else {
            "·".bright_black().to_string()
        };
        println!(
            "    {} {} {} {}",
            output::pad(&output::severity_indicator(&a.severity), 16),
            output::pad(&output::truncate(&a.title, 28), 28),
            output::pad(&source.bright_black().to_string(), 14),
            ack
        );
    }

    let total = resp
        .pagination
        .as_ref()
        .map(|p| p.total)
        .unwrap_or(resp.alerts.len() as u32);
    output::count_label("alert(s)", total);
    println!();

    Ok(())
}

async fn ack(client: &ApiClient, id: &str, json: bool) -> RmsResult<()> {
    let resp: Alert = client
        .patch(&format!("/alerts/{id}/acknowledge"), &serde_json::json!({}))
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    println!();
    output::success(&format!(
        "Alert acknowledged: {}",
        resp.title.bold()
    ));
    println!();

    Ok(())
}
