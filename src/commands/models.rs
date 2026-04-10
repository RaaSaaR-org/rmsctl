use crate::cli::ModelsCommand;
use crate::client::ApiClient;
use crate::error::RmsResult;
use crate::models::*;
use crate::output;
use colored::*;

pub async fn run(cmd: &ModelsCommand, client: &ApiClient, json: bool) -> RmsResult<()> {
    match cmd {
        ModelsCommand::List => list(client, json).await,
        ModelsCommand::Show { name } => show(client, name, json).await,
        ModelsCommand::Compare {
            run_a,
            run_b,
            metrics,
        } => compare(client, run_a, run_b, metrics, json).await,
    }
}

async fn list(client: &ApiClient, json: bool) -> RmsResult<()> {
    let resp: RegistryResponse = client.get("/models/registry").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    if resp.registered_models.is_empty() {
        output::section("◈", "Model Registry");
        output::info("No models registered.");
        println!();
        return Ok(());
    }

    output::section("◈", "Model Registry");
    output::table_header(&[("NAME", 28), ("VERSIONS", 10), ("LATEST STAGE", 16)]);

    for m in &resp.registered_models {
        let name = m.name.as_deref().unwrap_or("-");
        let version_count = m
            .latest_versions
            .as_ref()
            .map(|v| v.len().to_string())
            .unwrap_or("-".into());
        let stage = m
            .latest_versions
            .as_ref()
            .and_then(|v| v.first())
            .and_then(|v| v.current_stage.as_deref())
            .unwrap_or("-");

        println!(
            "    {} {} {}",
            output::pad(&name.bold().to_string(), 28),
            output::pad(&version_count, 10),
            output::pad(&output::status_indicator(stage), 16)
        );
    }

    let total = resp.count.unwrap_or(resp.registered_models.len() as u32);
    output::count_label("model(s)", total);
    println!();

    Ok(())
}

async fn show(client: &ApiClient, name: &str, json: bool) -> RmsResult<()> {
    let detail_path = format!("/models/registry/{name}");
    let versions_path = format!("/models/registry/{name}/versions");
    let (detail, versions) = tokio::join!(
        client.get::<ModelDetailResponse>(&detail_path),
        client.get::<ModelVersionsResponse>(&versions_path)
    );

    let detail = detail?;
    let versions = versions?;

    if json {
        let combined = serde_json::json!({
            "model": detail.registered_model,
            "versions": versions.versions,
        });
        println!("{}", serde_json::to_string_pretty(&combined)?);
        return Ok(());
    }

    output::section("◈", &format!("Model: {}", name.bold()));

    output::field("Name", name);

    if !versions.versions.is_empty() {
        println!();
        println!(
            "    {} {}",
            "│".bright_black(),
            "Versions".bold().bright_black()
        );
        output::table_header(&[("VERSION", 10), ("STAGE", 14), ("STATUS", 14), ("RUN", 20)]);

        for v in &versions.versions {
            let ver = v.version.as_deref().unwrap_or("-");
            let stage = v.current_stage.as_deref().unwrap_or("-");
            let status = v.status.as_deref().unwrap_or("-");
            let run = v.run_id.as_deref().unwrap_or("-");

            println!(
                "    {} {} {} {}",
                output::pad(ver, 10),
                output::pad(&output::status_indicator(stage), 14),
                output::pad(&output::status_indicator(status), 14),
                output::truncate(run, 20).bright_black()
            );
        }
    }

    println!();

    Ok(())
}

async fn compare(
    client: &ApiClient,
    run_a: &str,
    run_b: &str,
    metrics: &str,
    json: bool,
) -> RmsResult<()> {
    let resp: serde_json::Value = client
        .get_with_query(
            "/models/compare",
            &[("runIds", &format!("{run_a},{run_b}")), ("metricKeys", metrics)],
        )
        .await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    output::section("◈", "Model Comparison");

    output::field("Run A", run_a);
    output::field("Run B", run_b);
    output::field("Metrics", metrics);

    println!();

    // Display comparison data generically
    if let Some(obj) = resp.as_object() {
        for (key, val) in obj {
            let display = match val {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                other => serde_json::to_string_pretty(other).unwrap_or_default(),
            };
            output::field(key, &display);
        }
    }

    println!();

    Ok(())
}
