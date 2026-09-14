use crate::cli::SkillsCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::models::Pagination;
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub async fn run(cmd: &SkillsCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        SkillsCommand::List { published } => list(client, ctx, *published).await,
        SkillsCommand::Show { id } => show(client, ctx, id).await,
        SkillsCommand::ForRobot { robot_id } => for_robot(client, ctx, robot_id).await,
        SkillsCommand::Compatible { id } => compatible(client, ctx, id).await,
        SkillsCommand::Check { id, robot_id } => check(client, ctx, id, robot_id).await,
        SkillsCommand::Validate { id } => validate(client, ctx, id).await,
        SkillsCommand::Publish { id } => transition(client, ctx, id, Transition::Publish).await,
        SkillsCommand::Deprecate { id } => transition(client, ctx, id, Transition::Deprecate).await,
        SkillsCommand::Archive { id } => transition(client, ctx, id, Transition::Archive).await,
        SkillsCommand::Execute {
            id,
            robot_id,
            prompt,
        } => execute(client, ctx, id, robot_id, prompt.as_deref()).await,
        SkillsCommand::Abort { id } => abort(client, ctx, id).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────
//
// A skill carries no `robotTypeId`: the type relation is the plural
// `compatibleRobotTypes`, and only the detail route includes it. The list
// route does return `requiredCapabilities`, so that is what the table shows.

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Skill {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    parameters_schema: Option<Value>,
    #[serde(default)]
    default_parameters: Option<Value>,
    #[serde(default)]
    preconditions: Option<Vec<Condition>>,
    #[serde(default)]
    postconditions: Option<Vec<Condition>>,
    #[serde(default)]
    required_capabilities: Option<Vec<String>>,
    #[serde(default)]
    timeout: Option<f64>,
    #[serde(default)]
    max_retries: Option<u32>,
    #[serde(default)]
    linked_model_version_id: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    compatible_robot_types: Option<Vec<RobotTypeRef>>,
}

impl Skill {
    fn label(&self) -> String {
        match (&self.name, &self.version) {
            (Some(n), Some(v)) => format!("{n} v{v}"),
            (Some(n), None) => n.clone(),
            _ => self.id.clone(),
        }
    }

    fn capabilities(&self) -> String {
        match self.required_capabilities.as_deref() {
            Some(caps) if !caps.is_empty() => caps.join(", "),
            _ => "-".to_string(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RobotTypeRef {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Condition {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    check: Option<String>,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillsResponse {
    #[serde(default)]
    skills: Vec<Skill>,
    #[serde(default)]
    pagination: Option<Pagination>,
}

/// `/skills/published` and `/skills/for-robot/{id}` answer with a bare count,
/// not a pagination envelope.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillsCountResponse {
    #[serde(default)]
    skills: Vec<Skill>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    robot_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillDetailResponse {
    #[serde(default)]
    skill: Option<Skill>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RobotCompatibility {
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    robot_name: Option<String>,
    #[serde(default)]
    robot_type: Option<String>,
    #[serde(default)]
    compatible: Option<bool>,
    #[serde(default)]
    missing_capabilities: Option<Vec<String>>,
    #[serde(default)]
    matching_capabilities: Option<Vec<String>>,
}

impl RobotCompatibility {
    /// The server calls a robot incompatible for two different reasons; only
    /// one of them shows up in `missingCapabilities`. An empty list next to
    /// `compatible: false` means the robot's *type* is not in the skill's
    /// `compatibleRobotTypes`.
    fn type_mismatch(&self) -> bool {
        self.compatible == Some(false)
            && self
                .missing_capabilities
                .as_deref()
                .is_none_or(|m| m.is_empty())
    }
}

/// `/skills/{id}/compatible-robots` answers with `res.json(result)` — flat, no
/// wrapper key.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatibilityReport {
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    total_robots: Option<u32>,
    #[serde(default)]
    compatible_robots: Option<u32>,
    #[serde(default)]
    robots: Vec<RobotCompatibility>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationResponse {
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    valid: Option<bool>,
    #[serde(default)]
    errors: Option<Vec<ValidationError>>,
    #[serde(default)]
    coerced_parameters: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationError {
    #[serde(default)]
    path: Option<Vec<Value>>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillMutationResponse {
    #[serde(default)]
    skill: Option<Skill>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExecuteResponse {
    #[serde(default)]
    result: Option<ExecutionResult>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExecutionResult {
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    parameters: Option<Value>,
    #[serde(default)]
    output: Option<Value>,
    #[serde(default)]
    precondition_results: Option<Vec<ConditionCheckResult>>,
    #[serde(default)]
    postcondition_results: Option<Vec<ConditionCheckResult>>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    retry_count: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConditionCheckResult {
    #[serde(default)]
    condition: Option<Condition>,
    #[serde(default)]
    passed: Option<bool>,
    #[serde(default)]
    actual_value: Option<Value>,
    #[serde(default)]
    expected_value: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

/// Only the display name is needed, to name the robot in the abort gate.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RobotDetail {
    #[serde(default)]
    name: Option<String>,
}

// ── Argument guards ─────────────────────────────────────────────────────

/// `/skills/published`, `/skills/chains` and `/skills/for-robot` are registered
/// *before* `/skills/:id`, so interpolating one of those words into the detail
/// path returns HTTP 200 with a list envelope — a detail heading over the wrong
/// payload. Everything that goes into a `{id}` segment is checked here instead.
const RESERVED_SEGMENTS: [&str; 3] = ["published", "chains", "for-robot"];

fn require_skill_id(id: &str) -> RmsResult<&str> {
    let id = id.trim();

    if RESERVED_SEGMENTS.contains(&id) {
        return Err(RmsError::InvalidInput(format!(
            "`{id}` is a sibling route under /skills, not a skill id — \
             asking for it would print a list of skills under a detail heading. \
             Run `rmsctl skills list` and pass one of the ids it prints."
        )));
    }
    if !is_uuid_shaped(id) {
        return Err(RmsError::InvalidInput(format!(
            "`{id}` is not a skill id: skill ids are UUIDs \
             (e.g. 4ecc3005-1a8a-45b1-b31c-9b12fd5b68f8). \
             Run `rmsctl skills list` to see them."
        )));
    }
    Ok(id)
}

fn is_uuid_shaped(s: &str) -> bool {
    let groups = [8usize, 4, 4, 4, 12];
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == groups.len()
        && parts
            .iter()
            .zip(groups)
            .all(|(part, len)| part.len() == len && part.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Robot ids are not UUIDs (`sim-robot-001`), so only shape that would break
/// the URL is rejected.
fn require_robot_id(robot_id: &str) -> RmsResult<&str> {
    let robot_id = robot_id.trim();
    if robot_id.is_empty() || robot_id.contains('/') || robot_id.contains(char::is_whitespace) {
        return Err(RmsError::InvalidInput(format!(
            "`{robot_id}` is not a robot id — run `rmsctl robots list`."
        )));
    }
    Ok(robot_id)
}

async fn fetch_skill(client: &ApiClient, id: &str) -> RmsResult<Skill> {
    let resp: SkillDetailResponse = client.get(&format!("/skills/{id}")).await?;
    resp.skill
        .ok_or_else(|| RmsError::Parse(format!("the server returned no skill for {id}")))
}

// ── list ────────────────────────────────────────────────────────────────

async fn list(client: &ApiClient, ctx: &Ctx, published: bool) -> RmsResult<()> {
    if published {
        // `/skills/published` takes no query parameters and returns everything.
        if ctx.json {
            return output::json_out(&client.get_value("/skills/published").await?);
        }
        let resp: SkillsCountResponse = client.get("/skills/published").await?;

        output::section("◈", "Published Skills");
        if ctx.has_paging() {
            output::info("--page/--limit do not apply here: the published list is unpaged.");
        }
        if resp.skills.is_empty() {
            output::info("No published skills. `rmsctl skills list` shows drafts too.");
            println!();
            return Ok(());
        }

        skill_table(&resp.skills);
        output::count_paged("published skill(s)", resp.skills.len(), resp.count, None);
        println!();
        return Ok(());
    }

    let query = ctx.paging();

    if ctx.json {
        return output::json_out(&client.get_value_query("/skills", &query).await?);
    }

    let resp: SkillsResponse = client.get_query("/skills", &query).await?;

    output::section("◈", "Skill Library");

    if resp.skills.is_empty() {
        output::info("No skills registered.");
        println!();
        return Ok(());
    }

    skill_table(&resp.skills);

    // The envelope's `total` is the unpaged count while the array is one page
    // (server default pageSize=50), so the footer must compare the two.
    let pag = resp.pagination.as_ref();
    output::count_paged(
        "skill(s)",
        resp.skills.len(),
        pag.and_then(|p| p.total()),
        pag.and_then(|p| p.total_pages()),
    );
    println!();
    Ok(())
}

fn skill_table(skills: &[Skill]) {
    output::table_header(&[
        ("ID", output::ID_W),
        ("NAME", 24),
        ("VER", 7),
        ("STATUS", 14),
        ("CAPABILITIES", 24),
    ]);
    for s in skills {
        println!(
            "    {} {} {} {} {}",
            output::pad(&s.id, output::ID_W),
            output::pad(&output::truncate(s.name.as_deref().unwrap_or("-"), 24), 24),
            output::pad(&output::truncate(s.version.as_deref().unwrap_or("-"), 7), 7),
            output::pad(
                &output::status_indicator(s.status.as_deref().unwrap_or("unknown")),
                14
            ),
            output::truncate(&s.capabilities(), 24)
        );
    }
}

// ── show ────────────────────────────────────────────────────────────────

async fn show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let id = require_skill_id(id)?;
    let path = format!("/skills/{id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let skill = fetch_skill(client, id).await?;

    output::section("◈", &format!("Skill: {}", skill.label().bold()));

    output::field("ID", &skill.id);
    if let Some(name) = &skill.name {
        output::field("Name", name);
    }
    output::field("Version", skill.version.as_deref().unwrap_or("-"));
    output::field(
        "Status",
        &output::status_indicator(skill.status.as_deref().unwrap_or("unknown")),
    );
    if let Some(desc) = &skill.description {
        render_paragraph("Description", desc);
    }
    if let Some(timeout) = skill.timeout {
        output::field("Timeout", &format!("{timeout:.0}s"));
    }
    if let Some(retries) = skill.max_retries {
        output::field("Max retries", &retries.to_string());
    }
    if let Some(model) = &skill.linked_model_version_id {
        output::field("Model version", model);
    }
    if let Some(created) = &skill.created_at {
        output::field("Created", created);
    }
    if let Some(updated) = &skill.updated_at {
        output::field("Updated", updated);
    }

    output::subsection("Required capabilities");
    match skill.required_capabilities.as_deref() {
        Some(caps) if !caps.is_empty() => {
            for cap in caps {
                output::field("·", cap);
            }
        }
        _ => output::info("None — no capability gate on this skill."),
    }

    output::subsection("Compatible robot types");
    match skill.compatible_robot_types.as_deref() {
        Some(types) if !types.is_empty() => {
            for t in types {
                let name = t
                    .name
                    .as_deref()
                    .or(t.model.as_deref())
                    .or(t.id.as_deref())
                    .unwrap_or("-");
                match t.manufacturer.as_deref() {
                    Some(m) if m != "Unknown" => output::field("·", &format!("{name} ({m})")),
                    _ => output::field("·", name),
                }
            }
        }
        // An empty relation is not "no robots" — the server treats it as
        // "every type passes the type check".
        _ => output::info("Any — the skill declares no robot-type restriction."),
    }

    output::subsection("Parameter contract");
    match &skill.parameters_schema {
        Some(schema) => render_param_contract(schema),
        None => output::info("No parametersSchema on this skill."),
    }

    output::subsection("Default parameters");
    match &skill.default_parameters {
        Some(defaults) => render_kv(defaults, "No default parameters."),
        None => output::info("No default parameters."),
    }

    render_conditions("Preconditions", skill.preconditions.as_deref());
    render_conditions("Postconditions", skill.postconditions.as_deref());

    println!();
    Ok(())
}

/// `parametersSchema` is JSON Schema: the contract lives in `properties`, and
/// which keys are mandatory lives in the sibling `required` array.
fn render_param_contract(schema: &Value) {
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    match schema.get("properties").and_then(|p| p.as_object()) {
        Some(props) if !props.is_empty() => {
            output::table_header(&[
                ("PARAMETER", 18),
                ("TYPE", 10),
                ("REQ", 4),
                ("DESCRIPTION", 40),
            ]);
            for (name, spec) in props {
                println!(
                    "    {} {} {} {}",
                    output::pad(&output::truncate(name, 18), 18),
                    output::pad(spec.get("type").and_then(|t| t.as_str()).unwrap_or("-"), 10),
                    output::pad(
                        &output::bool_indicator(Some(required.contains(&name.as_str()))),
                        4
                    ),
                    output::truncate(
                        spec.get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or("-"),
                        40
                    )
                );
            }
        }
        // A schema that is not an object-with-properties still carries the
        // contract, so print it rather than claim the skill takes nothing.
        _ => output::field("Schema", &compact(schema)),
    }
}

fn render_kv(value: &Value, empty_msg: &str) {
    match value.as_object() {
        Some(map) if !map.is_empty() => {
            for (k, v) in map {
                output::field(&output::truncate(k, 14), &scalar(v));
            }
        }
        Some(_) => output::info(empty_msg),
        None if value.is_null() => output::info(empty_msg),
        None => output::field("value", &compact(value)),
    }
}

fn render_conditions(label: &str, conditions: Option<&[Condition]>) {
    let conditions = match conditions {
        Some(c) if !c.is_empty() => c,
        _ => return,
    };
    output::subsection(label);
    for c in conditions {
        let name = c.name.as_deref().unwrap_or("-");
        let check = c.check.as_deref().unwrap_or("-");
        let kind = c.kind.as_deref().unwrap_or("-");
        output::field(&output::truncate(name, 14), &format!("{check} [{kind}]"));
        if let Some(params) = &c.params
            && !params.is_null()
        {
            output::field("", &compact(params));
        }
    }
}

/// `field()` is one line; a description is not. Continuation lines reuse the
/// same 14-column label gutter so the paragraph stays aligned.
fn render_paragraph(label: &str, text: &str) {
    const WIDTH: usize = 66;
    let mut line = String::new();
    let mut first = true;
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > WIDTH {
            output::field(if first { label } else { "" }, &line);
            first = false;
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() || first {
        output::field(if first { label } else { "" }, &line);
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "-".to_string(),
        other => other.to_string(),
    }
}

fn compact(v: &Value) -> String {
    output::truncate(&v.to_string(), 80)
}

// ── for-robot ───────────────────────────────────────────────────────────

async fn for_robot(client: &ApiClient, ctx: &Ctx, robot_id: &str) -> RmsResult<()> {
    let robot_id = require_robot_id(robot_id)?;
    let path = format!("/skills/for-robot/{robot_id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp: SkillsCountResponse = client
        .get(&path)
        .await
        .map_err(|e| explain_unknown_target(e, robot_id))?;

    output::section("◈", &format!("Skills for {}", robot_id.bold()));

    if resp.skills.is_empty() {
        output::info("No skill is both published and compatible with this robot.");
        println!();
        return Ok(());
    }

    skill_table(&resp.skills);
    // Published skills only — and when the robot's type is not a registered
    // robot type the server falls back to a list it builds without loading the
    // type relation, which lets every published skill through. `skills check`
    // is the authoritative answer, and it is what execute uses.
    output::info(
        "Published skills only. Confirm one with `rmsctl skills check <id> <robot-id>` \
         before running it.",
    );
    output::count_paged("skill(s)", resp.skills.len(), resp.count, None);
    println!();
    Ok(())
}

// ── compatible-robots / check-robot ─────────────────────────────────────

async fn compatible(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let id = require_skill_id(id)?;
    let path = format!("/skills/{id}/compatible-robots");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let report: CompatibilityReport = client
        .get(&path)
        .await
        .map_err(|e| explain_unknown_target(e, id))?;

    output::section("◈", &format!("Compatible robots: {id}"));

    output::stat_line(
        "│",
        "Robots checked",
        &report
            .total_robots
            .map(|t| t.to_string())
            .unwrap_or_else(|| report.robots.len().to_string()),
        "orange",
    );
    let ok = report.compatible_robots.unwrap_or_else(|| {
        report
            .robots
            .iter()
            .filter(|r| r.compatible == Some(true))
            .count() as u32
    });
    output::stat_line(
        if ok > 0 { "●" } else { "│" },
        "Compatible",
        &ok.to_string(),
        if ok > 0 { "green" } else { "yellow" },
    );

    if report.robots.is_empty() {
        println!();
        output::info("No robots registered.");
        println!();
        return Ok(());
    }

    println!();
    output::table_header(&[
        ("ROBOT", 24),
        ("NAME", 22),
        ("TYPE", 12),
        ("OK", 4),
        ("MISSING CAPABILITIES", 30),
    ]);
    for r in &report.robots {
        println!(
            "    {} {} {} {} {}",
            output::pad(
                &output::truncate(r.robot_id.as_deref().unwrap_or("-"), 24),
                24
            ),
            output::pad(
                &output::truncate(r.robot_name.as_deref().unwrap_or("-"), 22),
                22
            ),
            output::pad(
                &output::truncate(r.robot_type.as_deref().unwrap_or("-"), 12),
                12
            ),
            output::pad(&output::bool_indicator(r.compatible), 4),
            output::truncate(&caps_list(r.missing_capabilities.as_deref()), 30)
        );
    }

    if report.robots.iter().any(|r| r.type_mismatch()) {
        println!();
        output::info(
            "A robot marked incompatible with no missing capabilities failed the \
             robot-type check — see 'Compatible robot types' in `rmsctl skills show`.",
        );
    }

    output::count_paged("robot(s)", report.robots.len(), report.total_robots, None);
    println!();
    Ok(())
}

async fn check(client: &ApiClient, ctx: &Ctx, id: &str, robot_id: &str) -> RmsResult<()> {
    let id = require_skill_id(id)?;
    let robot_id = require_robot_id(robot_id)?;
    let path = format!("/skills/{id}/check-robot/{robot_id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let compat: RobotCompatibility = client
        .get(&path)
        .await
        .map_err(|e| explain_unknown_target(e, &format!("{id} / {robot_id}")))?;

    output::section("◈", &format!("Compatibility: {robot_id}"));
    output::field("Skill", id);
    render_compatibility(&compat);

    // The check route never says which types are allowed, so fetch the skill
    // when the type is the thing that failed.
    if compat.type_mismatch()
        && let Ok(skill) = fetch_skill(client, id).await
    {
        let allowed: Vec<String> = skill
            .compatible_robot_types
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|t| t.name.clone().or_else(|| t.id.clone()))
            .collect();
        if !allowed.is_empty() {
            output::field("Allowed types", &allowed.join(", "));
        }
    }

    println!();
    Ok(())
}

fn render_compatibility(c: &RobotCompatibility) {
    if let Some(name) = &c.robot_name {
        output::field("Robot", name);
    }
    if let Some(id) = &c.robot_id {
        output::field("Robot ID", id);
    }
    output::field("Robot type", c.robot_type.as_deref().unwrap_or("-"));
    output::field_colored(
        "Compatible",
        match c.compatible {
            Some(true) => "yes".green().bold(),
            Some(false) => "NO".red().bold(),
            None => "-".normal(),
        },
    );
    output::field(
        "Matching caps",
        &caps_list(c.matching_capabilities.as_deref()),
    );
    let missing = caps_list(c.missing_capabilities.as_deref());
    if c.missing_capabilities
        .as_deref()
        .is_some_and(|m| !m.is_empty())
    {
        output::field_colored("Missing caps", missing.yellow());
    } else {
        output::field("Missing caps", &missing);
    }
    if c.type_mismatch() {
        output::field_colored(
            "Blocked by",
            "robot type (capabilities are satisfied)".yellow(),
        );
    }
}

fn caps_list(caps: Option<&[String]>) -> String {
    match caps {
        Some(c) if !c.is_empty() => c.join(", "),
        _ => "-".to_string(),
    }
}

/// The compatibility and for-robot routes answer an unknown skill or robot with
/// a bare 400 and a generic sentence (`sendFailure` drops the real message), so
/// name the ids the request carried.
fn explain_unknown_target(e: RmsError, target: &str) -> RmsError {
    match e {
        RmsError::Api {
            status: 400,
            message,
        } => RmsError::Api {
            status: 400,
            message: format!(
                "{message} — the server says only this much. Check that {target} exists"
            ),
        },
        other => other,
    }
}

// ── validate ────────────────────────────────────────────────────────────

/// POST `/skills/{id}/validate` only computes a report — it writes nothing — so
/// unlike publish/deprecate/archive/execute/abort it is not put behind `--yes`.
async fn validate(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let id = require_skill_id(id)?;
    let skill = fetch_skill(client, id).await?;

    // The route rejects a body without `parameters`, and the only parameter set
    // the CLI can know about is the skill's own defaults.
    let parameters = match &skill.default_parameters {
        Some(p) if p.is_object() => p.clone(),
        _ => {
            return Err(RmsError::InvalidInput(format!(
                "skill {id} has no defaultParameters object to validate, and \
                 POST /skills/{{id}}/validate requires a parameters object"
            )));
        }
    };

    let body = serde_json::json!({ "parameters": parameters });
    let path = format!("/skills/{id}/validate");

    if ctx.json {
        return output::json_out(&client.post_value(&path, &body).await?);
    }

    let resp: ValidationResponse = client.post(&path, &body).await?;

    output::section("◈", &format!("Validate: {}", skill.label().bold()));
    output::info("Checking the skill's own defaultParameters against parametersSchema.");

    output::field_colored(
        "Valid",
        match resp.valid {
            Some(true) => "yes".green().bold(),
            Some(false) => "NO".red().bold(),
            None => "-".normal(),
        },
    );
    if let Some(skill_id) = &resp.skill_id {
        output::field("Skill", skill_id);
    }

    output::subsection("Parameters checked");
    render_kv(&parameters, "No parameters.");

    match resp.errors.as_deref() {
        Some(errors) if !errors.is_empty() => {
            output::subsection("Errors");
            output::table_header(&[("PATH", 22), ("CODE", 16), ("MESSAGE", 40)]);
            for e in errors {
                let path = e
                    .path
                    .as_deref()
                    .map(|p| p.iter().map(scalar).collect::<Vec<_>>().join("."))
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| "(root)".to_string());
                println!(
                    "    {} {} {}",
                    output::pad(&output::truncate(&path, 22), 22),
                    output::pad(&output::truncate(e.code.as_deref().unwrap_or("-"), 16), 16),
                    output::truncate(e.message.as_deref().unwrap_or("-"), 40)
                );
            }
        }
        _ => {}
    }

    // Coercion is what the robot would actually receive; show it when it
    // differs from what was sent.
    if let Some(coerced) = &resp.coerced_parameters
        && coerced != &parameters
    {
        output::subsection("After coercion");
        render_kv(coerced, "No parameters.");
    }

    println!();
    Ok(())
}

// ── publish / deprecate / archive ───────────────────────────────────────

#[derive(Copy, Clone)]
enum Transition {
    Publish,
    Deprecate,
    Archive,
}

impl Transition {
    fn segment(self) -> &'static str {
        match self {
            Transition::Publish => "publish",
            Transition::Deprecate => "deprecate",
            Transition::Archive => "archive",
        }
    }

    fn target_status(self) -> &'static str {
        match self {
            Transition::Publish => "published",
            Transition::Deprecate => "deprecated",
            Transition::Archive => "archived",
        }
    }

    /// The server enforces draft → published → deprecated → archived (and
    /// draft → archived), rejecting anything else with a 400 whose real message
    /// `sendFailure` throws away.
    fn allowed_from(self) -> &'static [&'static str] {
        match self {
            Transition::Publish => &["draft"],
            Transition::Deprecate => &["published"],
            Transition::Archive => &["deprecated", "draft"],
        }
    }

    fn consequence(self) -> &'static str {
        match self {
            Transition::Publish => "operators will be able to run it on the fleet",
            Transition::Deprecate => "it stays runnable but drops out of the published list",
            Transition::Archive => "it drops out of the library",
        }
    }
}

async fn transition(client: &ApiClient, ctx: &Ctx, id: &str, t: Transition) -> RmsResult<()> {
    let id = require_skill_id(id)?;
    let skill = fetch_skill(client, id).await?;
    let current = skill.status.as_deref().unwrap_or("unknown");

    if !t.allowed_from().contains(&current) {
        return Err(RmsError::InvalidInput(format!(
            "cannot {} a skill whose status is `{current}`: the server only allows it from {}",
            t.segment(),
            t.allowed_from().join(" or ")
        )));
    }

    if !output::confirm(
        &format!(
            "This will set skill {} ({}) from {current} to {} — {}",
            skill.label(),
            id,
            t.target_status(),
            t.consequence()
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let path = format!("/skills/{id}/{}", t.segment());
    let body = serde_json::json!({});

    if ctx.json {
        return output::json_out(&client.post_value(&path, &body).await?);
    }

    let resp: SkillMutationResponse = client.post(&path, &body).await?;

    output::success(&format!(
        "{} is now {}",
        skill.label(),
        resp.skill
            .as_ref()
            .and_then(|s| s.status.clone())
            .unwrap_or_else(|| t.target_status().to_string())
    ));
    if let Some(msg) = &resp.message {
        output::info(msg);
    }
    println!();
    Ok(())
}

// ── execute ─────────────────────────────────────────────────────────────

async fn execute(
    client: &ApiClient,
    ctx: &Ctx,
    id: &str,
    robot_id: &str,
    prompt: Option<&str>,
) -> RmsResult<()> {
    let id = require_skill_id(id)?;
    let robot_id = require_robot_id(robot_id)?;
    let prompt = prompt.map(str::trim).filter(|p| !p.is_empty());

    let skill = fetch_skill(client, id).await?;

    // The server refuses an incompatible robot *after* accepting the request
    // and answers 400 with the reason buried in a body the client discards, so
    // ask the read-only compatibility route first.
    let compat: RobotCompatibility = client
        .get(&format!("/skills/{id}/check-robot/{robot_id}"))
        .await
        .map_err(|e| explain_unknown_target(e, &format!("{id} / {robot_id}")))?;
    if compat.compatible != Some(true) {
        output::section("◈", &format!("Execute: {}", skill.label().bold()));
        render_compatibility(&compat);
        println!();
        return Err(RmsError::InvalidInput(format!(
            "robot {robot_id} is not compatible with skill {}: the server would reject \
             the execution. See `rmsctl skills check {id} {robot_id}`",
            skill.label()
        )));
    }

    let status_note = match skill.status.as_deref() {
        Some("published") => String::new(),
        Some(other) => format!(" (skill status: {other})"),
        None => String::new(),
    };

    if !output::confirm(
        &format!(
            "This will RUN skill {} on robot {robot_id}{status_note} — \
             the robot MOVES and keeps moving until the skill finishes, its {} timeout \
             expires, or you run `rmsctl skills abort {id}@{robot_id}`",
            skill.label(),
            skill
                .timeout
                .map(|t| format!("{t:.0}s"))
                .unwrap_or_else(|| "default".to_string())
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    // `parameters` is merged over the skill's defaults server-side, and the
    // robot agent reads the task prompt out of `parameters.taskPrompt`.
    let body = match prompt {
        Some(p) => serde_json::json!({ "robotId": robot_id, "parameters": { "taskPrompt": p } }),
        None => serde_json::json!({ "robotId": robot_id }),
    };
    let path = format!("/skills/{id}/execute");

    if ctx.json {
        return output::json_out(&client.post_value(&path, &body).await?);
    }

    let resp: ExecuteResponse = client
        .post(&path, &body)
        .await
        .map_err(|e| explain_failed_execution(e, id, robot_id))?;

    output::section("◈", &format!("Execute: {}", skill.label().bold()));

    match &resp.result {
        Some(r) => render_execution(r),
        None => output::info("The server returned no execution result."),
    }
    if let Some(msg) = &resp.message {
        output::info(msg);
    }
    println!();
    Ok(())
}

fn render_execution(r: &ExecutionResult) {
    output::field(
        "Status",
        &output::status_indicator(r.status.as_deref().unwrap_or("unknown")),
    );
    if let Some(robot) = &r.robot_id {
        output::field("Robot", robot);
    }
    if let Some(started) = &r.started_at {
        output::field("Started", started);
    }
    if let Some(completed) = &r.completed_at {
        output::field("Completed", completed);
    }
    if let Some(duration) = r.duration {
        output::field("Duration", &format!("{:.1}s", duration / 1000.0));
    }
    if let Some(retries) = r.retry_count {
        output::field("Retries", &retries.to_string());
    }
    if let Some(err) = &r.error {
        output::field_colored("Error", err.red());
    }

    if let Some(params) = &r.parameters {
        output::subsection("Parameters");
        render_kv(params, "No parameters.");
    }
    if let Some(out) = &r.output
        && !out.is_null()
    {
        output::subsection("Output");
        render_kv(out, "No output.");
    }

    render_condition_results("Precondition checks", r.precondition_results.as_deref());
    render_condition_results("Postcondition checks", r.postcondition_results.as_deref());
}

fn render_condition_results(label: &str, results: Option<&[ConditionCheckResult]>) {
    let results = match results {
        Some(r) if !r.is_empty() => r,
        _ => return,
    };
    output::subsection(label);
    for r in results {
        let name = r
            .condition
            .as_ref()
            .and_then(|c| c.name.clone())
            .unwrap_or_else(|| "-".to_string());
        let mut detail = output::bool_indicator(r.passed);
        if let Some(err) = &r.error {
            detail = format!("{detail} {}", err.red());
        } else if let (Some(actual), Some(expected)) = (&r.actual_value, &r.expected_value) {
            detail = format!(
                "{detail} {} (expected {})",
                scalar(actual),
                scalar(expected)
            );
        }
        output::field(&output::truncate(&name, 14), &detail);
    }
}

/// A failed execution comes back as HTTP 400 carrying `{result, message}`; the
/// client keeps only the generic `message`, so point at where the reason is.
fn explain_failed_execution(e: RmsError, id: &str, robot_id: &str) -> RmsError {
    match e {
        RmsError::Api {
            status: 400,
            message,
        } => RmsError::Api {
            status: 400,
            message: format!(
                "{message} — the server reports the cause only inside the 400 body. \
                 Re-run with --json to read it, or check preconditions with \
                 `rmsctl skills validate {id}` and `rmsctl skills check {id} {robot_id}`"
            ),
        },
        other => other,
    }
}

// ── abort ───────────────────────────────────────────────────────────────

/// POST `/skills/{id}/abort` needs both the skill (path) and the robot (body),
/// but the subcommand takes a single argument — so the robot is appended with
/// `@`, the way `host@port` is written.
async fn abort(client: &ApiClient, ctx: &Ctx, arg: &str) -> RmsResult<()> {
    let (id, robot_id) = match arg.split_once('@') {
        Some((id, robot)) => (require_skill_id(id)?, require_robot_id(robot)?),
        None => {
            let id = require_skill_id(arg).unwrap_or(arg.trim());
            return Err(RmsError::InvalidInput(format!(
                "`skills abort` needs the robot too — the server requires robotId in the \
                 body. Write it as `rmsctl skills abort {id}@<robot-id>` \
                 (`rmsctl robots list` shows robot ids)."
            )));
        }
    };

    let skill = fetch_skill(client, id).await?;

    let robot_name = client
        .get::<RobotDetail>(&format!("/robots/{robot_id}"))
        .await
        .ok()
        .and_then(|r| r.name)
        .unwrap_or_else(|| robot_id.to_string());

    if !output::confirm(
        &format!(
            "This will ABORT skill {} on robot {robot_name} ({robot_id}) — \
             the running motion stops mid-trajectory",
            skill.label()
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let path = format!("/skills/{id}/abort");
    let body = serde_json::json!({ "robotId": robot_id });

    // A successful abort is 204 with no body, which arrives here as `null`.
    match client.post_value(&path, &body).await {
        Ok(v) => {
            if ctx.json {
                return output::json_out(&v);
            }
            output::success(&format!("Abort sent for {} on {robot_id}", skill.label()));
            println!();
            Ok(())
        }
        // 404 is the server's way of saying the robot had nothing running.
        Err(RmsError::Api { status: 404, .. }) => {
            if ctx.json {
                return output::json_out(&serde_json::json!({
                    "skillId": id,
                    "robotId": robot_id,
                    "aborted": false,
                    "reason": "No active execution to abort",
                }));
            }
            output::info(&format!(
                "Nothing to abort: {robot_id} is not running {}.",
                skill.label()
            ));
            println!();
            Ok(())
        }
        Err(e) => Err(e),
    }
}
