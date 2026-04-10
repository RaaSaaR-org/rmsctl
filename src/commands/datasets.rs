use crate::cli::DatasetsCommand;
use crate::client::ApiClient;
use crate::error::RmsResult;
use crate::models::*;
use crate::output;
use colored::*;

pub async fn run(cmd: &DatasetsCommand, client: &ApiClient, json: bool) -> RmsResult<()> {
    match cmd {
        DatasetsCommand::List => list(client, json).await,
        DatasetsCommand::Show { id } => show(client, id, json).await,
        DatasetsCommand::Episodes { id } => episodes(client, id, json).await,
    }
}

async fn list(client: &ApiClient, json: bool) -> RmsResult<()> {
    let resp: DatasetsResponse = client.get("/datasets").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    if resp.datasets.is_empty() {
        output::section("◈", "Datasets");
        output::info("No datasets found.");
        println!();
        return Ok(());
    }

    output::section("◈", "Datasets");
    output::table_header(&[("NAME", 24), ("STATUS", 14), ("DEMOS", 8), ("FRAMES", 10), ("QUALITY", 8)]);

    for d in &resp.datasets {
        let name = d.name.as_deref().unwrap_or(&d.id);
        let status = d.status.as_deref().unwrap_or("unknown");
        let demos = d.demonstration_count.map(|v| v.to_string()).unwrap_or("-".into());
        let frames = d.total_frames.map(|v| v.to_string()).unwrap_or("-".into());
        let quality = d.quality_score.map(|v| format!("{v:.0}%")).unwrap_or("-".into());

        println!(
            "    {} {} {} {} {}",
            output::pad(&output::truncate(name, 24).bold().to_string(), 24),
            output::pad(&output::status_indicator(status), 14),
            output::pad(&demos, 8),
            output::pad(&frames, 10),
            quality
        );
    }

    let total = resp.pagination.as_ref().map(|p| p.total).unwrap_or(resp.datasets.len() as u32);
    output::count_label("dataset(s)", total);
    println!();

    Ok(())
}

async fn show(client: &ApiClient, id: &str, json: bool) -> RmsResult<()> {
    let resp: serde_json::Value = client.get(&format!("/datasets/{id}")).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    let ds = resp.get("dataset").unwrap_or(&resp);
    let name = ds.get("name").and_then(|v| v.as_str()).unwrap_or(id);

    output::section("◈", &format!("Dataset: {}", name.bold()));

    output::field("ID", id);
    if let Some(v) = ds.get("name").and_then(|v| v.as_str()) {
        output::field("Name", v);
    }
    if let Some(v) = ds.get("status").and_then(|v| v.as_str()) {
        output::field_colored("Status", output::status_indicator(v).normal());
    }
    if let Some(v) = ds.get("description").and_then(|v| v.as_str()) {
        output::field("Description", v);
    }

    println!();
    println!(
        "    {} {}",
        "│".bright_black(),
        "Data".bold().bright_black()
    );
    if let Some(v) = ds.get("demonstrationCount").and_then(|v| v.as_u64()) {
        output::field("Demos", &v.to_string());
    }
    if let Some(v) = ds.get("totalFrames").and_then(|v| v.as_u64()) {
        output::field("Total Frames", &v.to_string());
    }
    if let Some(v) = ds.get("fps").and_then(|v| v.as_f64()) {
        output::field("FPS", &format!("{v:.1}"));
    }
    if let Some(v) = ds.get("qualityScore").and_then(|v| v.as_f64()) {
        let color = if v >= 80.0 { "green" } else if v >= 50.0 { "yellow" } else { "red" };
        output::meter("Quality", v, 100.0, color);
    }

    println!();
    println!(
        "    {} {}",
        "│".bright_black(),
        "Config".bold().bright_black()
    );
    if let Some(v) = ds.get("robotTypeId").and_then(|v| v.as_str()) {
        output::field("Robot Type", v);
    }
    if let Some(v) = ds.get("skillId").and_then(|v| v.as_str()) {
        output::field("Skill", v);
    }
    if let Some(v) = ds.get("createdAt").and_then(|v| v.as_str()) {
        output::field("Created", v);
    }

    println!();

    Ok(())
}

async fn episodes(client: &ApiClient, id: &str, json: bool) -> RmsResult<()> {
    let resp: DatasetEpisodesResponse = client.get(&format!("/datasets/{id}/episodes")).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    if resp.episodes.is_empty() {
        output::section("◈", &format!("Episodes: {id}"));
        output::info("No episodes in this dataset.");
        println!();
        return Ok(());
    }

    output::section("◈", &format!("Episodes: {}", id.bold()));
    output::table_header(&[("INDEX", 8), ("FRAMES", 10), ("DURATION", 10), ("FLAG", 6)]);

    for ep in &resp.episodes {
        let idx = ep.index.map(|v| v.to_string()).unwrap_or("-".into());
        let frames = ep.frame_count.map(|v| v.to_string()).unwrap_or("-".into());
        let dur = ep.duration_seconds.map(|v| format!("{v:.1}s")).unwrap_or("-".into());
        let flag = if ep.flagged == Some(true) {
            "!".red().to_string()
        } else {
            "·".bright_black().to_string()
        };

        println!(
            "    {} {} {} {}",
            output::pad(&idx, 8),
            output::pad(&frames, 10),
            output::pad(&dur, 10),
            flag
        );
    }

    output::count_label("episode(s)", resp.episodes.len() as u32);
    println!();

    Ok(())
}
