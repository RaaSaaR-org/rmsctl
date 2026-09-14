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

    #[error("Failed to parse response: {0}")]
    Parse(String),

    #[error("{0}")]
    Other(String),
}

impl RmsError {
    pub fn hint(&self) -> Option<String> {
        match self {
            RmsError::Api { status: 400, .. } => {
                Some("The server rejected the request body. Check the flags against --help.".into())
            }
            RmsError::Api { status: 401, .. } | RmsError::Api { status: 403, .. } => Some(
                "Not authorised. Pass --token <jwt>, set RMS_TOKEN, or run the server with AUTH_DISABLED=true."
                    .into(),
            ),
            RmsError::Api { status: 404, .. } => Some(
                "Resource not found. Check the ID and try the corresponding 'list' command.".into(),
            ),
            RmsError::Api { status: 409, .. } => {
                Some("Conflict — the resource is already in that state or busy.".into())
            }
            RmsError::Api { status: 502, .. } | RmsError::Api { status: 504, .. } => Some(
                "The server reached the robot agent but got no usable answer. Is the robot online?"
                    .into(),
            ),
            RmsError::Api { status: 503, .. } => {
                Some("A backing service is not initialised on the server (storage, sim, or agent).".into())
            }
            RmsError::Network(_) => Some(format!(
                "Is the NeoDEM server running? Try: curl {}",
                crate::config::health_probe_url()
            )),
            RmsError::Config(_) => Some(format!(
                "Run with --server <url>, set RMS_SERVER_URL, or create {}",
                crate::config::config_path_display()
            )),
            RmsError::Parse(_) => Some(
                "This usually means the server API changed. Re-run with --json to see the raw body."
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
        RmsError::Parse(e.to_string())
    }
}

pub type RmsResult<T> = Result<T, RmsError>;
