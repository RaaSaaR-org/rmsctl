use crate::cli::EmbodimentsCommand;
use crate::client::ApiClient;
use crate::ctx::Ctx;
use crate::error::{RmsError, RmsResult};
use crate::models::Pagination;
use crate::output;
use colored::*;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub async fn run(cmd: &EmbodimentsCommand, client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    match cmd {
        EmbodimentsCommand::List => list(client, ctx).await,
        EmbodimentsCommand::Show { tag } => show(client, ctx, tag).await,
    }
}

// ── Response shapes ─────────────────────────────────────────────────────

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

impl RobotTypeRef {
    /// `name (manufacturer model)` — the readable half of the join the server
    /// already performs, so nobody has to look a UUID up by hand.
    fn label(&self) -> Option<String> {
        let name = self.name.as_deref().filter(|n| !n.is_empty())?;
        match (self.manufacturer.as_deref(), self.model.as_deref()) {
            (Some(mfr), Some(model)) if !mfr.is_empty() && !model.is_empty() => {
                Some(format!("{name} ({mfr} {model})"))
            }
            _ => Some(name.to_string()),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Embodiment {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    config_yaml: Option<String>,
    #[serde(default)]
    action_dim: Option<u32>,
    #[serde(default)]
    proprioception_dim: Option<u32>,
    #[serde(default)]
    robot_type_id: Option<String>,
    #[serde(default)]
    robot_type: Option<RobotTypeRef>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EmbodimentsResponse {
    #[serde(default)]
    embodiments: Vec<Embodiment>,
    #[serde(default)]
    pagination: Option<Pagination>,
}

/// `GET /embodiments/:tag` answers `{ "embodiment": { ... } }`, but the same
/// prefix with an empty segment falls through to the *list* handler, so the key
/// stays optional rather than blowing up with a bare serde error.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EmbodimentDetailResponse {
    #[serde(default)]
    embodiment: Option<Embodiment>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EmbodimentConfigResponse {
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    config: Option<Value>,
}

// ── list ────────────────────────────────────────────────────────────────

/// The server applies `pageSize = 20` when the request sends nothing, which
/// silently clipped the table while the footer printed `pagination.total`. Ask
/// for a big page unless the user picked a slice themselves.
const LIST_PAGE_SIZE: &str = "200";

async fn list(client: &ApiClient, ctx: &Ctx) -> RmsResult<()> {
    let query: Vec<(&'static str, String)> = if ctx.has_paging() {
        ctx.paging()
    } else {
        vec![("pageSize", LIST_PAGE_SIZE.to_string())]
    };

    if ctx.json {
        return output::json_out(&client.get_value_query("/embodiments", &query).await?);
    }

    let resp: EmbodimentsResponse = client.get_query("/embodiments", &query).await?;

    output::section("◈", "Embodiments");

    if resp.embodiments.is_empty() {
        output::info("No embodiment configs found.");
        println!();
        return Ok(());
    }

    output::table_header(&[
        ("TAG", 18),
        ("MANUFACTURER", 16),
        ("MODEL", 16),
        ("ACT DIM", 8),
        ("PROP DIM", 8),
    ]);

    for e in &resp.embodiments {
        let tag = output::truncate(e.tag.as_deref().unwrap_or("-"), 17);
        println!(
            "    {} {} {} {} {}",
            output::pad(&tag.bold().to_string(), 18),
            output::pad(
                &output::truncate(e.manufacturer.as_deref().unwrap_or("-"), 15),
                16
            ),
            output::pad(&output::truncate(e.model.as_deref().unwrap_or("-"), 15), 16),
            output::pad(&dim_cell(e.action_dim), 8),
            dim_cell(e.proprioception_dim)
        );
    }

    output::count_paged(
        "embodiment(s)",
        resp.embodiments.len(),
        resp.pagination.as_ref().and_then(|p| p.total()),
        resp.pagination.as_ref().and_then(|p| p.total_pages()),
    );
    println!();
    Ok(())
}

fn dim_cell(v: Option<u32>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "-".into())
}

// ── show ────────────────────────────────────────────────────────────────

async fn show(client: &ApiClient, ctx: &Ctx, tag: &str) -> RmsResult<()> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Err(RmsError::InvalidInput(
            "a tag is required: `rmsctl embodiments show <tag>` (an empty tag hits the list route, not a detail)"
                .into(),
        ));
    }

    let seg = encode_segment(tag);
    let detail_path = format!("/embodiments/{seg}");
    let config_path = format!("/embodiments/{seg}/config");

    if ctx.json {
        let (detail, config) = tokio::join!(
            client.get_value(&detail_path),
            client.get_value(&config_path)
        );
        let detail = detail.map_err(|e| not_found(e, tag))?;

        // Pass the detail body through untouched and graft the config onto it,
        // so scripts still see id / configYaml / timestamps / robotType.
        let mut out = match detail {
            Value::Object(map) => map,
            other => {
                let mut map = Map::new();
                map.insert("embodiment".into(), other);
                map
            }
        };
        match config {
            Ok(v) => {
                let cfg = v.get("config").cloned().unwrap_or(v);
                out.insert("config".into(), cfg);
            }
            // A swallowed error here used to look identical to a healthy
            // embodiment: configYaml is NOT NULL, so every embodiment has one.
            Err(e) => {
                out.insert("config".into(), Value::Null);
                out.insert("configError".into(), Value::String(e.to_string()));
            }
        }
        return output::json_out(&Value::Object(out));
    }

    let (detail, config) = tokio::join!(
        client.get::<EmbodimentDetailResponse>(&detail_path),
        client.get::<EmbodimentConfigResponse>(&config_path)
    );

    let e = detail
        .map_err(|e| not_found(e, tag))?
        .embodiment
        .ok_or_else(|| {
            RmsError::Other(format!(
                "The server answered without an embodiment for tag '{tag}'."
            ))
        })?;

    output::section("◈", &format!("Embodiment: {}", tag.bold()));

    if let Some(v) = &e.tag {
        output::field("Tag", v);
    }
    if let Some(v) = &e.id {
        output::field("ID", v);
    }
    if let Some(v) = &e.manufacturer {
        output::field("Manufacturer", v);
    }
    if let Some(v) = &e.model {
        output::field("Model", v);
    }
    match e.description.as_deref().filter(|d| !d.trim().is_empty()) {
        Some(d) => output::field("Description", d),
        None => output::field_colored("Description", "—".bright_black()),
    }
    if let Some(v) = e.action_dim {
        output::field("Action Dim", &v.to_string());
    }
    if let Some(v) = e.proprioception_dim {
        output::field("Proprio Dim", &v.to_string());
    }

    match (&e.robot_type, e.robot_type_id.as_deref()) {
        (Some(rt), id) => {
            match rt.label() {
                Some(label) => output::field("Robot Type", &label),
                None => output::field("Robot Type", id.unwrap_or("-")),
            }
            if let Some(id) = rt.id.as_deref().or(id) {
                output::field("Type ID", id);
            }
        }
        (None, Some(id)) => {
            output::field("Robot Type", id);
        }
        (None, None) => output::field_colored("Robot Type", "not linked".bright_black()),
    }

    // The stored YAML is the source of truth but too long for a detail view;
    // say how much there is and leave the text itself to --json.
    if let Some(yaml) = &e.config_yaml {
        let lines = yaml.lines().count();
        output::field(
            "Config YAML",
            &format!(
                "{lines} line{}, {} bytes",
                if lines == 1 { "" } else { "s" },
                yaml.len()
            ),
        );
    }
    if let Some(v) = &e.created_at {
        output::field("Created", v);
    }
    if let Some(v) = &e.updated_at {
        output::field("Updated", v);
    }

    match config {
        Ok(cfg) => match cfg.config.filter(|v| !v.is_null()) {
            Some(v) => render_config(&v),
            // configYaml is non-nullable, so a 200 with no config at all means
            // the route changed shape — say so instead of printing nothing.
            None => {
                println!();
                output::warn("The server returned no parsed configuration for this embodiment.");
            }
        },
        // Only reachable if the row vanished between the two requests.
        Err(RmsError::Api { status: 404, .. }) => {}
        Err(err) => {
            println!();
            output::warn(&format!("Configuration unavailable: {err}"));
        }
    }

    println!();
    Ok(())
}

/// The shared 404 hint talks about IDs and the corresponding `list`, but this
/// route is keyed on a tag and the table prints no ID — so answer in the
/// command's own vocabulary.
fn not_found(err: RmsError, tag: &str) -> RmsError {
    match err {
        RmsError::Api { status: 404, .. } => RmsError::Other(format!(
            "No embodiment with tag '{tag}'. Run `rmsctl embodiments list` to see the known tags."
        )),
        other => other,
    }
}

/// Percent-encode one path segment.
///
/// A tag carrying `/`, a space or `?` would otherwise rewrite the route: the
/// list handler sits on the same prefix and answers 200 with a completely
/// different envelope.
fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

// ── config rendering ────────────────────────────────────────────────────

/// Width budget for a value printed on one field row.
const VALUE_W: usize = 52;

/// `/embodiments/:tag/config` returns the whole parsed YAML document, and its
/// top level is almost all nested — action, proprioception, cameras, limits.
/// Flattening that with `to_string` put ~58 normalisation floats on one line,
/// so group containers into their own blocks and preview long arrays.
fn render_config(config: &Value) {
    output::subsection("Configuration");

    let Some(obj) = config.as_object() else {
        output::field("config", &compact(config, VALUE_W));
        return;
    };

    // Scalars and flat lists first: they are the identity of the config.
    for (key, val) in obj {
        if let Some(s) = leaf_str(val) {
            output::field(key, &s);
        }
    }

    for (key, val) in obj {
        if leaf_str(val).is_some() {
            continue;
        }
        output::subsection(key);
        match val {
            Value::Object(nested) => {
                for (k, v) in nested {
                    match leaf_str(v) {
                        Some(s) => output::field(k, &s),
                        // Two levels down (limits.position, action.normalization)
                        // is where the numeric payloads live.
                        None => match v.as_object() {
                            Some(inner) => {
                                for (ik, iv) in inner {
                                    output::field(
                                        &format!("{k}.{ik}"),
                                        &leaf_str(iv).unwrap_or_else(|| compact(iv, VALUE_W)),
                                    );
                                }
                            }
                            None => output::field(k, &compact(v, VALUE_W)),
                        },
                    }
                }
            }
            // Arrays of objects: cameras, mostly. Name each entry and summarise
            // its scalar fields rather than dumping the JSON.
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    match item.as_object() {
                        Some(map) => {
                            // Camera names run past the 14-char label column;
                            // a clipped name is worse than a ragged row.
                            let label = map
                                .get("name")
                                .and_then(|n| n.as_str())
                                .map(|n| output::truncate(n, 24))
                                .unwrap_or_else(|| format!("[{i}]"));
                            output::field(&label, &summarise_object(map));
                        }
                        None => output::field(
                            &format!("[{i}]"),
                            &leaf_str(item).unwrap_or_else(|| compact(item, VALUE_W)),
                        ),
                    }
                }
                if items.is_empty() {
                    output::field_colored("(empty)", "—".bright_black());
                }
            }
            other => output::field(key, &compact(other, VALUE_W)),
        }
    }
}

/// A scalar, or a list of scalars previewed with its length — anything that
/// belongs on a single field row.
fn leaf_str(v: &Value) -> Option<String> {
    match v {
        Value::Null => Some("-".into()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(output::truncate(s, VALUE_W)),
        Value::Array(items) => {
            if items.is_empty() {
                return Some("[] (0)".into());
            }
            let parts: Option<Vec<String>> = items.iter().map(scalar_str).collect();
            let parts = parts?;
            let joined = parts.join(", ");
            let n = parts.len();
            if joined.chars().count() <= VALUE_W {
                Some(format!("[{joined}] ({n})"))
            } else {
                Some(format!(
                    "[{}] ({n})",
                    output::truncate(&joined, VALUE_W.saturating_sub(6))
                ))
            }
        }
        Value::Object(_) => None,
    }
}

fn scalar_str(v: &Value) -> Option<String> {
    match v {
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        Value::Null => Some("null".into()),
        _ => None,
    }
}

/// `resolution=[640, 480] (2) fov=70` — one line per array entry.
fn summarise_object(map: &Map<String, Value>) -> String {
    let parts: Vec<String> = map
        .iter()
        .filter(|(k, _)| k.as_str() != "name")
        .map(|(k, v)| {
            let val = leaf_str(v).unwrap_or_else(|| compact(v, 20));
            format!("{k}={val}")
        })
        .collect();
    if parts.is_empty() {
        return "-".into();
    }
    output::truncate(&parts.join("  "), VALUE_W)
}

fn compact(v: &Value, max: usize) -> String {
    output::truncate(&serde_json::to_string(v).unwrap_or_default(), max)
}
