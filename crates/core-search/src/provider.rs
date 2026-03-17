//! Production HTTP search provider; call only after user confirms.

use crate::{ExternalSearch, SearchError};
use async_trait::async_trait;
use std::time::Duration;
use url::Url;

/// HTTP client that performs a GET request to a configurable URL with query parameter.
/// Use for fallback web search after user confirms.
#[derive(Clone)]
pub struct HttpSearchProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl HttpSearchProvider {
    /// Build a provider. `base_url` should be the endpoint (e.g. https://api.example.com/search);
    /// the query is appended as `?q=<query>`. `timeout_secs` is the request timeout.
    pub fn new(
        base_url: String,
        api_key: Option<String>,
        timeout_secs: u64,
    ) -> Result<Self, SearchError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| SearchError::Request(e.to_string()))?;
        Ok(Self {
            client,
            base_url,
            api_key,
        })
    }

    /// Build from a config-like tuple (url, api_key, timeout_secs).
    pub fn from_options(
        url: &str,
        api_key: Option<&str>,
        timeout_secs: u64,
    ) -> Result<Self, SearchError> {
        Self::new(url.to_string(), api_key.map(String::from), timeout_secs)
    }
}

#[async_trait]
impl ExternalSearch for HttpSearchProvider {
    async fn execute(&self, query: &str) -> Result<String, SearchError> {
        let url = Url::parse_with_params(&self.base_url, &[("q", query)])
            .map_err(|e| SearchError::Request(e.to_string()))?;
        let mut req = self.client.get(url);
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| SearchError::Request(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(SearchError::Request(format!(
                "search API returned {}",
                resp.status()
            )));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| SearchError::Request(e.to_string()))?;
        Ok(text)
    }
}
