use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use property_facilitator::{Pack, Settings};
use reqwest::redirect::Policy;
use reqwest::StatusCode;
use serde_json::Value;

pub const TOKEN: &str = "test-service-token-0123456789abcdef";
pub const PASSWORD: &str = "correct horse battery";

pub fn temp_db(label: &str) -> PathBuf {
    let nanos = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos(),
        Err(_) => 0,
    };
    let mut path = std::env::temp_dir();
    path.push(format!(
        "aice-property-{label}-{}-{nanos}.sqlite",
        std::process::id()
    ));
    path
}

pub fn settings(pack: Pack, db: PathBuf) -> Settings {
    let mut settings = Settings::default_for(pack);
    settings.bind = "127.0.0.1:0".to_string();
    settings.database_path = db;
    settings.service_token = TOKEN.to_string();
    settings
}

pub fn hotels(db: PathBuf, upstream: Option<String>, delegated: Vec<String>) -> Settings {
    let mut settings = settings(Pack::Hotels, db);
    settings.property_mcp_url = upstream;
    settings.delegated_tools = delegated;
    settings.extensions = [("101".to_string(), "204".to_string())]
        .into_iter()
        .collect();
    settings
}

pub fn client() -> reqwest::Client {
    match reqwest::Client::builder().redirect(Policy::none()).build() {
        Ok(client) => client,
        Err(error) => panic!("http client: {error}"),
    }
}

pub async fn post_mcp_with(base: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut request = client().post(format!("{base}/mcp")).json(&body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => panic!("mcp post failed: {error}"),
    };
    let status = response.status();
    let value = response.json().await.unwrap_or(Value::Null);
    (status, value)
}

pub async fn post_mcp(base: &str, body: Value) -> Value {
    let (status, value) = post_mcp_with(base, Some(TOKEN), body).await;
    assert_eq!(status, StatusCode::OK, "mcp call failed: {value}");
    value
}

pub async fn call_tool(base: &str, name: &str, arguments: Value) -> Value {
    post_mcp(
        base,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }),
    )
    .await
}

/// A logged-in desk browser: session cookie plus the CSRF token from the desk page.
pub struct Desk {
    pub base: String,
    pub cookie: String,
    pub csrf: String,
}

pub async fn try_login(base: &str, username: &str, password: &str) -> reqwest::Response {
    match client()
        .post(format!("{base}/login"))
        .form(&[("username", username), ("password", password)])
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => panic!("login post failed: {error}"),
    }
}

pub async fn login(base: &str, username: &str, password: &str) -> Desk {
    let response = try_login(base, username, password).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER, "login rejected");
    let cookie = response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or("")
        .to_string();
    assert!(cookie.starts_with("aice_desk="), "missing session cookie");
    let desk = Desk {
        base: base.to_string(),
        cookie,
        csrf: String::new(),
    };
    let html = desk.page("/desk").await;
    let csrf = html
        .split("name=\"csrf\" value=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or("")
        .to_string();
    assert!(!csrf.is_empty(), "desk page has no csrf token");
    Desk { csrf, ..desk }
}

impl Desk {
    pub async fn page(&self, path: &str) -> String {
        match client()
            .get(format!("{}{path}", self.base))
            .header(reqwest::header::COOKIE, &self.cookie)
            .send()
            .await
        {
            Ok(response) => {
                assert_eq!(response.status(), StatusCode::OK, "GET {path}");
                response.text().await.unwrap_or_default()
            }
            Err(error) => panic!("GET {path}: {error}"),
        }
    }

    pub async fn get_json(&self, path: &str) -> Value {
        match client()
            .get(format!("{}{path}", self.base))
            .header(reqwest::header::COOKIE, &self.cookie)
            .send()
            .await
        {
            Ok(response) => {
                assert_eq!(response.status(), StatusCode::OK, "GET {path}");
                response.json().await.unwrap_or(Value::Null)
            }
            Err(error) => panic!("GET {path}: {error}"),
        }
    }

    /// Submit a desk form (browser style) and return the status code.
    pub async fn form(&self, path: &str, csrf: Option<&str>) -> StatusCode {
        let fields: Vec<(&str, &str)> = match csrf {
            Some(value) => vec![("csrf", value)],
            None => Vec::new(),
        };
        match client()
            .post(format!("{}{path}", self.base))
            .header(reqwest::header::COOKIE, &self.cookie)
            .form(&fields)
            .send()
            .await
        {
            Ok(response) => response.status(),
            Err(error) => panic!("POST {path}: {error}"),
        }
    }

    /// Call the JSON API with the CSRF header.
    pub async fn api_post(&self, path: &str) -> (StatusCode, Value) {
        match client()
            .post(format!("{}{path}", self.base))
            .header(reqwest::header::COOKIE, &self.cookie)
            .header("x-csrf-token", &self.csrf)
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status();
                (status, response.json().await.unwrap_or(Value::Null))
            }
            Err(error) => panic!("POST {path}: {error}"),
        }
    }
}

pub fn ticket_id(created: &Value) -> String {
    created["result"]["structuredContent"]["ticket_id"]
        .as_str()
        .unwrap_or("")
        .to_string()
}
