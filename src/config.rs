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

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("rmsctl").join("config.json"))
}

fn load_config_file() -> Option<ConfigFile> {
    let path = config_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn load_config(
    cli_server: Option<&str>,
    cli_token: Option<&str>,
) -> RmsResult<RmsConfig> {
    let file = load_config_file();

    let server_url = cli_server
        .map(String::from)
        .or_else(|| std::env::var("RMS_SERVER_URL").ok())
        .or_else(|| file.as_ref().and_then(|f| f.server_url.clone()))
        .unwrap_or_else(|| DEFAULT_SERVER_URL.to_string());

    // Validate URL
    if !server_url.starts_with("http://") && !server_url.starts_with("https://") {
        return Err(RmsError::Config(format!(
            "Invalid server URL: {server_url}"
        )));
    }

    let token = cli_token
        .map(String::from)
        .or_else(|| std::env::var("RMS_TOKEN").ok())
        .or_else(|| file.and_then(|f| f.token));

    Ok(RmsConfig { server_url, token })
}
