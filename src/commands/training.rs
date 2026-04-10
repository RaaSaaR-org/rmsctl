use crate::cli::TrainingCommand;
use crate::client::ApiClient;
use crate::error::RmsResult;
use crate::models::*;
use crate::output;
use colored::*;

pub async fn run(cmd: &TrainingCommand, client: &ApiClient, json: bool) -> RmsResult<()> {
    match cmd {
        TrainingCommand::List => list(client, json).await,
        TrainingCommand::Show { id } => show(client, id, json).await,
        TrainingCommand::Active => active(client, json).await,
        TrainingCommand::Queue => queue(client, json).await,
    }
}

async fn list(client: &ApiClient, json: bool) -> RmsResult<()> {
    let resp: TrainingJobsResponse = client.get("/training/jobs").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    if resp.jobs.is_empty() {
        output::section("◈", "Training Jobs");
        output::info("No training jobs found.");
        println!();
        return Ok(());
    }

    output::section("◈", "Training Jobs");
    output::table_header(&[("ID", 18), ("MODEL", 14), ("METHOD", 10), ("STATUS", 14), ("PROGRESS", 10)]);

    for j in &resp.jobs {
        let model = j.base_model.as_deref().unwrap_or("-");
        let method = j.fine_tune_method.as_deref().unwrap_or("-");
        let status = j.status.as_deref().unwrap_or("unknown");
        let progress = j.progress.map(|v| format!("{v:.0}%")).unwrap_or("-".into());

        println!(
            "    {} {} {} {} {}",
            output::pad(&output::truncate(&j.id, 18), 18),
            output::pad(model, 14),
            output::pad(method, 10),
            output::pad(&output::status_indicator(status), 14),
            progress
        );
    }

    let total = resp.pagination.as_ref().map(|p| p.total).unwrap_or(resp.jobs.len() as u32);
    output::count_label("job(s)", total);
    println!();

    Ok(())
}

async fn show(client: &ApiClient, id: &str, json: bool) -> RmsResult<()> {
    let resp: TrainingJobDetailResponse = client.get(&format!("/training/jobs/{id}")).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    let j = &resp.job;
    output::section("◈", &format!("Training Job: {}", id.bold()));

    output::field("ID", &j.id);
    if let Some(v) = &j.base_model {
        output::field("Base Model", v);
    }
    if let Some(v) = &j.fine_tune_method {
        output::field("Method", v);
    }
    if let Some(v) = &j.dataset_id {
        output::field("Dataset", v);
    }
    if let Some(v) = &j.status {
        output::field_colored("Status", output::status_indicator(v).normal());
    }

    println!();
    println!(
        "    {} {}",
        "│".bright_black(),
        "Progress".bold().bright_black()
    );
    if let Some(v) = j.progress {
        let color = if v >= 100.0 { "green" } else { "orange" };
        output::meter("Completion", v, 100.0, color);
    }
    if let (Some(cur), Some(total)) = (j.current_epoch, j.total_epochs) {
        output::field("Epoch", &format!("{cur} / {total}"));
    }
    if let Some(v) = j.current_loss {
        output::field("Loss", &format!("{v:.6}"));
    }
    if let Some(v) = j.learning_rate {
        output::field("Learning Rate", &format!("{v:.2e}"));
    }

    println!();
    println!(
        "    {} {}",
        "│".bright_black(),
        "Timeline".bold().bright_black()
    );
    if let Some(v) = &j.created_at {
        output::field("Created", v);
    }
    if let Some(v) = &j.started_at {
        output::field("Started", v);
    }
    if let Some(v) = &j.completed_at {
        output::field("Completed", v);
    }
    if let Some(v) = &j.error {
        println!();
        println!(
            "    {} {} {}",
            "▲".red(),
            "Error:".red().bold(),
            v.red()
        );
    }

    println!();

    Ok(())
}

async fn active(client: &ApiClient, json: bool) -> RmsResult<()> {
    let resp: TrainingActiveResponse = client.get("/training/active").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    if resp.jobs.is_empty() {
        output::section("◈", "Active Training");
        output::info("No training jobs currently running.");
        println!();
        return Ok(());
    }

    output::section("◈", "Active Training");

    for j in &resp.jobs {
        let model = j.base_model.as_deref().unwrap_or("-");
        let method = j.fine_tune_method.as_deref().unwrap_or("-");

        println!(
            "    {} {} {} {}",
            "│".bright_black(),
            j.id.bold(),
            model.bright_black(),
            method.bright_black()
        );

        if let Some(v) = j.progress {
            output::meter("Progress", v, 100.0, "orange");
        }
        if let (Some(cur), Some(total)) = (j.current_epoch, j.total_epochs) {
            output::field("Epoch", &format!("{cur} / {total}"));
        }
        if let Some(v) = j.current_loss {
            output::field("Loss", &format!("{v:.6}"));
        }
        println!();
    }

    Ok(())
}

async fn queue(client: &ApiClient, json: bool) -> RmsResult<()> {
    let resp: QueueStats = client.get("/training/queue/stats").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    output::section("◈", "Training Queue");

    output::stat_line(
        "│",
        "Pending",
        &resp.pending.unwrap_or(0).to_string(),
        if resp.pending.unwrap_or(0) > 0 { "yellow" } else { "dim" },
    );
    output::stat_line(
        "│",
        "Active",
        &resp.active.unwrap_or(0).to_string(),
        if resp.active.unwrap_or(0) > 0 { "orange" } else { "dim" },
    );
    output::stat_line(
        "│",
        "Completed",
        &resp.completed.unwrap_or(0).to_string(),
        if resp.completed.unwrap_or(0) > 0 { "green" } else { "dim" },
    );
    output::stat_line(
        "│",
        "Failed",
        &resp.failed.unwrap_or(0).to_string(),
        if resp.failed.unwrap_or(0) > 0 { "red" } else { "dim" },
    );

    println!();

    Ok(())
}
