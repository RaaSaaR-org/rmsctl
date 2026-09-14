use crate::cli::AlertsCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::models::{Alert, AlertsResponse};
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};

pub async fn run(cmd: &AlertsCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        AlertsCommand::List { severity, active } => {
            list(client, ctx, severity.as_deref(), *active).await
        }
        AlertsCommand::Show { id } => show(client, ctx, id).await,
        AlertsCommand::Ack { id } => ack(client, ctx, id).await,
        AlertsCommand::History => history(client, ctx).await,
        AlertsCommand::Delete { id } => delete(client, ctx, id).await,
        AlertsCommand::Clear { acknowledged, all } => clear(client, ctx, *acknowledged, *all).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────
//
// `Alert` / `AlertsResponse` live in models.rs because the status dashboard
// needs them too. Only the bulk-delete envelope is local.

/// `DELETE /alerts/clear/*` answers `{ "success": true, "deleted": 55 }`,
/// unlike `DELETE /alerts/{id}` which sends only `{ "success": true }`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClearResponse {
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    deleted: Option<u32>,
}

const SEVERITIES: [&str; 4] = ["critical", "error", "warning", "info"];

// ── list ────────────────────────────────────────────────────────────────

async fn list(
    client: &ApiClient,
    ctx: &Ctx,
    severity: Option<&str>,
    active: bool,
) -> RmsResult<()> {
    let severity = severity.map(normalize_severity).transpose()?;

    let mut own: Vec<(&'static str, String)> = Vec::new();
    if let Some(s) = &severity {
        own.push(("severity", s.clone()));
    }

    // `/alerts/active` takes only the severity/source filters: it ignores
    // page/pageSize and answers `{alerts:[...]}` with no pagination envelope,
    // so paging flags are pointless there and say so below.
    let (path, query) = if active {
        ("/alerts/active", own)
    } else {
        ("/alerts", ctx.with_paging(own))
    };

    if ctx.json {
        return output::json_out(&client.get_value_query(path, &query).await?);
    }

    let resp: AlertsResponse = client.get_query(path, &query).await?;

    output::section("◈", if active { "Active Alerts" } else { "Alerts" });

    if active && ctx.has_paging() {
        output::info(
            "--page/--limit do not apply to --active: the server returns every unacknowledged alert at once.",
        );
    }

    if resp.data.is_empty() {
        // An empty page under a filter is not an all-clear — the server does
        // not validate the GET filters, so a narrow filter looks identical to
        // a quiet fleet. Always name the filter that produced the emptiness.
        match describe_filter(severity.as_deref(), active) {
            Some(f) => output::info(&format!("No alerts match {f}.")),
            None => output::info("All clear — no alerts recorded."),
        }
        println!();
        return Ok(());
    }

    // The ID column is what makes `alerts ack`/`show`/`delete` usable: titles
    // repeat heavily in real data (75 alerts over 17 distinct titles).
    output::table_header(&[
        ("ID", output::ID_W),
        ("SEVERITY", 11),
        ("TITLE", 38),
        ("SOURCE", 15),
        ("ACK", 3),
    ]);
    for a in &resp.data {
        println!(
            "    {} {} {} {} {}",
            output::pad(&a.id, output::ID_W),
            output::pad(&severity_cell(a.severity.as_deref()), 11),
            output::pad(&output::truncate(a.title.as_deref().unwrap_or("-"), 38), 38),
            output::pad(&output::truncate(source_cell(a), 15), 15),
            output::bool_indicator(a.acknowledged),
        );
    }

    let pag = resp.pagination.as_ref();
    output::count_paged(
        if active {
            "active alert(s)"
        } else {
            "alert(s)"
        },
        resp.data.len(),
        pag.and_then(|p| p.total()),
        pag.and_then(|p| p.total_pages()),
    );
    println!();
    Ok(())
}

/// The `source` field is only a category (robot | task | system | user); the
/// identifying value is `sourceId`, so prefer it and fall back.
fn source_cell(a: &Alert) -> &str {
    a.source_id
        .as_deref()
        .or(a.source.as_deref())
        .unwrap_or("-")
}

fn severity_cell(severity: Option<&str>) -> String {
    output::severity_indicator(severity.unwrap_or("unknown"))
}

fn describe_filter(severity: Option<&str>, active: bool) -> Option<String> {
    let mut parts = Vec::new();
    if active {
        parts.push("unacknowledged only".to_string());
    }
    if let Some(s) = severity {
        parts.push(format!("severity {s}"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(format!("the filter in force ({})", parts.join(", ")))
    }
}

/// Validate and lower-case `--severity` before it reaches the wire.
///
/// The server validates severity only on POST: the GET filters cast the raw
/// string straight through, so `?severity=CRITICAL` or `?severity=bogus`
/// answers 200 with an empty page, which reads as "all clear".
fn normalize_severity(raw: &str) -> RmsResult<String> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let value = part.trim().to_lowercase();
        if value.is_empty() {
            continue;
        }
        if !SEVERITIES.contains(&value.as_str()) {
            return Err(RmsError::InvalidInput(format!(
                "unknown severity '{}': expected one of {} (comma-separate for several, e.g. --severity warning,info)",
                part.trim(),
                SEVERITIES.join(", ")
            )));
        }
        out.push(value);
    }
    if out.is_empty() {
        return Err(RmsError::InvalidInput(format!(
            "--severity needs a value: one of {} (comma-separated)",
            SEVERITIES.join(", ")
        )));
    }
    Ok(out.join(","))
}

// ── show ────────────────────────────────────────────────────────────────

async fn show(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    let v = client.get_value(&format!("/alerts/{id}")).await?;

    if ctx.json {
        return output::json_out(&v);
    }

    // This route answers a bare Alert object, with no envelope.
    let a: Alert = serde_json::from_value(v.clone())?;

    output::section("◈", &format!("Alert {}", a.id.bold()));
    output::field("Severity", &severity_cell(a.severity.as_deref()));
    output::field("Title", a.title.as_deref().unwrap_or("-"));
    output::field("Message", a.message.as_deref().unwrap_or("-"));
    output::field("Source", a.source.as_deref().unwrap_or("-"));
    if let Some(source_id) = &a.source_id {
        output::field("Source ID", source_id);
    }
    output::field("Raised", a.timestamp.as_deref().unwrap_or("-"));

    output::subsection("Acknowledgement");
    match a.acknowledged {
        Some(true) => {
            output::field_colored("State", "acknowledged".green());
            // acknowledgedAt / dismissable / autoDismissMs are not on the
            // shared `Alert` struct, so read them off the untouched body.
            if let Some(at) = v.get("acknowledgedAt").and_then(|x| x.as_str()) {
                output::field("At", at);
            }
            output::field("By", a.acknowledged_by.as_deref().unwrap_or("-"));
        }
        _ => output::field_colored("State", "ACTIVE — unacknowledged".yellow().bold()),
    }

    if let Some(d) = v.get("dismissable").and_then(|x| x.as_bool()) {
        output::field("Dismissable", &output::bool_indicator(Some(d)));
    }
    if let Some(ms) = v.get("autoDismissMs").and_then(|x| x.as_u64()) {
        output::field("Auto-dismiss", &duration(ms));
    }

    println!();
    Ok(())
}

/// `autoDismissMs` arrives in milliseconds and runs to a week, which reads
/// badly as "10080 min".
fn duration(ms: u64) -> String {
    match ms {
        0..=999 => format!("{ms} ms"),
        _ if ms < 60_000 => format!("{} s", ms / 1_000),
        _ if ms < 3_600_000 => format!("{} min", ms / 60_000),
        _ if ms < 86_400_000 => format!("{} h", ms / 3_600_000),
        _ => format!("{} d", ms / 86_400_000),
    }
}

// ── ack ─────────────────────────────────────────────────────────────────

async fn ack(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    if !output::confirm(
        &format!(
            "This will acknowledge alert {id}{} — it drops off the active list",
            describe(client, id).await
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    // The route ignores the body and takes the acknowledging user from the
    // auth context, so with AUTH_DISABLED acknowledgedBy comes back null.
    let resp = client
        .patch_value(&format!("/alerts/{id}/acknowledge"), &serde_json::json!({}))
        .await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    output::success(&format!("Alert {id} acknowledged"));
    if let Some(at) = resp.get("acknowledgedAt").and_then(|x| x.as_str()) {
        output::field("At", at);
    }
    output::field(
        "By",
        resp.get("acknowledgedBy")
            .and_then(|x| x.as_str())
            .unwrap_or("- (server had no authenticated user)"),
    );
    println!();
    Ok(())
}

// ── history ─────────────────────────────────────────────────────────────

async fn history(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    // This route pages on `page`/`pageSize` like the list route; a raw
    // `?limit=` is accepted and ignored, still answering 50 rows. So
    // `paging()`, never `paging_limit()`.
    let query = ctx.with_paging(Vec::new());

    if ctx.json {
        return output::json_out(&client.get_value_query("/alerts/history", &query).await?);
    }

    let resp: AlertsResponse = client.get_query("/alerts/history", &query).await?;

    output::section("◈", "Alert History");

    if resp.data.is_empty() {
        output::info("No alerts recorded.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("WHEN", 16),
        ("ID", output::ID_W),
        ("SEVERITY", 11),
        ("TITLE", 32),
        ("ACK", 3),
    ]);
    for a in &resp.data {
        println!(
            "    {} {} {} {} {}",
            output::pad(&when(a.timestamp.as_deref()), 16),
            output::pad(&a.id, output::ID_W),
            output::pad(&severity_cell(a.severity.as_deref()), 11),
            output::pad(&output::truncate(a.title.as_deref().unwrap_or("-"), 32), 32),
            output::bool_indicator(a.acknowledged),
        );
    }

    let pag = resp.pagination.as_ref();
    output::count_paged(
        "alert(s) in history",
        resp.data.len(),
        pag.and_then(|p| p.total()),
        pag.and_then(|p| p.total_pages()),
    );
    println!();
    Ok(())
}

/// `2026-09-13T16:00:31.680Z` -> `09-13 16:00:31`. Anything in another shape
/// is passed through untouched rather than mangled.
fn when(ts: Option<&str>) -> String {
    let ts = ts.unwrap_or("-");
    let chars: Vec<char> = ts.chars().collect();
    if chars.len() >= 19 && chars[4] == '-' && chars[10] == 'T' {
        let date: String = chars[5..10].iter().collect();
        let time: String = chars[11..19].iter().collect();
        format!("{date} {time}")
    } else {
        ts.to_string()
    }
}

// ── delete / clear ──────────────────────────────────────────────────────

async fn delete(client: &ApiClient, ctx: &Ctx, id: &str) -> RmsResult<()> {
    if !output::confirm(
        &format!(
            "This will PERMANENTLY DELETE alert {id}{} — acknowledging it instead keeps the audit trail",
            describe(client, id).await
        ),
        ctx.yes,
    ) {
        return Ok(());
    }

    let resp = client.delete_value(&format!("/alerts/{id}")).await?;

    if ctx.json {
        return output::json_out(&resp);
    }
    output::success(&format!("Alert {id} deleted"));
    println!();
    Ok(())
}

async fn clear(client: &ApiClient, ctx: &Ctx, acknowledged: bool, all: bool) -> RmsResult<()> {
    let (path, scope, filter) = match (acknowledged, all) {
        (true, true) => {
            return Err(RmsError::InvalidInput(
                "pass either --acknowledged or --all, not both".into(),
            ));
        }
        (true, false) => (
            "/alerts/clear/acknowledged",
            "acknowledged alert(s)",
            Some(("acknowledged", "true")),
        ),
        (false, true) => ("/alerts/clear/all", "alert(s), acknowledged or not", None),
        (false, false) => {
            return Err(RmsError::InvalidInput(
                "say what to clear: --acknowledged (only acknowledged alerts) or --all (every alert)"
                    .into(),
            ));
        }
    };

    // The DELETE routes report only how many they removed, after the fact.
    // Count first so the confirmation names the real blast radius.
    let doomed = match total_matching(client, filter).await {
        Some(n) => format!("all {n} "),
        None => "all ".to_string(),
    };

    if !output::confirm(
        &format!("This will PERMANENTLY DELETE {doomed}{scope}"),
        ctx.yes,
    ) {
        return Ok(());
    }

    let resp = client.delete_value(path).await?;

    if ctx.json {
        return output::json_out(&resp);
    }

    let parsed: ClearResponse = serde_json::from_value(resp)?;
    match parsed.deleted {
        Some(n) => output::success(&format!("Deleted {n} alert(s)")),
        None if parsed.success == Some(true) => output::success("Alerts cleared"),
        None => output::warn("The server did not report how many alerts it deleted."),
    }
    println!();
    Ok(())
}

/// Total rows behind a filter, via the list route's pagination envelope.
///
/// `pageSize=1` keeps it cheap: only the count is wanted. Returns None rather
/// than failing — this only sharpens a confirmation message.
async fn total_matching(client: &ApiClient, filter: Option<(&str, &str)>) -> Option<u32> {
    let mut query: Vec<(&str, String)> = vec![("pageSize", "1".to_string())];
    if let Some((k, v)) = filter {
        query.push((k, v.to_string()));
    }
    let resp: AlertsResponse = client.get_query("/alerts", &query).await.ok()?;
    resp.pagination.and_then(|p| p.total())
}

/// Best-effort `— "title" (severity)` suffix for a confirmation line.
///
/// A lookup failure is swallowed: the mutating call a moment later reports the
/// 404 with a proper hint, and the gate should not die on a cosmetic fetch.
async fn describe(client: &ApiClient, id: &str) -> String {
    match client.get::<Alert>(&format!("/alerts/{id}")).await {
        Ok(a) => match a.title {
            Some(t) => format!(
                " — \"{}\" ({})",
                output::truncate(&t, 60),
                a.severity.as_deref().unwrap_or("unknown severity")
            ),
            None => String::new(),
        },
        Err(_) => String::new(),
    }
}
