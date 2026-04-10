use thiserror::Error;

#[derive(Error, Debug)]
pub enum RmsError {
    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },

    #[error("Network error: {0}")]
    Network(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("{0}")]
    Other(String),
}

impl RmsError {
    pub fn hint(&self) -> Option<String> {
        match self {
            RmsError::Api { status: 404, .. } => Some(
                "Resource not found. Check the ID and try the corresponding 'list' command.".into(),
            ),
            RmsError::Network(_) => Some(
                "Is the NeoDEM server running? Try: curl http://localhost:3001/health".into(),
            ),
            RmsError::Config(_) => Some(
                "Run with --server <url> or set RMS_SERVER_URL, or create ~/.config/rmsctl/config.json"
                    .into(),
            ),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for RmsError {
    fn from(e: reqwest::Error) -> Self {
        RmsError::Network(e.to_string())
    }
}

impl From<serde_json::Error> for RmsError {
    fn from(e: serde_json::Error) -> Self {
        RmsError::Other(format!("JSON parse error: {e}"))
    }
}

pub type RmsResult<T> = Result<T, RmsError>;
