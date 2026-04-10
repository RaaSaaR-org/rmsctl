use crate::config::RmsConfig;
use crate::error::{RmsError, RmsResult};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Serialize;

pub struct ApiClient {
    http: Client,
    base_url: String,
    token: Option<String>,
}

#[derive(serde::Deserialize)]
struct ErrorBody {
    error: String,
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

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(t) => req.bearer_auth(t),
            None => req,
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> RmsResult<T> {
        let req = self.http.get(self.url(path));
        self.send(self.apply_auth(req)).await
    }

    pub async fn get_with_query<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> RmsResult<T> {
        let req = self.http.get(self.url(path)).query(query);
        self.send(self.apply_auth(req)).await
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> RmsResult<T> {
        let req = self.http.post(self.url(path)).json(body);
        self.send(self.apply_auth(req)).await
    }

    pub async fn patch<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> RmsResult<T> {
        let req = self.http.patch(self.url(path)).json(body);
        self.send(self.apply_auth(req)).await
    }

    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> RmsResult<T> {
        let req = self.http.delete(self.url(path));
        self.send(self.apply_auth(req)).await
    }

    async fn send<T: DeserializeOwned>(&self, req: reqwest::RequestBuilder) -> RmsResult<T> {
        let resp = req.send().await?;
        let status = resp.status();

        if status.is_success() {
            let body = resp.text().await?;
            serde_json::from_str(&body).map_err(|e| {
                RmsError::Other(format!("Failed to parse response: {e}\nBody: {body}"))
            })
        } else {
            let code = status.as_u16();
            let body = resp.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ErrorBody>(&body)
                .map(|e| e.error)
                .unwrap_or(body);
            Err(RmsError::Api {
                status: code,
                message,
            })
        }
    }
}
