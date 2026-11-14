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
    /// reqwest's own text is "error sending request for url (…)" whatever went
    /// wrong; the cause — a DNS failure, a refused connection, a TLS error —
    /// is in the source chain, so it is appended.
    fn from(e: reqwest::Error) -> Self {
        let mut message = e.to_string();
        let mut source = std::error::Error::source(&e);
        while let Some(cause) = source {
            let text = cause.to_string();
            if !text.is_empty() && !message.contains(&text) {
                message.push_str(": ");
                message.push_str(&text);
            }
            source = cause.source();
        }
        RmsError::Network(message)
    }
}

impl From<serde_json::Error> for RmsError {
    fn from(e: serde_json::Error) -> Self {
        RmsError::Parse(e.to_string())
    }
}

pub type RmsResult<T> = Result<T, RmsError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_network_error_says_why_the_request_failed() {
        // Nothing listens on the discard port of the loopback interface here:
        // the connection is refused, and the message has to say so rather
        // than only "error sending request".
        let e = reqwest::get("http://127.0.0.1:9/health").await.unwrap_err();
        let RmsError::Network(message) = RmsError::from(e) else {
            panic!("not a network error");
        };
        assert!(message.contains("127.0.0.1:9"), "{message}");
        assert!(
            message.to_lowercase().contains("refused"),
            "the cause is missing: {message}"
        );
    }
}
