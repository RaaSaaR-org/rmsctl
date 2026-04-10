use crate::cli::DeploymentsCommand;
use crate::client::ApiClient;
use crate::error::RmsResult;
use crate::models::*;
use crate::output;
use colored::*;

pub async fn run(cmd: &DeploymentsCommand, client: &ApiClient, json: bool) -> RmsResult<()> {
    match cmd {
        DeploymentsCommand::List => list(client, json).await,
        DeploymentsCommand::Show { id } => show(client, id, json).await,
    }
}

async fn list(client: &ApiClient, json: bool) -> RmsResult<()> {
    let resp: DeploymentsResponse = client.get("/deployments").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    if resp.deployments.is_empty() {
        output::section("◈", "Deployments");
        output::info("No deployments found.");
        println!();
        return Ok(());
    }

    output::section("◈", "Deployments");
    output::table_header(&[("ID", 18), ("STRATEGY", 14), ("STATUS", 14), ("STAGE", 10), ("ROBOTS", 8)]);

    for d in &resp.deployments {
        let strategy = d.strategy.as_deref().unwrap_or("-");
        let status = d.status.as_deref().unwrap_or("unknown");
        let stage = match (d.current_stage, d.total_stages) {
            (Some(c), Some(t)) => format!("{c}/{t}"),
            _ => "-".into(),
        };
        let robots = d.deployed_count.map(|v| v.to_string()).unwrap_or("-".into());

        println!(
            "    {} {} {} {} {}",
            output::pad(&output::truncate(&d.id, 18), 18),
            output::pad(strategy, 14),
            output::pad(&output::status_indicator(status), 14),
            output::pad(&stage, 10),
            robots
        );
    }

    let total = resp.pagination.as_ref().map(|p| p.total).unwrap_or(resp.deployments.len() as u32);
    output::count_label("deployment(s)", total);
    println!();

    Ok(())
}

async fn show(client: &ApiClient, id: &str, json: bool) -> RmsResult<()> {
    let resp: DeploymentDetailResponse = client.get(&format!("/deployments/{id}")).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    let d = &resp.deployment;
    output::section("◈", &format!("Deployment: {}", id.bold()));

    output::field("ID", &d.id);
    if let Some(v) = &d.strategy {
        output::field("Strategy", v);
    }
    if let Some(v) = &d.status {
        output::field_colored("Status", output::status_indicator(v).normal());
    }
    if let Some(v) = &d.model_version_id {
        output::field("Model Version", v);
    }

    println!();
    println!(
        "    {} {}",
        "│".bright_black(),
        "Rollout".bold().bright_black()
    );
    if let (Some(c), Some(t)) = (d.current_stage, d.total_stages) {
        output::meter("Stage", c as f64, t as f64, "orange");
    }
    if let Some(v) = d.eligible_robot_count {
        output::field("Eligible", &v.to_string());
    }
    if let Some(v) = d.deployed_count {
        output::field("Deployed", &v.to_string());
    }
    if let Some(v) = d.failed_count {
        if v > 0 {
            output::stat_line("▲", "Failed", &v.to_string(), "red");
        }
    }

    if let Some(v) = &d.created_at {
        println!();
        output::field("Created", v);
    }

    println!();

    Ok(())
}
