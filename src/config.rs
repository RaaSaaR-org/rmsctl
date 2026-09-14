use crate::error::{RmsError, RmsResult};
use serde::Deserialize;
use std::path::PathBuf;

pub struct RmsConfig {
    pub server_url: String,
    pub token: Option<String>,
}

#[derive(Deserialize)]
struct ConfigFile {
    server_url: Option<String>,
    token: Option<String>,
}

const DEFAULT_SERVER_URL: &str = "http://localhost:3001/api";

/// Candidate config locations, in precedence order.
///
/// `~/.config/rmsctl/config.json` comes first on every platform: it is what the
/// docs have always promised and what anyone coming from kubectl expects. On
/// macOS `dirs::config_dir()` resolves to `~/Library/Application Support`, so
/// that is kept as a fallback rather than the only answer.
fn config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".config").join("rmsctl").join("config.json"));
    }
    if let Some(dir) = dirs::config_dir() {
        let p = dir.join("rmsctl").join("config.json");
        if !paths.contains(&p) {
            paths.push(p);
        }
    }
    paths
}

/// The path shown in error hints and docs.
pub fn config_path_display() -> String {
    config_paths()
        .first()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "~/.config/rmsctl/config.json".to_string())
}

/// Health endpoint for the "is the server up?" hint. The server mounts it at
/// the root, not under `/api`, so strip a trailing `/api` if present.
pub fn health_probe_url() -> String {
    let base = std::env::var("RMS_SERVER_URL").unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
    let root = base.trim_end_matches('/').trim_end_matches("/api");
    format!("{root}/health")
}

fn load_config_file() -> Option<ConfigFile> {
    for path in config_paths() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            match serde_json::from_str::<ConfigFile>(&content) {
                Ok(cfg) => return Some(cfg),
                Err(e) => {
                    eprintln!(
                        "  warning: ignoring malformed config at {}: {e}",
                        path.display()
                    );
                }
            }
        }
    }
    None
}

pub fn load_config(cli_server: Option<&str>, cli_token: Option<&str>) -> RmsResult<RmsConfig> {
    let file = load_config_file();

    let server_url = cli_server
        .map(String::from)
        .or_else(|| std::env::var("RMS_SERVER_URL").ok())
        .or_else(|| file.as_ref().and_then(|f| f.server_url.clone()))
        .unwrap_or_else(|| DEFAULT_SERVER_URL.to_string());

    if !server_url.starts_with("http://") && !server_url.starts_with("https://") {
        return Err(RmsError::Config(format!(
            "Invalid server URL: {server_url} (must start with http:// or https://)"
        )));
    }

    let token = cli_token
        .map(String::from)
        .or_else(|| std::env::var("RMS_TOKEN").ok())
        .or_else(|| file.and_then(|f| f.token));

    Ok(RmsConfig { server_url, token })
}
