use crate::cli::UpdatesCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};

pub async fn run(cmd: &UpdatesCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        UpdatesCommand::List => list(client, ctx).await,
        UpdatesCommand::Show { id } => show(client, ctx, id).await,
        UpdatesCommand::Deployments { robot_id } => deployments(client, ctx, robot_id).await,
        UpdatesCommand::Approve { id, approver_id } => approve(client, ctx, id, approver_id).await,
        UpdatesCommand::Deploy {
            id,
            robot_id,
            previous_version,
        } => deploy(client, ctx, id, robot_id, previous_version.as_deref()).await,
        UpdatesCommand::Rollback {
            id,
            robot_id,
            target_version,
        } => rollback(client, ctx, id, robot_id, target_version).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────
//
// Both GET lists are BARE ARRAYS — `/updates` and `/updates/deployments/:id`
// return the rows with no envelope and no pagination block.

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdatePackage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    changelog: Option<String>,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    public_key: Option<String>,
    #[serde(default)]
    checksum: Option<String>,
    #[serde(default)]
    file_size: Option<u64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    approved_by: Option<String>,
    #[serde(default)]
    approved_at: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

impl UpdatePackage {
    fn status(&self) -> &str {
        self.status.as_deref().unwrap_or("unknown")
    }

    fn version(&self) -> &str {
        self.version.as_deref().unwrap_or("unknown version")
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateDeployment {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    package_id: Option<String>,
    #[serde(default)]
    robot_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    previous_version: Option<String>,
    #[serde(default)]
    deployed_at: Option<String>,
    #[serde(default)]
    rolled_back_at: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

// ── Local formatting ────────────────────────────────────────────────────

/// `output::status_indicator` does not know the OTA vocabulary: `deployed` is
/// the good terminal state for a package and `downloading`/`installing` are
/// work in progress, all of which fall through to its colourless default.
fn update_status(status: &str) -> String {
    match status.to_lowercase().as_str() {
        "deployed" => format!("{} {}", "●".bright_black(), "deployed".green().bold()),
        "rolled_back" => format!("{} {}", "◐".bright_black(), "rolled_back".yellow().bold()),
        "downloading" | "installing" => format!(
            "{} {}",
            "◐".bright_black(),
            status.truecolor(255, 103, 0).bold()
        ),
        _ => output::status_indicator(status),
    }
}

fn human_size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KiB", b as f64 / 1024.0),
        b => format!("{:.1} MiB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// `output::field` is a single line; changelogs run to several sentences, so
/// wrap on word boundaries and blank the label on continuation lines.
fn field_wrapped(label: &str, text: &str, width: usize) {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    for (i, line) in lines.iter().enumerate() {
        output::field(if i == 0 { label } else { "" }, line);
    }
}

/// Neither GET route reads `page`/`pageSize` (`update.routes.ts` passes only
/// `status` through to Prisma), so the paging flags are applied to the array
/// here instead of being sent as query params the server would drop.
fn page_slice<'a, T>(rows: &'a [T], ctx: &Ctx) -> &'a [T] {
    let Some(limit) = ctx.limit.filter(|l| *l > 0).map(|l| l as usize) else {
        return rows;
    };
    let page = ctx.page.unwrap_or(1).max(1) as usize;
    let start = (page - 1) * limit;
    if start >= rows.len() {
        return &[];
    }
    &rows[start..(start + limit).min(rows.len())]
}

fn total_pages(total: usize, ctx: &Ctx) -> Option<u32> {
    let limit = ctx.limit.filter(|l| *l > 0)? as usize;
    Some(total.div_ceil(limit) as u32)
}

/// `--page` past the last page: say so rather than print a bare table header.
fn past_end(total: usize, ctx: &Ctx) -> RmsResult<()> {
    let pages = total_pages(total, ctx).unwrap_or(1);
    output::info(&format!(
        "Page {} is past the end — {total} row(s) in {pages} page(s) at this --limit.",
        ctx.page.unwrap_or(1)
    ));
    println!();
    Ok(())
}

// ── list ────────────────────────────────────────────────────────────────

async fn list(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value("/updates").await?);
    }

    let all: Vec<UpdatePackage> = client.get("/updates").await?;

    output::section("◈", "Update Packages");

    if all.is_empty() {
        output::info("No update packages on this server.");
        println!();
        return Ok(());
    }

    let rows = page_slice(&all, ctx);
    if rows.is_empty() {
        return past_end(all.len(), ctx);
    }

    output::table_header(&[
        ("ID", output::ID_W),
        ("VERSION", 9),
        ("STATUS", 15),
        ("SIZE", 9),
        ("CHECKSUM", 14),
        ("CHANGELOG", 32),
    ]);
    for p in rows {
        println!(
            "    {} {} {} {} {} {}",
            output::pad(p.id.as_deref().unwrap_or("-"), output::ID_W),
            output::pad(&output::truncate(p.version(), 9), 9),
            output::pad(&update_status(p.status()), 15),
            output::pad(
                &p.file_size.map(human_size).unwrap_or_else(|| "-".into()),
                9
            ),
            // The checksum, signature and public key are only useful in full;
            // 'show' prints them, the table just fingerprints them.
            output::pad(
                &output::truncate(p.checksum.as_deref().unwrap_or("-"), 14),
                14
            ),
            output::truncate(p.changelog.as_deref().unwrap_or("-"), 32)
        );
    }

    let pending = all.iter().filter(|p| p.status() == "pending").count();
    if pending > 0 {
        println!();
        output::info(&format!(
            "{pending} awaiting approval — rmsctl updates approve <id> --approver-id <you>"
        ));
    }

    output::count_paged(
        "update package(s)",
        rows.len(),
        Some(all.len() as u32),
        total_pages(all.len(), ctx),
    );
    println!();
    Ok(())
}

// ── show ────────────────────────────────────────────────────────────────

async fn fetch_package(client: &ApiClient, id: &str) -> RmsResult<UpdatePackage> {
    client.get(&format!("/updates/{id}")).await
}

async fn show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    if ctx.json {
        return output::json_out(&client.get_value(&format!("/updates/{id}")).await?);
    }

    let p = fetch_package(client, id).await?;

    output::section("◈", &format!("Update {}", p.version().bold()));
    output::field("ID", p.id.as_deref().unwrap_or(id));
    output::field("Status", &update_status(p.status()));
    if let Some(size) = p.file_size {
        output::field("Package size", &human_size(size));
    }
    if let Some(created) = &p.created_at {
        output::field("Created", created);
    }
    match (&p.approved_by, &p.approved_at) {
        (Some(by), Some(at)) => output::field("Approved", &format!("{by} at {at}")),
        (Some(by), None) => output::field("Approved by", by),
        _ => output::field_colored("Approved", "not approved".yellow()),
    }

    if let Some(changelog) = p.changelog.as_deref().filter(|c| !c.trim().is_empty()) {
        output::subsection("Changelog");
        field_wrapped("", changelog, 70);
    }

    // Full values here and nowhere else — the signature and key are 88 and 44
    // base64 characters, the checksum 64 hex.
    output::subsection("Provenance");
    output::field("Checksum", p.checksum.as_deref().unwrap_or("-"));
    output::field("Signature", p.signature.as_deref().unwrap_or("-"));
    output::field("Public key", p.public_key.as_deref().unwrap_or("-"));
    output::info(
        "Ed25519 over the checksum. No artifact is stored server-side, so nothing can be re-verified against it.",
    );

    let id = p.id.as_deref().unwrap_or(id);
    println!();
    match p.status() {
        "pending" => output::info(&format!(
            "Next: rmsctl updates approve {id} --approver-id <you>"
        )),
        "approved" | "deployed" => {
            output::info(&format!("Next: rmsctl updates deploy {id} <robot-id>"))
        }
        _ => {}
    }
    println!();
    Ok(())
}

// ── deployments ─────────────────────────────────────────────────────────

async fn deployments(client: &ApiClient, ctx: &Ctx, robot_id: &str) -> RmsResult<()> {
    let path = format!("/updates/deployments/{robot_id}");

    if ctx.json {
        return output::json_out(&client.get_value(&path).await?);
    }

    let all: Vec<UpdateDeployment> = client.get(&path).await?;

    output::section("◈", &format!("Update Deployments: {}", robot_id.bold()));

    if all.is_empty() {
        output::info("No update has ever been deployed to this robot.");
        println!();
        return Ok(());
    }

    // A deployment row carries only packageId, so resolve versions from the
    // package list. A rollback row can carry the literal packageId "rollback"
    // (UpdateService.triggerRollback) and resolves to nothing; so does a
    // package that has since been deleted, hence the soft failure.
    let packages: Vec<UpdatePackage> = client.get("/updates").await.unwrap_or_default();
    let version_of = |package_id: Option<&str>| -> String {
        package_id
            .and_then(|pid| {
                packages
                    .iter()
                    .find(|p| p.id.as_deref() == Some(pid))
                    .and_then(|p| p.version.clone())
            })
            .unwrap_or_else(|| "-".into())
    };

    let in_flight = all
        .iter()
        .filter(|d| {
            matches!(
                d.status.as_deref(),
                Some("pending") | Some("downloading") | Some("installing")
            )
        })
        .count();
    let live = all
        .iter()
        .find(|d| d.status.as_deref() == Some("success"))
        .map(|d| version_of(d.package_id.as_deref()));

    output::stat_line(
        "│",
        "Confirmed version",
        live.as_deref().unwrap_or("none confirmed"),
        if live.is_some() { "green" } else { "dim" },
    );
    output::stat_line(
        if in_flight == 0 { "│" } else { "◐" },
        "In flight",
        &in_flight.to_string(),
        if in_flight == 0 { "dim" } else { "orange" },
    );
    println!();

    let rows = page_slice(&all, ctx);
    if rows.is_empty() {
        return past_end(all.len(), ctx);
    }

    output::table_header(&[
        ("CREATED", 25),
        ("STATUS", 14),
        ("VERSION", 9),
        ("PREVIOUS", 9),
        ("PACKAGE", output::ID_W),
    ]);
    for d in rows {
        println!(
            "    {} {} {} {} {}",
            output::pad(d.created_at.as_deref().unwrap_or("-"), 25),
            output::pad(&update_status(d.status.as_deref().unwrap_or("unknown")), 14),
            output::pad(
                &output::truncate(&version_of(d.package_id.as_deref()), 9),
                9
            ),
            output::pad(
                &output::truncate(d.previous_version.as_deref().unwrap_or("-"), 9),
                9
            ),
            d.package_id.as_deref().unwrap_or("-")
        );
        if let Some(err) = d.error_message.as_deref().filter(|e| !e.trim().is_empty()) {
            println!("    {} {}", "└".bright_black(), err.red());
        }
    }

    output::count_paged(
        "deployment(s)",
        rows.len(),
        Some(all.len() as u32),
        total_pages(all.len(), ctx),
    );
    println!();
    Ok(())
}

// ── approve ─────────────────────────────────────────────────────────────

fn require_flag(flag: &str, value: &str) -> RmsResult<String> {
    match value.trim() {
        "" => Err(RmsError::InvalidInput(format!(
            "{flag} is required and must not be blank"
        ))),
        v => Ok(v.to_string()),
    }
}

/// The rollback handler compares the target numerically against the 0.0.1
/// anti-rollback floor, so anything non-numeric becomes a NaN comparison and
/// comes back as a confusing 400. Catch the shape here instead.
fn require_semver(flag: &str, value: &str) -> RmsResult<String> {
    let v = require_flag(flag, value)?;
    let parts: Vec<&str> = v.split('.').collect();
    let numeric = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if numeric {
        Ok(v)
    } else {
        Err(RmsError::InvalidInput(format!(
            "{flag}: '{v}' is not a version like 1.4.0 — the server rejects anything it cannot compare against its 0.0.1 floor"
        )))
    }
}

async fn approve(client: &ApiClient, ctx: &Ctx, id: &str, approver_id: &str) -> RmsResult<()> {
    let approver_id = require_flag("--approver-id", approver_id)?;

    // Also proves the ID exists before the gate quotes a version at the user.
    let pkg = fetch_package(client, id).await?;
    if pkg.status() != "pending" {
        output::warn(&format!(
            "Update is already {} — the server only approves packages in 'pending' and will answer 400.",
            pkg.status()
        ));
    }

    if !output::confirm(
        &format!(
            "This will APPROVE update {} ({id}) for fleet deployment, signed off as {approver_id}",
            pkg.version()
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let body = serde_json::json!({ "approverId": approver_id });
    let resp = client
        .post_value(&format!("/updates/{id}/approve"), &body)
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    output::success(&format!("Update {} approved", pkg.version()));
    if let Ok(approved) = serde_json::from_value::<UpdatePackage>(resp) {
        output::field("Status", &update_status(approved.status()));
        if let Some(at) = &approved.approved_at {
            output::field("Approved at", at);
        }
        output::info(&format!(
            "Next: rmsctl updates deploy {id} <robot-id> --yes"
        ));
    }
    println!();
    Ok(())
}

// ── deploy ──────────────────────────────────────────────────────────────

async fn deploy(
    client: &ApiClient,
    ctx: &Ctx,
    id: &str,
    robot_id: &str,
    previous_version: Option<&str>,
) -> RmsResult<()> {
    let previous_version = match previous_version {
        Some(v) => Some(require_flag("--previous-version", v)?),
        None => None,
    };

    let pkg = fetch_package(client, id).await?;
    if !matches!(pkg.status(), "approved" | "deployed") {
        output::warn(&format!(
            "Update is {} — the server refuses to deploy anything that has not been approved.",
            pkg.status()
        ));
    }

    let mut what = format!(
        "This will DEPLOY firmware {} ({id}) to robot {robot_id}",
        pkg.version()
    );
    if let Some(prev) = &previous_version {
        what.push_str(&format!(", recording {prev} as the version it replaces"));
    }
    if !output::confirm(&what, ctx.yes) {
        return Ok(());
    }

    // The route reads only `previousVersion`; send `{}` when it is absent
    // rather than an explicit null.
    let body = match &previous_version {
        Some(prev) => serde_json::json!({ "previousVersion": prev }),
        None => serde_json::json!({}),
    };
    let resp = client
        .post_value(&format!("/updates/{id}/deploy/{robot_id}"), &body)
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    output::success(&format!(
        "Deployment of {} to {robot_id} recorded",
        pkg.version()
    ));
    render_deployment(&resp);
    // Worth saying out loud: UpdateService.deployToRobot writes the row and
    // flips the package to 'deployed' without ever contacting the robot.
    output::info(
        "The server records the intent and flips the package to 'deployed'; it does not push the artifact to the robot.",
    );
    output::info(&format!(
        "Track it with: rmsctl updates deployments {robot_id}"
    ));
    println!();
    Ok(())
}

// ── rollback ────────────────────────────────────────────────────────────

async fn rollback(
    client: &ApiClient,
    ctx: &Ctx,
    id: &str,
    robot_id: &str,
    target_version: &str,
) -> RmsResult<()> {
    let target_version = require_semver("--target-version", target_version)?;

    // The handler ignores the package ID in the path — triggerRollback works
    // from the robot's own last successful deployment — so the ID is only
    // there to keep the route shape. Don't imply it selects the target.
    let pkg = fetch_package(client, id).await?;

    if !output::confirm(
        &format!(
            "This will ROLL BACK robot {robot_id} to version {target_version}, marking its current update ({}) as rolled_back",
            pkg.version()
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let body = serde_json::json!({ "targetVersion": target_version });
    let resp = client
        .post_value(&format!("/updates/{id}/rollback/{robot_id}"), &body)
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    output::success(&format!(
        "Rollback of {robot_id} to {target_version} recorded"
    ));
    render_deployment(&resp);
    output::info(&format!(
        "Check it with: rmsctl updates deployments {robot_id}"
    ));
    println!();
    Ok(())
}

/// Both deploy and rollback answer with an UpdateDeployment row.
fn render_deployment(resp: &serde_json::Value) {
    let Ok(d) = serde_json::from_value::<UpdateDeployment>(resp.clone()) else {
        return;
    };
    if let Some(id) = &d.id {
        output::field("Deployment", id);
    }
    output::field(
        "Status",
        &update_status(d.status.as_deref().unwrap_or("unknown")),
    );
    if let Some(prev) = &d.previous_version {
        output::field("Previous", prev);
    }
    if let Some(at) = &d.rolled_back_at {
        output::field("Rolled back", at);
    }
    if let Some(at) = &d.deployed_at {
        output::field("Deployed at", at);
    }
    if let Some(err) = d.error_message.as_deref().filter(|e| !e.trim().is_empty()) {
        output::field_colored("Error", err.red());
    }
}
