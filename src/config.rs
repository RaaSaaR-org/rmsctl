use crate::error::{RmsError, RmsResult};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::OnceLock;

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

/// The server URL this run resolved (flag > env > file > default), set once by
/// `load_config`. The "is the server up?" hint must name the server that was
/// actually asked, not whatever `RMS_SERVER_URL` happens to say.
static RESOLVED_SERVER_URL: OnceLock<String> = OnceLock::new();

/// Health endpoint for the "is the server up?" hint: the resolved server's,
/// or — before a config was loaded — the env var's, else the default.
pub fn health_probe_url() -> String {
    let base = RESOLVED_SERVER_URL.get().cloned().unwrap_or_else(|| {
        std::env::var("RMS_SERVER_URL").unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string())
    });
    health_url_for(&base)
}

/// The server mounts `/health` at the root, not under `/api`, so a trailing
/// `/api` is stripped.
fn health_url_for(base: &str) -> String {
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

    let _ = RESOLVED_SERVER_URL.set(server_url.clone());
    Ok(RmsConfig { server_url, token })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_health_url_hangs_off_the_root_of_the_server_that_was_used() {
        assert_eq!(
            health_url_for("https://neodem.emai.dev/api"),
            "https://neodem.emai.dev/health"
        );
        assert_eq!(
            health_url_for("https://neodem.emai.dev/api/"),
            "https://neodem.emai.dev/health"
        );
        assert_eq!(
            health_url_for("http://localhost:3001"),
            "http://localhost:3001/health"
        );
    }

    #[test]
    fn the_hint_names_the_server_passed_with_server_not_the_env_var() {
        // Whatever RMS_SERVER_URL says in the test's environment, --server wins.
        let cfg = load_config(Some("https://neodem-bogus.invalid/api"), None).unwrap();
        assert_eq!(cfg.server_url, "https://neodem-bogus.invalid/api");
        assert_eq!(health_probe_url(), "https://neodem-bogus.invalid/health");
    }
}
