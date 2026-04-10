use crate::cli::EvalCommand;
use crate::client::ApiClient;
use crate::error::RmsResult;
use crate::models::*;
use crate::output;
use colored::*;

pub async fn run(cmd: &EvalCommand, client: &ApiClient, json: bool) -> RmsResult<()> {
    match cmd {
        EvalCommand::Episodes {
            robot_id,
            model_version,
        } => episodes(client, robot_id.as_deref(), model_version.as_deref(), json).await,
        EvalCommand::SuccessRate {
            robot_id,
            model_version,
        } => success_rate(client, robot_id.as_deref(), model_version.as_deref(), json).await,
        EvalCommand::Compare {
            version_a,
            version_b,
        } => compare(client, version_a, version_b, json).await,
    }
}

async fn episodes(
    client: &ApiClient,
    robot_id: Option<&str>,
    model_version: Option<&str>,
    json: bool,
) -> RmsResult<()> {
    let mut query: Vec<(&str, &str)> = Vec::new();
    if let Some(v) = robot_id {
        query.push(("robotId", v));
    }
    if let Some(v) = model_version {
        query.push(("modelVersion", v));
    }

    let resp: EvalEpisodesResponse = if query.is_empty() {
        client.get("/evaluation/episodes").await?
    } else {
        client
            .get_with_query("/evaluation/episodes", &query)
            .await?
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    if resp.episodes.is_empty() {
        output::section("◈", "Evaluation Episodes");
        output::info("No evaluation episodes found.");
        println!();
        return Ok(());
    }

    output::section("◈", "Evaluation Episodes");
    output::table_header(&[("ROBOT", 16), ("MODEL", 14), ("TASK", 24), ("RESULT", 10), ("DURATION", 10)]);

    for ep in &resp.episodes {
        let robot = ep.robot_id.as_deref().unwrap_or("-");
        let model = ep.model_version.as_deref().unwrap_or("-");
        let task = ep.task_prompt.as_deref().unwrap_or("-");
        let result = match ep.success {
            Some(true) => "pass".green().bold().to_string(),
            Some(false) => "fail".red().bold().to_string(),
            None => "-".bright_black().to_string(),
        };
        let dur = ep
            .duration_ms
            .map(|v| format!("{:.1}s", v as f64 / 1000.0))
            .unwrap_or("-".into());

        println!(
            "    {} {} {} {} {}",
            output::pad(robot, 16),
            output::pad(model, 14),
            output::pad(&output::truncate(task, 24), 24),
            output::pad(&result, 10),
            dur
        );
    }

    let total = resp
        .pagination
        .as_ref()
        .map(|p| p.total)
        .unwrap_or(resp.episodes.len() as u32);
    output::count_label("episode(s)", total);
    println!();

    Ok(())
}

async fn success_rate(
    client: &ApiClient,
    robot_id: Option<&str>,
    model_version: Option<&str>,
    json: bool,
) -> RmsResult<()> {
    let mut query: Vec<(&str, &str)> = Vec::new();
    if let Some(v) = robot_id {
        query.push(("robotId", v));
    }
    if let Some(v) = model_version {
        query.push(("modelVersion", v));
    }

    let resp: SuccessRate = if query.is_empty() {
        client.get("/evaluation/success-rate").await?
    } else {
        client
            .get_with_query("/evaluation/success-rate", &query)
            .await?
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    output::section("◈", "Evaluation Success Rate");

    if let Some(v) = &resp.model_version {
        output::field("Model Version", v);
    }
    if let Some(v) = resp.total_episodes {
        output::field("Total Episodes", &v.to_string());
    }
    if let Some(v) = resp.success_count {
        output::field("Successes", &v.to_string());
    }
    if let Some(rate) = resp.success_rate {
        let color = if rate >= 80.0 {
            "green"
        } else if rate >= 50.0 {
            "yellow"
        } else {
            "red"
        };
        output::meter("Success Rate", rate, 100.0, color);
    }

    println!();

    Ok(())
}

async fn compare(
    client: &ApiClient,
    version_a: &str,
    version_b: &str,
    json: bool,
) -> RmsResult<()> {
    let resp: EvalComparison = client
        .get_with_query(
            "/evaluation/compare",
            &[("versionA", version_a), ("versionB", version_b)],
        )
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    output::section("◈", "Model Comparison");

    let va = resp.version_a.as_deref().unwrap_or(version_a);
    let vb = resp.version_b.as_deref().unwrap_or(version_b);

    output::field("Version A", va);
    if let Some(rate) = resp.success_rate_a {
        output::meter("Success", rate, 100.0, "orange");
    }

    println!();

    output::field("Version B", vb);
    if let Some(rate) = resp.success_rate_b {
        output::meter("Success", rate, 100.0, "orange");
    }

    if let Some(imp) = resp.improvement {
        println!();
        let (label, color) = if imp >= 0.0 {
            (format!("+{imp:.1}%"), "green")
        } else {
            (format!("{imp:.1}%"), "red")
        };
        output::stat_line("◈", "Improvement", &label, color);
    }

    println!();

    Ok(())
}
