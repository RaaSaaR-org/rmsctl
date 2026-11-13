use crate::config::RmsConfig;
use crate::error::{RmsError, RmsResult};
use reqwest::{Client, Method};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub struct ApiClient {
    http: Client,
    base_url: String,
    token: Option<String>,
}

/// The server is not consistent about which key carries the failure text:
/// most routes send `{"error": "..."}`, a few add a longer `{"message": "..."}`.
#[derive(serde::Deserialize)]
struct ErrorBody {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

/// A response the caller interprets itself: the status, the server's
/// `Retry-After` and the untouched body.
///
/// The typed verbs below collapse every refusal into `RmsError::Api`, which is
/// right for a one-shot command. The dataset upload has to branch on the
/// server's stable `code` (retry `UPLOAD_BUSY`, list what `UPLOAD_INCOMPLETE`
/// is missing), so it gets the whole answer.
#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub retry_after: Option<u64>,
    pub body: String,
}

impl RawResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as JSON; `null` when it is empty or not JSON (a proxy's HTML
    /// error page, for one).
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    /// The server's stable error code, e.g. `UPLOAD_BUSY`.
    pub fn code(&self) -> Option<String> {
        self.json()
            .get("code")
            .and_then(|c| c.as_str())
            .map(String::from)
    }
}

impl ApiClient {
    pub fn new(config: &RmsConfig) -> Self {
        Self {
            http: Client::new(),
            base_url: config.server_url.trim_end_matches('/').to_string(),
            token: config.token.clone(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// The base URL this client talks to, without a trailing slash.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// An authenticated request the caller finishes (headers, body, timeout)
    /// and hands to `send_raw`.
    pub fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        self.build(method, path)
    }

    /// Send a request and return whatever the server said, refusals included.
    /// Only a transport failure is an `Err`.
    pub async fn send_raw(&self, req: reqwest::RequestBuilder) -> RmsResult<RawResponse> {
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let body = resp.text().await?;
        Ok(RawResponse {
            status,
            retry_after,
            body,
        })
    }

    fn build(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let req = self.http.request(method, self.url(path));
        match &self.token {
            Some(t) => req.bearer_auth(t),
            None => req,
        }
    }

    // ── Typed requests ──────────────────────────────────────────────────

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> RmsResult<T> {
        self.send(self.build(Method::GET, path)).await
    }

    pub async fn get_query<T: DeserializeOwned, Q: Serialize + ?Sized>(
        &self,
        path: &str,
        query: &Q,
    ) -> RmsResult<T> {
        self.send(self.build(Method::GET, path).query(query)).await
    }

    pub async fn post<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> RmsResult<T> {
        self.send(self.build(Method::POST, path).json(body)).await
    }

    // ── Raw passthrough ─────────────────────────────────────────────────
    //
    // `--json` must hand the caller exactly what the server said. Routing it
    // through a typed struct silently drops every field the struct omits, so
    // these return the untouched body instead.

    pub async fn get_value(&self, path: &str) -> RmsResult<Value> {
        self.send(self.build(Method::GET, path)).await
    }

    pub async fn get_value_query<Q: Serialize + ?Sized>(
        &self,
        path: &str,
        query: &Q,
    ) -> RmsResult<Value> {
        self.send(self.build(Method::GET, path).query(query)).await
    }

    pub async fn post_value<B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> RmsResult<Value> {
        self.send(self.build(Method::POST, path).json(body)).await
    }

    pub async fn patch_value<B: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> RmsResult<Value> {
        self.send(self.build(Method::PATCH, path).json(body)).await
    }

    pub async fn put_value<B: Serialize + ?Sized>(&self, path: &str, body: &B) -> RmsResult<Value> {
        self.send(self.build(Method::PUT, path).json(body)).await
    }

    pub async fn delete_value(&self, path: &str) -> RmsResult<Value> {
        self.send(self.build(Method::DELETE, path)).await
    }

    async fn send<T: DeserializeOwned>(&self, req: reqwest::RequestBuilder) -> RmsResult<T> {
        let resp = req.send().await?;
        let status = resp.status();
        let body = resp.text().await?;

        if !status.is_success() {
            let message = serde_json::from_str::<ErrorBody>(&body)
                .ok()
                .and_then(|e| e.error.or(e.message))
                .unwrap_or_else(|| {
                    if body.trim().is_empty() {
                        status.canonical_reason().unwrap_or("request failed").into()
                    } else {
                        body.clone()
                    }
                });
            return Err(RmsError::Api {
                status: status.as_u16(),
                message,
            });
        }

        // 204 and bare-`OK` handlers send no body; `null` deserializes into
        // Value and Option<T> so those callers do not have to special-case it.
        let text = if body.trim().is_empty() {
            "null"
        } else {
            &body
        };

        serde_json::from_str(text).map_err(|e| {
            RmsError::Parse(format!(
                "{e}\n  The server sent a shape this command does not understand.\n  Body: {}",
                crate::output::clip(&body, 600)
            ))
        })
    }
}
