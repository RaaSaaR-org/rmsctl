use crate::cli::EmbodimentsCommand;
use crate::client::ApiClient;
use crate::error::RmsResult;
use crate::models::*;
use crate::output;
use colored::*;

pub async fn run(cmd: &EmbodimentsCommand, client: &ApiClient, json: bool) -> RmsResult<()> {
    match cmd {
        EmbodimentsCommand::List => list(client, json).await,
        EmbodimentsCommand::Show { tag } => show(client, tag, json).await,
    }
}

async fn list(client: &ApiClient, json: bool) -> RmsResult<()> {
    let resp: EmbodimentsResponse = client.get("/embodiments").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    if resp.embodiments.is_empty() {
        output::section("◈", "Embodiments");
        output::info("No embodiment configs found.");
        println!();
        return Ok(());
    }

    output::section("◈", "Embodiments");
    output::table_header(&[("TAG", 18), ("MANUFACTURER", 16), ("MODEL", 16), ("ACT DIM", 8), ("PROP DIM", 8)]);

    for e in &resp.embodiments {
        let tag = e.tag.as_deref().unwrap_or("-");
        let mfr = e.manufacturer.as_deref().unwrap_or("-");
        let model = e.model.as_deref().unwrap_or("-");
        let act = e.action_dim.map(|v| v.to_string()).unwrap_or("-".into());
        let prop = e.proprioception_dim.map(|v| v.to_string()).unwrap_or("-".into());

        println!(
            "    {} {} {} {} {}",
            output::pad(&tag.bold().to_string(), 18),
            output::pad(mfr, 16),
            output::pad(model, 16),
            output::pad(&act, 8),
            prop
        );
    }

    let total = resp.pagination.as_ref().map(|p| p.total).unwrap_or(resp.embodiments.len() as u32);
    output::count_label("embodiment(s)", total);
    println!();

    Ok(())
}

async fn show(client: &ApiClient, tag: &str, json: bool) -> RmsResult<()> {
    let detail_path = format!("/embodiments/{tag}");
    let config_path = format!("/embodiments/{tag}/config");
    let (detail, config) = tokio::join!(
        client.get::<EmbodimentDetailResponse>(&detail_path),
        client.get::<EmbodimentConfigResponse>(&config_path)
    );

    let detail = detail?;

    if json {
        let config_data = config.ok();
        let combined = serde_json::json!({
            "embodiment": detail.embodiment,
            "config": config_data.and_then(|c| c.config),
        });
        println!("{}", serde_json::to_string_pretty(&combined)?);
        return Ok(());
    }

    let e = &detail.embodiment;
    output::section("◈", &format!("Embodiment: {}", tag.bold()));

    if let Some(v) = &e.tag {
        output::field("Tag", v);
    }
    if let Some(v) = &e.manufacturer {
        output::field("Manufacturer", v);
    }
    if let Some(v) = &e.model {
        output::field("Model", v);
    }
    if let Some(v) = e.action_dim {
        output::field("Action Dim", &v.to_string());
    }
    if let Some(v) = e.proprioception_dim {
        output::field("Proprio Dim", &v.to_string());
    }
    if let Some(v) = &e.robot_type_id {
        output::field("Robot Type", v);
    }

    // Show parsed config if available
    if let Ok(cfg) = config {
        if let Some(config_val) = &cfg.config {
            println!();
            println!(
                "    {} {}",
                "│".bright_black(),
                "Configuration".bold().bright_black()
            );
            if let Some(obj) = config_val.as_object() {
                for (key, val) in obj {
                    let display = match val {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        serde_json::Value::Null => "-".to_string(),
                        other => serde_json::to_string(other).unwrap_or_default(),
                    };
                    output::field(key, &display);
                }
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(config_val).unwrap_or_default()
                );
            }
        }
    }

    println!();

    Ok(())
}
