use crate::cli::TeleopCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::models::Pagination;
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};

pub async fn run(cmd: &TeleopCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        TeleopCommand::List {
            robot_id,
            status,
            session_type,
        } => {
            list(
                client,
                ctx,
                robot_id.as_deref(),
                status.as_deref(),
                session_type.as_deref(),
            )
            .await
        }
        TeleopCommand::Show { id } => show(client, ctx, id).await,
        TeleopCommand::Episodes { id } => episodes(client, ctx, id).await,
        TeleopCommand::Pause { id } => transition(client, ctx, id, Transition::Pause).await,
        TeleopCommand::Resume { id } => transition(client, ctx, id, Transition::Resume).await,
        TeleopCommand::End { id } => transition(client, ctx, id, Transition::End).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    id: String,
    #[serde(default)]
    operator_id: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    /// `vr_quest`, `vr_vision_pro`, `bilateral_aloha`, `kinesthetic`,
    /// `keyboard_mouse` or `gamepad`.
    #[serde(default, rename = "type")]
    session_type: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    ended_at: Option<String>,
    #[serde(default)]
    frame_count: Option<u64>,
    /// Seconds, filled in when the session ends.
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    fps: Option<f64>,
    #[serde(default)]
    language_instr: Option<String>,
    #[serde(default)]
    quality_score: Option<f64>,
    #[serde(default)]
    exported_dataset_id: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
    /// The *target* episode count, not how many were recorded — a session set
    /// up for three takes reports 3 while `episodes` still lists none.
    #[serde(default)]
    num_episodes: Option<u32>,
    #[serde(default)]
    dataset_repo_id: Option<String>,
    /// Which recorder held the frames: `agent` (on the robot), `sim` (rows on
    /// the server) or `sidecar` (lerobot-record on the hardware).
    #[serde(default)]
    recorder_kind: Option<String>,
    #[serde(default)]
    agent_dataset_path: Option<String>,
    #[serde(default)]
    sidecar_dataset_path: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

impl Session {
    fn status_str(&self) -> &str {
        self.status.as_deref().unwrap_or("unknown")
    }

    fn operator(&self) -> &str {
        self.operator_id.as_deref().unwrap_or("an unknown operator")
    }

    fn robot(&self) -> &str {
        self.robot_id.as_deref().unwrap_or("an unknown robot")
    }
}

/// `GET /teleoperation/sessions` — rows under `sessions`, paged with
/// `page` + `limit`.
#[derive(Debug, Deserialize, Serialize)]
struct SessionsResponse {
    #[serde(default)]
    sessions: Vec<Session>,
    #[serde(default)]
    pagination: Option<Pagination>,
}

/// `GET /teleoperation/sessions/{id}` and every mutating route wrap the single
/// session in `session` — a different envelope from the list above.
#[derive(Debug, Deserialize, Serialize)]
struct SessionResponse {
    #[serde(default)]
    session: Option<Session>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Episode {
    #[serde(default)]
    episode_index: Option<u32>,
    #[serde(default)]
    frame_count: Option<u64>,
    /// Seconds from the start of the session.
    #[serde(default)]
    start_time: Option<f64>,
    #[serde(default)]
    end_time: Option<f64>,
    #[serde(default)]
    duration_s: Option<f64>,
    /// Absent means the recorder of the day could not count drops, which is
    /// not the same as zero drops.
    #[serde(default)]
    dropped_frames: Option<u64>,
    #[serde(default)]
    fps_actual: Option<f64>,
    /// How the joint targets were produced (`ik`, `orientation`,
    /// `hand-tracking`, `manual`). Absent — not empty — on older episodes.
    #[serde(default)]
    retarget_modes: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EpisodesResponse {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    episodes: Vec<Episode>,
}

// ── Formatting ──────────────────────────────────────────────────────────

const TYPES: &[&str] = &[
    "vr_quest",
    "vr_vision_pro",
    "bilateral_aloha",
    "kinesthetic",
    "keyboard_mouse",
    "gamepad",
];
const STATUSES: &[&str] = &["created", "recording", "paused", "completed", "failed"];

/// `output::status_indicator` has no `recording` in its vocabulary, so it would
/// print the one state an operator must not miss — somebody driving a robot
/// right now — as unstyled text. Everything else defers to it.
fn session_status(status: &str) -> String {
    if status.eq_ignore_ascii_case("recording") {
        return format!(
            "{} {}",
            "◐".bright_black(),
            status.truecolor(255, 103, 0).bold()
        );
    }
    output::status_indicator(status)
}

fn is_live(s: &Session) -> bool {
    matches!(
        s.status_str().to_lowercase().as_str(),
        "recording" | "paused"
    )
}

fn fmt_secs(secs: f64) -> String {
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let mins = (secs / 60.0).floor();
        format!("{mins:.0}m {:.0}s", secs - mins * 60.0)
    }
}

/// Episode timings are seconds with real precision (3.95, 19.77) — rounding to
/// one decimal turns a 3.95s take into a 4.0s one.
fn fmt_num(v: Option<f64>) -> String {
    v.map(|n| format!("{n:.2}")).unwrap_or_else(|| "-".into())
}

/// `?type=` and `?status=` take comma-separated lists, and an unknown value is
/// not an error: the server matches nothing and answers 200 with an empty page.
/// A typo would therefore read as "no such sessions", so it is caught here.
fn filter_csv(raw: &str, allowed: &[&str], flag: &str) -> RmsResult<String> {
    let mut values = Vec::new();
    for part in raw.split(',') {
        let value = part.trim().to_lowercase();
        if value.is_empty() {
            continue;
        }
        if !allowed.contains(&value.as_str()) {
            return Err(RmsError::InvalidInput(format!(
                "unknown {flag} value '{}' — expected one of: {}",
                part.trim(),
                allowed.join(", ")
            )));
        }
        values.push(value);
    }
    if values.is_empty() {
        return Err(RmsError::InvalidInput(format!(
            "{flag} needs at least one value: {}",
            allowed.join(", ")
        )));
    }
    Ok(values.join(","))
}

// ── list ────────────────────────────────────────────────────────────────

async fn list(
    client: &ApiClient,
    ctx: &Ctx,
    robot_id: Option<&str>,
    status: Option<&str>,
    session_type: Option<&str>,
) -> RmsResult<()> {
    let mut filters: Vec<(&'static str, String)> = Vec::new();
    if let Some(r) = robot_id {
        filters.push(("robotId", r.to_string()));
    }
    if let Some(s) = status {
        filters.push(("status", filter_csv(s, STATUSES, "--status")?));
    }
    if let Some(t) = session_type {
        filters.push(("type", filter_csv(t, TYPES, "--type")?));
    }
    let query = ctx.with_paging_limit(filters);

    if ctx.json {
        return output::json_out(
            &client
                .get_value_query("/teleoperation/sessions", &query)
                .await?,
        );
    }

    let resp: SessionsResponse = client.get_query("/teleoperation/sessions", &query).await?;

    output::section("◈", "Teleoperation Sessions");

    if resp.sessions.is_empty() {
        output::info("No teleoperation sessions match.");
        println!();
        return Ok(());
    }

    // Counted over the rows on this page, not the envelope total: with --page
    // the server's total covers slices that are not on screen.
    let recording = resp
        .sessions
        .iter()
        .filter(|s| s.status_str().eq_ignore_ascii_case("recording"))
        .count();
    let paused = resp
        .sessions
        .iter()
        .filter(|s| s.status_str().eq_ignore_ascii_case("paused"))
        .count();

    output::stat_line("│", "Shown", &resp.sessions.len().to_string(), "orange");
    output::stat_line(
        if recording > 0 { "◐" } else { "│" },
        "Recording now",
        &recording.to_string(),
        if recording > 0 { "orange" } else { "dim" },
    );
    output::stat_line(
        "│",
        "Paused",
        &paused.to_string(),
        if paused > 0 { "yellow" } else { "dim" },
    );
    println!();

    output::table_header(&[
        ("STATUS", 13),
        ("ID", output::ID_W),
        ("ROBOT", 18),
        ("OPERATOR", 14),
        ("TYPE", 15),
        ("FRAMES", 7),
    ]);
    for s in &resp.sessions {
        println!(
            "    {} {} {} {} {} {}",
            output::pad(&session_status(s.status_str()), 13),
            output::pad(&s.id, output::ID_W),
            output::pad(
                &output::truncate(s.robot_id.as_deref().unwrap_or("-"), 18),
                18
            ),
            output::pad(
                &output::truncate(s.operator_id.as_deref().unwrap_or("-"), 14),
                14
            ),
            output::pad(
                &output::truncate(s.session_type.as_deref().unwrap_or("-"), 15),
                15
            ),
            s.frame_count
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".into())
        );
    }

    let pag = resp.pagination.as_ref();
    output::count_paged(
        "teleop session(s)",
        resp.sessions.len(),
        pag.and_then(|p| p.total()),
        pag.and_then(|p| p.total_pages()),
    );

    if recording + paused > 0 {
        output::info("A live session holds control of its robot — 'teleop end <id>' releases it.");
    }
    println!();
    Ok(())
}

// ── show ────────────────────────────────────────────────────────────────

async fn fetch_session(client: &ApiClient, id: &str) -> RmsResult<Session> {
    let resp: SessionResponse = client.get(&format!("/teleoperation/sessions/{id}")).await?;
    resp.session.ok_or_else(|| {
        RmsError::Other(format!(
            "the server answered 200 but sent no session for {id}"
        ))
    })
}

async fn show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(
            &client
                .get_value(&format!("/teleoperation/sessions/{id}"))
                .await?,
        );
    }

    let s = fetch_session(client, id).await?;

    output::section("◈", &format!("Teleop Session {}", s.id.bold()));
    render_session(&s);

    if is_live(&s) {
        println!();
        output::info(&format!(
            "Live: {} is driving {}. Use 'teleop pause {}' or 'teleop end {}'.",
            s.operator(),
            s.robot(),
            s.id,
            s.id
        ));
    }
    println!();
    Ok(())
}

fn render_session(s: &Session) {
    output::field("Status", &session_status(s.status_str()));
    output::field("Robot", s.robot_id.as_deref().unwrap_or("-"));
    output::field("Operator", s.operator_id.as_deref().unwrap_or("-"));
    output::field("Interface", s.session_type.as_deref().unwrap_or("-"));
    if let Some(task) = &s.language_instr {
        output::field("Task", task);
    }

    output::subsection("Recording");
    let frames = s
        .frame_count
        .map(|c| c.to_string())
        .unwrap_or_else(|| "-".into());
    match s.fps {
        Some(fps) => output::field("Frames", &format!("{frames} @ {fps:.0} fps target")),
        None => output::field("Frames", &frames),
    }
    if let Some(d) = s.duration {
        output::field("Duration", &fmt_secs(d));
    }
    if let Some(n) = s.num_episodes {
        output::field("Episodes", &format!("{n} target"));
    }
    if let Some(q) = s.quality_score {
        output::field("Quality", &format!("{q:.0}"));
    }
    if let Some(kind) = &s.recorder_kind {
        output::field("Recorder", kind);
    }
    if let Some(ds) = &s.exported_dataset_id {
        output::field("Dataset", ds);
    }
    if let Some(repo) = &s.dataset_repo_id {
        output::field("Repo", repo);
    }
    for p in [&s.agent_dataset_path, &s.sidecar_dataset_path]
        .into_iter()
        .flatten()
    {
        output::field("Data path", p);
    }
    // A completed session can still carry an error: the recording ended, but
    // produced nothing worth exporting.
    if let Some(err) = &s.error_message {
        output::field_colored("Error", err.yellow());
    }

    output::subsection("Timeline");
    output::field("Created", s.created_at.as_deref().unwrap_or("-"));
    output::field("Started", s.started_at.as_deref().unwrap_or("-"));
    output::field("Ended", s.ended_at.as_deref().unwrap_or("-"));
    output::field("Updated", s.updated_at.as_deref().unwrap_or("-"));
}

// ── episodes ────────────────────────────────────────────────────────────

async fn episodes(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let path = format!("/teleoperation/sessions/{id}/episodes");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let resp: EpisodesResponse = client.get(&path).await?;

    output::section(
        "◈",
        &format!(
            "Episodes: {}",
            resp.session_id.as_deref().unwrap_or(id).bold()
        ),
    );

    if resp.episodes.is_empty() {
        output::info("No episodes recorded — the session has no frames to slice into takes.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("EP", 4),
        ("FRAMES", 8),
        ("START", 8),
        ("END", 8),
        ("LEN", 8),
        ("DROPPED", 8),
        ("FPS", 7),
        ("RETARGET", 22),
    ]);
    for e in &resp.episodes {
        println!(
            "    {} {} {} {} {} {} {} {}",
            output::pad(
                &e.episode_index
                    .map(|i| i.to_string())
                    .unwrap_or_else(|| "-".into()),
                4
            ),
            output::pad(
                &e.frame_count
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".into()),
                8
            ),
            output::pad(&fmt_num(e.start_time), 8),
            output::pad(&fmt_num(e.end_time), 8),
            output::pad(&fmt_num(e.duration_s), 8),
            output::pad(
                // "-" means the recorder could not count drops, "0" means none.
                &e.dropped_frames
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "-".into()),
                8
            ),
            output::pad(&fmt_num(e.fps_actual), 7),
            output::truncate(
                &e.retarget_modes
                    .as_ref()
                    .map(|m| m.join(","))
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| "-".into()),
                22
            )
        );
    }

    let dropped: u64 = resp.episodes.iter().filter_map(|e| e.dropped_frames).sum();
    output::count_paged("episode(s)", resp.episodes.len(), None, None);
    if dropped > 0 {
        output::warn(&format!(
            "{dropped} frame(s) dropped — the recorder missed ticks the operator did send."
        ));
    }
    println!();
    Ok(())
}

// ── pause / resume / end ────────────────────────────────────────────────

#[derive(Copy, Clone)]
enum Transition {
    Pause,
    Resume,
    End,
}

impl Transition {
    fn slug(self) -> &'static str {
        match self {
            Transition::Pause => "pause",
            Transition::Resume => "resume",
            Transition::End => "end",
        }
    }

    /// TeleoperationService rejects every other status with a 400, so the
    /// current state is checked before anything is POSTed.
    fn allowed_from(self) -> &'static [&'static str] {
        match self {
            Transition::Pause => &["recording"],
            Transition::Resume => &["paused"],
            Transition::End => &["recording", "paused"],
        }
    }

    fn gate(self, s: &Session) -> String {
        match self {
            Transition::Pause => format!(
                "This will PAUSE session {} — {} keeps the console but recording of {} stops",
                s.id,
                s.operator(),
                s.robot()
            ),
            Transition::Resume => format!(
                "This will RESUME recording on session {} — {} driving {}",
                s.id,
                s.operator(),
                s.robot()
            ),
            Transition::End => format!(
                "This will END session {} — {} loses live control of {} and the recording is finalised",
                s.id,
                s.operator(),
                s.robot()
            ),
        }
    }

    fn done(self) -> &'static str {
        match self {
            Transition::Pause => "paused",
            Transition::Resume => "resumed",
            Transition::End => "ended",
        }
    }
}

async fn transition(client: &ApiClient, ctx: &Ctx, id: &str, t: Transition) -> RmsResult<()> {
    let current = fetch_session(client, id).await?;
    let status = current.status_str().to_lowercase();
    if !t.allowed_from().contains(&status.as_str()) {
        let expected = t
            .allowed_from()
            .iter()
            .map(|s| format!("'{s}'"))
            .collect::<Vec<_>>()
            .join(" or ");
        return Err(RmsError::InvalidInput(format!(
            "session {id} is '{status}' — '{}' needs it to be {expected}",
            t.slug()
        )));
    }

    if !output::confirm(&t.gate(&current), ctx.yes) {
        return Ok(());
    }

    // The three handlers read only :id; they take no body.
    let resp = client
        .post_value(
            &format!("/teleoperation/sessions/{id}/{}", t.slug()),
            &serde_json::json!({}),
        )
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    output::success(&format!("Session {id} {}", t.done()));
    if let Some(msg) = resp.get("message").and_then(|m| m.as_str()) {
        output::info(msg);
    }

    if let Some(s) = resp
        .get("session")
        .and_then(|v| serde_json::from_value::<Session>(v.clone()).ok())
    {
        output::field("Status", &session_status(s.status_str()));
        if let Some(c) = s.frame_count {
            output::field("Frames", &c.to_string());
        }
        if let Some(d) = s.duration {
            output::field("Duration", &fmt_secs(d));
        }
        // `end` is where the export happens: it either names the dataset it
        // wrote or says why there is none.
        if let Some(ds) = &s.exported_dataset_id {
            output::field("Dataset", ds);
        }
        if let Some(err) = &s.error_message {
            output::warn(err);
        }
    }
    println!();
    Ok(())
}
