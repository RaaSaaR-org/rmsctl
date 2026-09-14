use crate::cli::EvalCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};

pub async fn run(cmd: &EvalCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        EvalCommand::Episodes {
            robot_id,
            model_version,
        } => episodes(client, ctx, robot_id.as_deref(), model_version.as_deref()).await,
        EvalCommand::SuccessRate {
            robot_id,
            model_version,
            period,
        } => {
            success_rate(
                client,
                ctx,
                robot_id.as_deref(),
                model_version.as_deref(),
                period.as_deref(),
            )
            .await
        }
        EvalCommand::Episode { id } => episode(client, ctx, id).await,
        EvalCommand::Errors {
            robot_id,
            model_version,
        } => errors(client, ctx, robot_id.as_deref(), model_version.as_deref()).await,
        EvalCommand::Rewards => rewards(client, ctx).await,
        EvalCommand::Compare {
            version_a,
            version_b,
            period,
        } => compare(client, ctx, version_a, version_b, period.as_deref()).await,
        EvalCommand::RunHardware {
            robot_id,
            skill_id,
            episodes,
            prompt,
        } => {
            run_hardware(
                client,
                ctx,
                robot_id,
                skill_id,
                *episodes,
                prompt.as_deref(),
            )
            .await
        }
    }
}

// ── Response shapes ─────────────────────────────────────────────────────

/// The joined `robot: { id, name, model }` every episode row carries.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EpisodeRobot {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Episode {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    /// Free string on the row; `model_version_id` is the (nullable) FK.
    #[serde(default)]
    model_version: Option<String>,
    #[serde(default)]
    model_version_id: Option<String>,
    #[serde(default)]
    task_prompt: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    ended_at: Option<String>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    error_type: Option<String>,
    #[serde(default)]
    video_url: Option<String>,
    /// Stored as a JSON *string* column, so it arrives quoted more often than
    /// as an object. Keep it untyped and decode on display.
    #[serde(default)]
    metadata: Option<serde_json::Value>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    robot: Option<EpisodeRobot>,
}

/// Pagination arrives as FLAT SIBLING KEYS on this route — there is no nested
/// `pagination` object here, unlike /robots and /datasets.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EpisodesResponse {
    #[serde(default)]
    episodes: Vec<Episode>,
    #[serde(default)]
    total: Option<u32>,
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    total_pages: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
struct EpisodeDetailResponse {
    #[serde(default)]
    episode: Option<Episode>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SuccessRateResult {
    /// Already a percentage (0-100), not a fraction.
    #[serde(default)]
    success_rate: Option<f64>,
    #[serde(default)]
    total_episodes: Option<u32>,
    /// The server calls the numerator `successfulEpisodes`; there is no
    /// `successCount` key on this route.
    #[serde(default)]
    successful_episodes: Option<u32>,
    #[serde(default)]
    period: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBreakdownItem {
    #[serde(default)]
    error_type: Option<String>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    percentage: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ErrorBreakdownResponse {
    #[serde(default)]
    errors: Vec<ErrorBreakdownItem>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelStats {
    #[serde(default)]
    model_version: Option<String>,
    #[serde(default)]
    success_rate: Option<f64>,
    #[serde(default)]
    total_episodes: Option<u32>,
    #[serde(default)]
    avg_duration_ms: Option<u64>,
    #[serde(default)]
    error_breakdown: Vec<ErrorBreakdownItem>,
}

/// `versionA`/`versionB` are nested stats OBJECTS, and the server emits no
/// `improvement` field — the delta is computed here.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Comparison {
    #[serde(default)]
    version_a: ModelStats,
    #[serde(default)]
    version_b: ModelStats,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Reward {
    #[serde(default)]
    episode_index: Option<u32>,
    #[serde(default)]
    reward_type: Option<String>,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    success: Option<bool>,
    /// Per-frame progress, already parsed out of its JSON string column.
    #[serde(default)]
    curve: Option<Vec<f64>>,
    #[serde(default)]
    fps: Option<f64>,
    #[serde(default)]
    job_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RewardsResponse {
    #[serde(default)]
    rewards: Vec<Reward>,
}

#[derive(Debug, Deserialize, Serialize)]
struct DatasetRow {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatasetsPagination {
    #[serde(default)]
    total: Option<u32>,
    #[serde(default)]
    total_pages: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
struct DatasetsResponse {
    #[serde(default)]
    datasets: Vec<DatasetRow>,
    #[serde(default)]
    pagination: Option<DatasetsPagination>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct HardwareEpisodeResult {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    steps: Option<u32>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct HardwareSummary {
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    skill_id: Option<String>,
    #[serde(default)]
    episodes: Option<u32>,
    #[serde(default)]
    success_count: Option<u32>,
    /// The agent reports this one as a FRACTION (0-1), unlike the percentage
    /// that /evaluation/success-rate returns.
    #[serde(default)]
    success_rate: Option<f64>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    results: Vec<HardwareEpisodeResult>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RunHardwareResponse {
    #[serde(default)]
    summary: Option<HardwareSummary>,
}

// ── Shared helpers ──────────────────────────────────────────────────────

const PERIODS: [&str; 4] = ["24h", "7d", "30d", "all"];

/// The server's period union is `24h | 7d | 30d` and its window switch has no
/// default arm, so an unrecognised value leaves the date filter unset and
/// widens the query to all time (verified live: `?period=all` -> HTTP 200 with
/// `"period":"all"`). Only forward values that have been checked, and name the
/// accepted set instead of letting a typo turn into a silent all-time answer.
fn validate_period(period: Option<&str>) -> RmsResult<Option<String>> {
    match period.map(str::trim).filter(|p| !p.is_empty()) {
        None => Ok(None),
        Some(p) if PERIODS.contains(&p) => Ok(Some(p.to_string())),
        Some(p) => Err(RmsError::InvalidInput(format!(
            "--period {p} is not a window the server understands; use one of: {}",
            PERIODS.join(", ")
        ))),
    }
}

/// What window the numbers on screen actually cover.
///
/// Both routes apply a default window when `period` is absent (24h for
/// success-rate and error-breakdown, 7d for compare), so an unqualified rate is
/// never "the" success rate. Say which window produced it, every time.
fn window_label(requested: Option<&str>, server_echo: Option<&str>, default: &str) -> String {
    let effective = server_echo.or(requested).unwrap_or(default);
    let human = if effective == "all" {
        "all time".to_string()
    } else {
        format!("last {effective}")
    };
    if requested.is_none() {
        format!("{human} (server default)")
    } else {
        human
    }
}

fn filter_label(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("all")
        .to_string()
}

fn fmt_duration(ms: Option<u64>) -> String {
    match ms {
        None => "-".to_string(),
        Some(ms) if ms < 1000 => format!("{ms}ms"),
        Some(ms) if ms < 60_000 => format!("{:.1}s", ms as f64 / 1000.0),
        Some(ms) => {
            let secs = ms / 1000;
            format!("{}m {:02}s", secs / 60, secs % 60)
        }
    }
}

/// ISO-8601 from the server ("2026-09-13T22:01:24.323Z"), trimmed to seconds.
fn fmt_ts(ts: Option<&str>) -> String {
    match ts {
        None => "-".to_string(),
        Some(t) => t.chars().take(19).collect::<String>().replacen('T', " ", 1),
    }
}

fn outcome_cell(success: Option<bool>) -> String {
    match success {
        Some(true) => "pass".green().bold().to_string(),
        Some(false) => "fail".red().bold().to_string(),
        None => "-".bright_black().to_string(),
    }
}

fn rate_color(rate: f64) -> &'static str {
    if rate >= 80.0 {
        "green"
    } else if rate >= 50.0 {
        "yellow"
    } else {
        "red"
    }
}

/// A 12-cell share bar, for tables where `output::meter` would take the whole
/// line to itself.
fn share_bar(percentage: f64) -> String {
    let filled = ((percentage / 100.0) * 12.0).round().clamp(0.0, 12.0) as usize;
    format!(
        "{}{}",
        "█".repeat(filled).red(),
        "░".repeat(12 - filled).bright_black()
    )
}

fn render_error_rows(items: &[ErrorBreakdownItem]) {
    output::table_header(&[("ERROR TYPE", 28), ("COUNT", 7), ("SHARE", 20)]);
    for e in items {
        let pct = e.percentage.unwrap_or(0.0);
        println!(
            "    {} {} {} {:>5.1}%",
            output::pad(
                &output::truncate(e.error_type.as_deref().unwrap_or("unknown"), 28),
                28
            ),
            output::pad(&e.count.map(|c| c.to_string()).unwrap_or("-".into()), 7),
            share_bar(pct),
            pct
        );
    }
}

// ── episodes ────────────────────────────────────────────────────────────

async fn episodes(
    client: &ApiClient,
    ctx: &Ctx,
    robot_id: Option<&str>,
    model_version: Option<&str>,
) -> RmsResult<()> {
    let mut own: Vec<(&'static str, String)> = Vec::new();
    if let Some(v) = robot_id {
        own.push(("robotId", v.to_string()));
    }
    if let Some(v) = model_version {
        own.push(("modelVersion", v.to_string()));
    }
    // This route spells its page size `limit`.
    let query = ctx.with_paging_limit(own);

    if ctx.json {
        return output::json_out(
            &client
                .get_value_query("/evaluation/episodes", &query)
                .await?,
        );
    }

    let resp: EpisodesResponse = client.get_query("/evaluation/episodes", &query).await?;

    output::section("◈", "Evaluation Episodes");
    output::field("Robot", &filter_label(robot_id));
    output::field("Model", &filter_label(model_version));
    // Unlike success-rate and compare, this route applies no date filter of its
    // own, so the rows below are all-time and will not match a 24h rate.
    output::field("Window", "all time (this route applies no window)");

    if resp.episodes.is_empty() {
        println!();
        output::info("No evaluation episodes recorded for this filter.");
        println!();
        return Ok(());
    }

    println!();
    // The full episode ID is what makes `eval episode <id>` usable; the task
    // prompt is shown in full there.
    output::table_header(&[
        ("ID", output::ID_W),
        ("STARTED", 19),
        ("ROBOT", 14),
        ("MODEL", 12),
        ("TASK", 16),
        ("RESULT", 6),
        ("DURATION", 8),
    ]);
    for ep in &resp.episodes {
        let robot = ep
            .robot
            .as_ref()
            .and_then(|r| r.name.as_deref())
            .or(ep.robot_id.as_deref())
            .unwrap_or("-");
        println!(
            "    {} {} {} {} {} {} {}",
            output::pad(ep.id.as_deref().unwrap_or("-"), output::ID_W),
            output::pad(&fmt_ts(ep.started_at.as_deref()), 19),
            // pad() leaves an over-long value alone, so every column that can
            // hold server text has to truncate before padding or the row
            // shifts — robot IDs are 36-char UUIDs by default.
            output::pad(&output::truncate(robot, 14), 14),
            output::pad(
                &output::truncate(ep.model_version.as_deref().unwrap_or("-"), 12),
                12
            ),
            output::pad(
                &output::truncate(ep.task_prompt.as_deref().unwrap_or("-"), 16),
                16
            ),
            output::pad(&outcome_cell(ep.success), 6),
            fmt_duration(ep.duration_ms).bright_black()
        );
    }

    output::count_paged(
        "episode(s)",
        resp.episodes.len(),
        resp.total,
        resp.total_pages,
    );
    println!();
    Ok(())
}

// ── episode (detail) ────────────────────────────────────────────────────

async fn episode(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    if id.trim().is_empty() {
        return Err(RmsError::InvalidInput(
            "an episode ID is required: eval episode <id> (IDs come from `eval episodes`)".into(),
        ));
    }
    let path = format!("/evaluation/episodes/{id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp: EpisodeDetailResponse = client.get(&path).await?;
    let Some(ep) = resp.episode else {
        output::section("◈", "Evaluation Episode");
        output::warn("The server answered without an episode object.");
        println!();
        return Ok(());
    };

    output::section("◈", &format!("Episode {}", id.bold()));

    let robot = match ep.robot.as_ref() {
        Some(r) => match (
            r.name.as_deref(),
            r.id.as_deref().or(ep.robot_id.as_deref()),
        ) {
            (Some(name), Some(rid)) => format!("{name} ({rid})"),
            (Some(name), None) => name.to_string(),
            (None, Some(rid)) => rid.to_string(),
            (None, None) => "-".to_string(),
        },
        None => ep.robot_id.as_deref().unwrap_or("-").to_string(),
    };
    output::field("Robot", &robot);
    if let Some(model) = ep.robot.as_ref().and_then(|r| r.model.as_deref()) {
        output::field("Hardware", model);
    }
    output::field("Model", ep.model_version.as_deref().unwrap_or("-"));
    if let Some(mv) = &ep.model_version_id {
        output::field("Model Version", mv);
    }
    output::field("Task", ep.task_prompt.as_deref().unwrap_or("-"));
    output::field_colored(
        "Result",
        match ep.success {
            Some(true) => "pass".green().bold(),
            Some(false) => "fail".red().bold(),
            None => "-".bright_black(),
        },
    );
    if let Some(err) = &ep.error_type {
        output::field_colored("Error", err.red());
    }
    output::field("Duration", &fmt_duration(ep.duration_ms));
    output::field("Started", &fmt_ts(ep.started_at.as_deref()));
    output::field("Ended", &fmt_ts(ep.ended_at.as_deref()));
    if let Some(created) = &ep.created_at {
        output::field("Recorded", &fmt_ts(Some(created)));
    }
    if let Some(url) = &ep.video_url {
        output::field("Video", url);
    }

    render_metadata(ep.metadata.as_ref());

    println!();
    Ok(())
}

/// `metadata` is a JSON string column, so it normally arrives as a quoted
/// string rather than an object. Decode one level so the agent's own keys
/// (steps, episodeIndex, skillId, rtc) are readable.
fn render_metadata(metadata: Option<&serde_json::Value>) {
    let Some(raw) = metadata else { return };

    let decoded = match raw {
        serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(s).ok(),
        other => Some(other.clone()),
    };

    match decoded {
        Some(serde_json::Value::Object(map)) if !map.is_empty() => {
            output::subsection("Metadata");
            for (k, v) in &map {
                let rendered = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                output::field(&output::truncate(k, 14), &output::clip(&rendered, 200));
            }
        }
        Some(serde_json::Value::Object(_)) | None => {
            // An unparseable or empty metadata string: show it raw rather than
            // claiming there was nothing there.
            if let serde_json::Value::String(s) = raw
                && !s.trim().is_empty()
                && s.trim() != "{}"
            {
                output::subsection("Metadata");
                output::field("raw", &output::clip(s, 200));
            }
        }
        Some(other) => {
            output::subsection("Metadata");
            output::field("value", &output::clip(&other.to_string(), 200));
        }
    }
}

// ── success-rate ────────────────────────────────────────────────────────

async fn success_rate(
    client: &ApiClient,
    ctx: &Ctx,
    robot_id: Option<&str>,
    model_version: Option<&str>,
    period: Option<&str>,
) -> RmsResult<()> {
    let period = validate_period(period)?;

    let mut query: Vec<(&'static str, String)> = Vec::new();
    if let Some(v) = robot_id {
        query.push(("robotId", v.to_string()));
    }
    if let Some(v) = model_version {
        query.push(("modelVersion", v.to_string()));
    }
    if let Some(p) = &period {
        query.push(("period", p.clone()));
    }

    if ctx.json {
        return output::json_out(
            &client
                .get_value_query("/evaluation/success-rate", &query)
                .await?,
        );
    }

    let resp: SuccessRateResult = client.get_query("/evaluation/success-rate", &query).await?;

    output::section("◈", "Evaluation Success Rate");

    // The response echoes no robotId/modelVersion, so state the filters the
    // user supplied — otherwise two runs with different filters look identical.
    output::field("Robot", &filter_label(robot_id));
    output::field("Model", &filter_label(model_version));
    output::field(
        "Period",
        &window_label(period.as_deref(), resp.period.as_deref(), "24h"),
    );

    let total = resp.total_episodes.unwrap_or(0);
    let successes = resp.successful_episodes.unwrap_or(0);
    output::field("Episodes", &total.to_string());

    // With no episodes the server returns a literal successRate of 0, which is
    // indistinguishable from a total-failure run. Never draw the red meter for
    // an empty window.
    if total == 0 {
        println!();
        output::info("No episodes recorded in this window.");
        if period.as_deref() != Some("all") {
            output::info("Widen it with --period 30d, or --period all for every episode.");
        }
        println!();
        return Ok(());
    }

    output::field("Successes", &successes.to_string());
    output::field("Failures", &total.saturating_sub(successes).to_string());

    // Already a percentage server-side, so the meter's max is 100.
    let rate = resp
        .success_rate
        .unwrap_or_else(|| successes as f64 / total as f64 * 100.0);
    output::meter("Success Rate", rate, 100.0, rate_color(rate));

    println!();
    Ok(())
}

// ── error-breakdown ─────────────────────────────────────────────────────

async fn errors(
    client: &ApiClient,
    ctx: &Ctx,
    robot_id: Option<&str>,
    model_version: Option<&str>,
) -> RmsResult<()> {
    let mut query: Vec<(&'static str, String)> = Vec::new();
    if let Some(v) = robot_id {
        query.push(("robotId", v.to_string()));
    }
    if let Some(v) = model_version {
        query.push(("modelVersion", v.to_string()));
    }

    if ctx.json {
        return output::json_out(
            &client
                .get_value_query("/evaluation/error-breakdown", &query)
                .await?,
        );
    }

    let resp: ErrorBreakdownResponse = client
        .get_query("/evaluation/error-breakdown", &query)
        .await?;

    output::section("◈", "Evaluation Error Breakdown");
    output::field("Robot", &filter_label(robot_id));
    output::field("Model", &filter_label(model_version));
    // This subcommand takes no --period, and the route applies 24h when none is
    // sent, so the window is fixed at the server default here.
    output::field("Period", &window_label(None, None, "24h"));

    if resp.errors.is_empty() {
        println!();
        output::info("No failed episodes in this window — nothing to break down.");
        println!();
        return Ok(());
    }

    let failures: u32 = resp.errors.iter().filter_map(|e| e.count).sum();
    output::field("Failures", &failures.to_string());

    println!();
    render_error_rows(&resp.errors);

    // Shares are of failures only, not of all episodes: the server filters on
    // success = false before aggregating.
    output::count_paged("error type(s)", resp.errors.len(), None, None);
    output::info("Shares are of failed episodes only.");
    println!();
    Ok(())
}

// ── compare ─────────────────────────────────────────────────────────────

async fn compare(
    client: &ApiClient,
    ctx: &Ctx,
    version_a: &str,
    version_b: &str,
    period: Option<&str>,
) -> RmsResult<()> {
    if version_a.trim().is_empty() || version_b.trim().is_empty() {
        return Err(RmsError::InvalidInput(
            "two model versions are required: eval compare <version-a> <version-b>".into(),
        ));
    }
    let period = validate_period(period)?;

    let mut query: Vec<(&'static str, String)> = vec![
        ("versionA", version_a.to_string()),
        ("versionB", version_b.to_string()),
    ];
    if let Some(p) = &period {
        query.push(("period", p.clone()));
    }

    if ctx.json {
        return output::json_out(
            &client
                .get_value_query("/evaluation/compare", &query)
                .await?,
        );
    }

    let resp: Comparison = client.get_query("/evaluation/compare", &query).await?;
    let a = &resp.version_a;
    let b = &resp.version_b;

    output::section("◈", "Model Comparison");
    // The compare route never echoes the period back, so the label comes from
    // the flag plus the server's documented 7d default.
    output::field("Period", &window_label(period.as_deref(), None, "7d"));

    let name_a = output::truncate(a.model_version.as_deref().unwrap_or(version_a), 18);
    let name_b = output::truncate(b.model_version.as_deref().unwrap_or(version_b), 18);

    println!();
    output::table_header(&[
        ("METRIC", 16),
        (name_a.as_str(), 18),
        (name_b.as_str(), 18),
        ("DELTA", 18),
    ]);
    compare_row(
        "Episodes",
        &a.total_episodes.unwrap_or(0).to_string(),
        &b.total_episodes.unwrap_or(0).to_string(),
        &delta_cell(
            a.total_episodes.map(f64::from),
            b.total_episodes.map(f64::from),
            "",
            true,
        ),
    );
    compare_row(
        "Success rate",
        &fmt_rate(a.success_rate, a.total_episodes),
        &fmt_rate(b.success_rate, b.total_episodes),
        &delta_cell(
            a.success_rate.filter(|_| has_episodes(a)),
            b.success_rate.filter(|_| has_episodes(b)),
            " pts",
            true,
        ),
    );
    compare_row(
        "Avg duration",
        &avg_duration(a),
        &avg_duration(b),
        // Lower is better for duration, so the sign colouring flips.
        &delta_cell(
            a.avg_duration_ms
                .filter(|_| has_episodes(a))
                .map(|v| v as f64 / 1000.0),
            b.avg_duration_ms
                .filter(|_| has_episodes(b))
                .map(|v| v as f64 / 1000.0),
            "s",
            false,
        ),
    );

    for (stats, fallback) in [(a, version_a), (b, version_b)] {
        let name = stats.model_version.as_deref().unwrap_or(fallback);
        output::subsection(name);
        if !has_episodes(stats) {
            output::info("No episodes in this window.");
            continue;
        }
        let rate = stats.success_rate.unwrap_or(0.0);
        output::meter("Success", rate, 100.0, rate_color(rate));
        if !stats.error_breakdown.is_empty() {
            render_error_rows(&stats.error_breakdown);
        }
    }

    println!();
    match (
        a.success_rate.filter(|_| has_episodes(a)),
        b.success_rate.filter(|_| has_episodes(b)),
    ) {
        (Some(ra), Some(rb)) => {
            let diff = rb - ra;
            let (label, color) = if diff > 0.0 {
                (format!("+{diff:.1} pts"), "green")
            } else if diff < 0.0 {
                (format!("{diff:.1} pts"), "red")
            } else {
                ("0.0 pts".to_string(), "dim")
            };
            output::stat_line("◈", "Delta (B - A)", &label, color);
            let verdict = if diff > 0.0 {
                format!("{name_b} is ahead of {name_a} in this window.")
            } else if diff < 0.0 {
                format!("{name_a} is ahead of {name_b} in this window.")
            } else {
                "Both versions scored the same in this window.".to_string()
            };
            output::info(&verdict);
        }
        _ => output::warn("Not comparable: at least one version has no episodes in this window."),
    }

    println!();
    Ok(())
}

fn compare_row(metric: &str, a: &str, b: &str, delta: &str) {
    println!(
        "    {} {} {} {}",
        output::pad(metric, 16),
        output::pad(a, 18),
        output::pad(b, 18),
        delta
    );
}

fn has_episodes(stats: &ModelStats) -> bool {
    stats.total_episodes.unwrap_or(0) > 0
}

/// A 0 % rate over 0 episodes is "no data", not a failure — show a dash.
fn fmt_rate(rate: Option<f64>, total: Option<u32>) -> String {
    match (rate, total.unwrap_or(0)) {
        (_, 0) => "-".bright_black().to_string(),
        (Some(r), _) => format!("{r:.1}%"),
        (None, _) => "-".bright_black().to_string(),
    }
}

fn avg_duration(stats: &ModelStats) -> String {
    if !has_episodes(stats) {
        return "-".bright_black().to_string();
    }
    fmt_duration(stats.avg_duration_ms)
}

fn delta_cell(a: Option<f64>, b: Option<f64>, unit: &str, higher_is_better: bool) -> String {
    let (Some(a), Some(b)) = (a, b) else {
        return "-".bright_black().to_string();
    };
    let diff = b - a;
    if diff == 0.0 {
        let text = if unit.is_empty() {
            "0".to_string()
        } else {
            format!("0.0{unit}")
        };
        return text.bright_black().to_string();
    }
    let text = if unit.is_empty() {
        format!("{diff:+.0}")
    } else {
        format!("{diff:+.1}{unit}")
    };
    if (diff > 0.0) == higher_is_better {
        text.green().bold().to_string()
    } else {
        text.red().bold().to_string()
    }
}

// ── rewards ─────────────────────────────────────────────────────────────

async fn rewards(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    // /evaluation/rewards is keyed on a dataset and answers 400 without
    // ?datasetId, while `eval rewards` takes no dataset argument — so walk one
    // page of datasets and ask for each. --page/--limit bound the fan-out.
    let list: DatasetsResponse = client.get_query("/datasets", &ctx.paging_limit()).await?;
    let scanned: Vec<(String, String)> = list
        .datasets
        .iter()
        .filter_map(|d| {
            d.id.clone()
                .map(|id| (id.clone(), d.name.clone().unwrap_or(id)))
        })
        .collect();

    if ctx.json {
        // A composite of per-dataset payloads: each value is the server's own
        // body, never a re-serialised struct. A dataset that fails is reported
        // in place rather than sinking the whole scan.
        let mut by_dataset = serde_json::Map::new();
        for (id, _) in &scanned {
            let value = match client
                .get_value_query("/evaluation/rewards", &[("datasetId", id.as_str())])
                .await
            {
                Ok(v) => v,
                Err(e) => serde_json::json!({ "error": e.to_string() }),
            };
            by_dataset.insert(id.clone(), value);
        }
        return output::json_out(&serde_json::json!({
            "scannedDatasets": scanned.len(),
            "rewardsByDataset": serde_json::Value::Object(by_dataset),
        }));
    }

    output::section("◈", "Reward-Model Scores");
    output::field("Datasets", &scanned.len().to_string());
    output::field(
        "Source",
        "one /evaluation/rewards call per dataset (--page / --limit)",
    );

    if scanned.is_empty() {
        println!();
        output::info("No datasets to scan for reward-model scores.");
        println!();
        return Ok(());
    }

    let mut rows = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for (id, name) in &scanned {
        let resp: RewardsResponse = match client
            .get_query("/evaluation/rewards", &[("datasetId", id.as_str())])
            .await
        {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        if resp.rewards.is_empty() {
            continue;
        }

        output::subsection(&format!(
            "{} {}",
            output::truncate(name, 40),
            id.bright_black()
        ));
        output::table_header(&[
            ("EPISODE", 8),
            ("TYPE", 12),
            ("SCORE", 8),
            ("OK", 3),
            ("FRAMES", 7),
            ("FPS", 6),
            ("JOB", output::ID_W),
        ]);
        for r in &resp.rewards {
            println!(
                "    {} {} {} {} {} {} {}",
                output::pad(
                    &r.episode_index.map(|i| i.to_string()).unwrap_or("-".into()),
                    8
                ),
                output::pad(
                    &output::truncate(r.reward_type.as_deref().unwrap_or("-"), 12),
                    12
                ),
                output::pad(&r.score.map(|s| format!("{s:.3}")).unwrap_or("-".into()), 8),
                output::pad(&output::bool_indicator(r.success), 3),
                output::pad(
                    &r.curve
                        .as_ref()
                        .map(|c| c.len().to_string())
                        .unwrap_or("-".into()),
                    7
                ),
                output::pad(&r.fps.map(|f| format!("{f:.0}")).unwrap_or("-".into()), 6),
                output::pad(r.job_id.as_deref().unwrap_or("-"), output::ID_W)
            );
            rows += 1;
        }
    }

    if rows == 0 {
        println!();
        output::info(&format!(
            "No reward-model scores recorded in the {} dataset(s) scanned.",
            scanned.len()
        ));
        output::info("Scores appear once a reward-model evaluation job completes.");
    } else {
        output::count_paged("reward score(s)", rows, None, None);
    }

    for f in &failures {
        output::warn(f);
    }
    // The scan only covers the datasets page that came back, so say when there
    // are more of them than were asked about.
    if let Some(total) = list.pagination.as_ref().and_then(|p| p.total)
        && total as usize > scanned.len()
    {
        output::info(&format!(
            "Scanned {} of {total} dataset(s) — use --page N or --limit {total} for the rest.",
            scanned.len(),
        ));
    }

    println!();
    Ok(())
}

// ── run-hardware ────────────────────────────────────────────────────────

async fn run_hardware(
    client: &ApiClient,
    ctx: &Ctx,
    robot_id: &str,
    skill_id: &str,
    episodes: Option<u32>,
    prompt: Option<&str>,
) -> RmsResult<()> {
    if robot_id.trim().is_empty() {
        return Err(RmsError::InvalidInput(
            "--robot-id is required: the server rejects a hardware run without one".into(),
        ));
    }
    if skill_id.trim().is_empty() {
        return Err(RmsError::InvalidInput(
            "--skill-id is required: the server rejects a hardware run without one".into(),
        ));
    }
    if episodes == Some(0) {
        return Err(RmsError::InvalidInput(
            "--episodes must be at least 1".into(),
        ));
    }
    // The agent clamps the count into 1..=50 before it starts executing.
    if episodes.unwrap_or(0) > 50 {
        output::warn("The robot agent caps a run at 50 episodes; the extra ones will not run.");
    }

    let planned = match episodes {
        Some(n) => format!("{n} episode(s)"),
        None => "5 episode(s) (server default)".to_string(),
    };

    // This drives a physical robot through a closed-loop skill, once per
    // episode, and cannot be called back once the agent starts.
    if !output::confirm(
        &format!("This will RUN skill {skill_id} on robot {robot_id} for {planned} of live motion"),
        ctx.yes,
    ) {
        return Ok(());
    }

    let mut body = serde_json::json!({ "robotId": robot_id, "skillId": skill_id });
    if let Some(n) = episodes {
        body["episodes"] = serde_json::json!(n);
    }
    if let Some(p) = prompt.map(str::trim).filter(|p| !p.is_empty()) {
        body["taskPrompt"] = serde_json::json!(p);
    }

    let resp = client.post_value("/evaluation/run-hardware", &body).await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    let parsed: RunHardwareResponse =
        serde_json::from_value(resp.clone()).unwrap_or(RunHardwareResponse { summary: None });

    output::section("◈", "Hardware Evaluation Run");

    let Some(summary) = parsed.summary else {
        output::success(&format!("Evaluation run accepted for robot {robot_id}"));
        output::warn("The server returned no run summary; re-run with --json for the raw body.");
        println!();
        return Ok(());
    };

    output::field("Robot", summary.robot_id.as_deref().unwrap_or(robot_id));
    output::field("Skill", summary.skill_id.as_deref().unwrap_or(skill_id));
    output::field("Started", &fmt_ts(summary.started_at.as_deref()));

    let ran = summary.episodes.unwrap_or(summary.results.len() as u32);
    output::field("Episodes run", &ran.to_string());
    if let Some(ok) = summary.success_count {
        output::field("Successes", &ok.to_string());
    }
    // The agent reports successRate as a fraction; the rest of this command
    // speaks percent.
    if let Some(rate) = summary.success_rate.filter(|_| ran > 0) {
        let pct = rate * 100.0;
        output::meter("Success Rate", pct, 100.0, rate_color(pct));
    }

    if !summary.results.is_empty() {
        println!();
        output::table_header(&[
            ("EPISODE", 8),
            ("STATUS", 16),
            ("STEPS", 7),
            ("DURATION", 10),
            ("ERROR", 30),
        ]);
        for r in &summary.results {
            println!(
                "    {} {} {} {} {}",
                output::pad(&r.index.map(|i| i.to_string()).unwrap_or("-".into()), 8),
                output::pad(
                    &output::status_indicator(r.status.as_deref().unwrap_or("-")),
                    16
                ),
                output::pad(&r.steps.map(|s| s.to_string()).unwrap_or("-".into()), 7),
                output::pad(&fmt_duration(r.duration_ms), 10),
                output::truncate(r.error.as_deref().unwrap_or("-"), 30)
            );
        }
    }

    println!();
    // Per-episode rows are POSTed to the server by the agent itself, so the
    // durable record lives under `eval episodes`, not in this response.
    output::success("Run finished — episodes recorded server-side.");
    output::info(&format!(
        "See them with: rmsctl eval episodes --robot-id {robot_id}"
    ));
    println!();
    Ok(())
}
