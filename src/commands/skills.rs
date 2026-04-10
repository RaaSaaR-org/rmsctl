use crate::cli::SkillsCommand;
use crate::client::ApiClient;
use crate::error::RmsResult;
use crate::models::*;
use crate::output;
use colored::*;

pub async fn run(cmd: &SkillsCommand, client: &ApiClient, json: bool) -> RmsResult<()> {
    match cmd {
        SkillsCommand::List => list(client, json).await,
        SkillsCommand::Show { id } => show(client, id, json).await,
    }
}

async fn list(client: &ApiClient, json: bool) -> RmsResult<()> {
    let resp: SkillsResponse = client.get("/skills").await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    if resp.skills.is_empty() {
        output::section("◈", "VLA Skills");
        output::info("No skills registered.");
        println!();
        return Ok(());
    }

    output::section("◈", "VLA Skills");
    output::table_header(&[("NAME", 24), ("VERSION", 10), ("STATUS", 14), ("ROBOT TYPE", 16)]);

    for s in &resp.skills {
        let name = s.name.as_deref().unwrap_or(&s.id);
        let version = s.version.as_deref().unwrap_or("-");
        let status = s.status.as_deref().unwrap_or("unknown");
        let robot_type = s.robot_type_id.as_deref().unwrap_or("-");

        println!(
            "    {} {} {} {}",
            output::pad(&output::truncate(name, 24).bold().to_string(), 24),
            output::pad(version, 10),
            output::pad(&output::status_indicator(status), 14),
            robot_type.bright_black()
        );
    }

    let total = resp.pagination.as_ref().map(|p| p.total).unwrap_or(resp.skills.len() as u32);
    output::count_label("skill(s)", total);
    println!();

    Ok(())
}

async fn show(client: &ApiClient, id: &str, json: bool) -> RmsResult<()> {
    let resp: SkillDetailResponse = client.get(&format!("/skills/{id}")).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    let s = &resp.skill;
    let name = s.name.as_deref().unwrap_or(&s.id);
    output::section("◈", &format!("Skill: {}", name.bold()));

    output::field("ID", &s.id);
    if let Some(v) = &s.name {
        output::field("Name", v);
    }
    if let Some(v) = &s.version {
        output::field("Version", v);
    }
    if let Some(v) = &s.description {
        output::field("Description", v);
    }
    if let Some(v) = &s.status {
        output::field_colored("Status", output::status_indicator(v).normal());
    }
    if let Some(v) = &s.robot_type_id {
        output::field("Robot Type", v);
    }
    if let Some(v) = &s.linked_model_version_id {
        output::field("Model Version", v);
    }

    if let Some(params) = &s.parameters {
        if params.is_object() && !params.as_object().unwrap().is_empty() {
            println!();
            println!(
                "    {} {}",
                "│".bright_black(),
                "Parameters".bold().bright_black()
            );
            if let Some(obj) = params.as_object() {
                for (key, val) in obj {
                    let display = match val {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        other => serde_json::to_string(other).unwrap_or_default(),
                    };
                    output::field(key, &display);
                }
            }
        }
    }

    println!();

    Ok(())
}
