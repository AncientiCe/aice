//! On-prem facilitator MCP for a hotel, care home, or ward.
//!
//! The voice runtime calls this server. Every tool opens a staff ticket.
//! Tools named in `delegated_tools` also call the property's own MCP once.

mod auth;
mod cli;
mod desk;
mod http;
mod pack;
mod phone;
mod store;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use core_observability::{
    record_property_auth_attempt, record_property_mcp_duration, record_property_mcp_error,
    record_property_request,
};
use core_policy::decide_ward_tool;
use serde::Deserialize;
use serde_json::{json, Value};

pub use auth::{random_token, Role, MIN_PASSWORD_CHARS};
pub use cli::{run_pack, PASSWORD_ENV};
pub use core_policy::WARD_NON_CLINICAL_TOOLS as WARD_TOOLS;
pub use pack::Pack;
pub use phone::resolve_place;
pub use store::{AuditEvent, Ticket, TicketStatus};

use store::TicketStore;

#[derive(Debug, thiserror::Error)]
pub enum FacilitatorError {
    #[error("unknown pack '{0}'")]
    UnknownPack(String),
    #[error("pack file says {found}, this process is {expected}")]
    PackMismatch { expected: String, found: String },
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("lock poisoned")]
    LockPoisoned,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("http: {0}")]
    Http(String),
    #[error("bind {bind}: {message}")]
    Bind { bind: String, message: String },
    #[error("ticket {0} not found")]
    TicketNotFound(String),
    #[error("cannot change ticket from {from} to {to}")]
    InvalidTransition { from: String, to: String },
    #[error("unknown extension {0}")]
    UnknownExtension(String),
    #[error("request has no room or extension")]
    MissingPlace,
    #[error("auth: {0}")]
    Auth(String),
    #[error("user '{0}' already exists")]
    DuplicateUser(String),
    #[error("user '{0}' does not exist")]
    UnknownUser(String),
    #[error("service token: {0}")]
    ServiceToken(String),
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub pack: Pack,
    pub bind: String,
    pub database_path: PathBuf,
    pub property_mcp_url: Option<String>,
    /// Bearer token sent to the property's own MCP, when it needs one.
    pub property_mcp_token: Option<String>,
    pub delegated_tools: Vec<String>,
    pub extensions: HashMap<String, String>,
    /// Shared secret the voice backend presents on `/mcp`.
    pub service_token: String,
    /// Prometheus exporter bind, for example `127.0.0.1:9791`.
    pub metrics_bind: Option<String>,
}

/// Env var that overrides the service token file.
pub const SERVICE_TOKEN_ENV: &str = "AICE_PROPERTY_SERVICE_TOKEN";
/// Env var that overrides the property MCP token file.
pub const PROPERTY_MCP_TOKEN_ENV: &str = "AICE_PROPERTY_MCP_TOKEN";

#[derive(Debug, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    pack: Option<String>,
    #[serde(default)]
    bind: Option<String>,
    #[serde(default)]
    database_path: Option<String>,
    #[serde(default)]
    property_mcp_url: Option<String>,
    #[serde(default)]
    property_mcp_token_file: Option<String>,
    #[serde(default)]
    delegated_tools: Vec<String>,
    #[serde(default)]
    extensions: HashMap<String, String>,
    #[serde(default)]
    service_token_file: Option<String>,
    #[serde(default)]
    metrics_bind: Option<String>,
}

impl Settings {
    /// Load `property.json`. The service token comes from `AICE_PROPERTY_SERVICE_TOKEN`
    /// or `service_token_file` (default `service.token` beside the config); a missing
    /// file is created with a fresh random token so the backend can read it.
    pub fn load_or_default(pack: Pack, path: &Path) -> Result<Self, FacilitatorError> {
        let base_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        if !path.exists() {
            let mut settings = Self::default_for(pack);
            settings.database_path = base_dir.join("property.sqlite");
            settings.service_token = resolve_service_token(&base_dir.join("service.token"))?;
            return Ok(settings);
        }
        let raw = std::fs::read_to_string(path)?;
        let file: SettingsFile = serde_json::from_str(&raw)?;
        let found = match file.pack {
            Some(value) => Pack::parse(&value)?,
            None => pack,
        };
        if found != pack {
            return Err(FacilitatorError::PackMismatch {
                expected: pack.as_str().to_string(),
                found: found.as_str().to_string(),
            });
        }
        let bind = file
            .bind
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| pack.default_bind().to_string());
        let database_path = relative_to(
            &base_dir,
            file.database_path
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("property.sqlite"),
        );
        let token_path = relative_to(
            &base_dir,
            file.service_token_file
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("service.token"),
        );
        let property_mcp_token = match std::env::var(PROPERTY_MCP_TOKEN_ENV) {
            Ok(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
            _ => match file
                .property_mcp_token_file
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                Some(name) => Some(read_token_file(&relative_to(&base_dir, name))?),
                None => None,
            },
        };
        Ok(Self {
            pack,
            bind,
            database_path,
            property_mcp_url: file
                .property_mcp_url
                .filter(|value| !value.trim().is_empty()),
            property_mcp_token,
            delegated_tools: file.delegated_tools,
            extensions: file.extensions,
            service_token: resolve_service_token(&token_path)?,
            metrics_bind: file.metrics_bind.filter(|value| !value.trim().is_empty()),
        })
    }

    /// Defaults with a fresh in-memory service token.
    pub fn default_for(pack: Pack) -> Self {
        Self {
            pack,
            bind: pack.default_bind().to_string(),
            database_path: PathBuf::from("property.sqlite"),
            property_mcp_url: None,
            property_mcp_token: None,
            delegated_tools: Vec::new(),
            extensions: HashMap::new(),
            service_token: random_token(),
            metrics_bind: None,
        }
    }
}

fn relative_to(base: &Path, name: &str) -> PathBuf {
    let candidate = PathBuf::from(name);
    if candidate.is_absolute() {
        candidate
    } else {
        base.join(candidate)
    }
}

/// Read a token file; tokens shorter than 32 characters are refused.
pub fn read_token_file(path: &Path) -> Result<String, FacilitatorError> {
    let token = std::fs::read_to_string(path)
        .map_err(|error| FacilitatorError::ServiceToken(format!("{}: {error}", path.display())))?
        .trim()
        .to_string();
    if token.len() < 32 {
        return Err(FacilitatorError::ServiceToken(format!(
            "{} must hold at least 32 characters",
            path.display()
        )));
    }
    Ok(token)
}

/// Service token for a client of the facilitator: the env override, else the
/// token file. Clients never create the file; the pack does.
pub fn client_service_token(token_file: Option<&Path>) -> Result<String, FacilitatorError> {
    if let Ok(value) = std::env::var(SERVICE_TOKEN_ENV) {
        if !value.trim().is_empty() {
            return Ok(value.trim().to_string());
        }
    }
    match token_file {
        Some(path) => read_token_file(path),
        None => Err(FacilitatorError::ServiceToken(format!(
            "set property.service_token_file or {SERVICE_TOKEN_ENV}"
        ))),
    }
}

/// Env override, else read the token file, else create it.
pub fn resolve_service_token(path: &Path) -> Result<String, FacilitatorError> {
    if let Ok(value) = std::env::var(SERVICE_TOKEN_ENV) {
        if !value.trim().is_empty() {
            return Ok(value.trim().to_string());
        }
    }
    if path.exists() {
        return read_token_file(path);
    }
    let token = random_token();
    write_secret_file(path, &token)?;
    tracing::info!(path = %path.display(), "created property service token");
    Ok(token)
}

/// Write a new secret file readable only by the current user.
pub fn write_secret_file(path: &Path, contents: &str) -> Result<(), FacilitatorError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    std::io::Write::write_all(&mut file, contents.as_bytes())?;
    Ok(())
}

/// Create a desk account in the property database.
pub fn add_user(
    database_path: &Path,
    username: &str,
    password: &str,
    role: Role,
) -> Result<(), FacilitatorError> {
    let username = username.trim();
    auth::validate_new_user(username, password)?;
    let store = TicketStore::open(database_path)?;
    let hash = auth::hash_password(password)?;
    store.insert_user(username, &hash, role)
}

/// Delete a desk account and end its sessions.
pub fn remove_user(database_path: &Path, username: &str) -> Result<(), FacilitatorError> {
    TicketStore::open(database_path)?.remove_user(username.trim())
}

/// Desk accounts, sorted by name.
pub fn list_users(database_path: &Path) -> Result<Vec<(String, Role)>, FacilitatorError> {
    TicketStore::open(database_path)?.list_users()
}

#[derive(Clone, Debug)]
pub struct ToolResponse {
    pub ticket_id: String,
    pub status: String,
    pub spoken: String,
    pub is_error: bool,
}

pub struct Facilitator {
    settings: Settings,
    store: TicketStore,
    upstream_tools: Mutex<Vec<String>>,
    http: reqwest::Client,
}

impl Facilitator {
    pub fn open(settings: Settings) -> Result<Arc<Self>, FacilitatorError> {
        let store = TicketStore::open(&settings.database_path)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|error| FacilitatorError::Http(error.to_string()))?;
        Ok(Arc::new(Self {
            settings,
            store,
            upstream_tools: Mutex::new(Vec::new()),
            http,
        }))
    }

    pub async fn refresh_upstream_tools(self: &Arc<Self>) {
        let Some(url) = self.settings.property_mcp_url.clone() else {
            return;
        };
        let started = Instant::now();
        match list_upstream_tools(
            &self.http,
            &url,
            self.settings.property_mcp_token.as_deref(),
        )
        .await
        {
            Ok(names) => {
                record_property_mcp_duration("tools_list", started.elapsed());
                match self.upstream_tools.lock() {
                    Ok(mut guard) => *guard = names,
                    Err(_) => record_property_mcp_error("lock"),
                }
            }
            Err(error) => {
                record_property_mcp_duration("tools_list", started.elapsed());
                record_property_mcp_error("upstream_list");
                tracing::warn!(%error, "property MCP tools/list failed");
            }
        }
    }

    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .settings
            .pack
            .tools()
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        if let Ok(extra) = self.upstream_tools.lock() {
            for name in extra.iter() {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
        }
        names
    }

    pub fn list_tickets(&self) -> Result<Vec<Ticket>, FacilitatorError> {
        self.store.list()
    }

    /// Change a ticket's status on behalf of a desk user; the store audits it.
    pub fn set_status(
        &self,
        id: &str,
        status: TicketStatus,
        actor: &str,
    ) -> Result<Ticket, FacilitatorError> {
        self.store.set_status(id, status, actor)
    }

    pub fn list_audit(&self, limit: u32) -> Result<Vec<AuditEvent>, FacilitatorError> {
        self.store.list_audit(limit)
    }

    pub fn pack(&self) -> Pack {
        self.settings.pack
    }

    pub(crate) fn service_token_matches(&self, presented: &str) -> bool {
        auth::service_token_matches(&self.settings.service_token, presented)
    }

    pub(crate) fn has_users(&self) -> Result<bool, FacilitatorError> {
        self.store.has_users()
    }

    /// Check a desk login. Unknown users cost the same argon2 time as known ones.
    pub(crate) fn login(
        &self,
        username: &str,
        password: &str,
    ) -> Result<LoginOutcome, FacilitatorError> {
        let username = username.trim();
        let outcome = match self.store.login_state(username)? {
            store::LoginState::Locked => LoginOutcome::Locked,
            store::LoginState::Unknown => {
                auth::burn_password_check(password);
                LoginOutcome::Rejected
            }
            store::LoginState::Known { password_hash } => {
                if auth::verify_password(password, &password_hash) {
                    self.store.clear_login_failures(username)?;
                    let token = auth::random_token();
                    let csrf = auth::random_token();
                    let expires_at = store::unix_millis() + auth::SESSION_MILLIS;
                    self.store.create_session(
                        &auth::token_digest(&token),
                        username,
                        &csrf,
                        expires_at,
                    )?;
                    LoginOutcome::Accepted { token }
                } else {
                    // The failure that trips the lock still reads as a wrong password;
                    // the next attempt is refused as locked.
                    if self.store.record_login_failure(username)? {
                        tracing::warn!(username, "desk account locked after failed logins");
                    }
                    LoginOutcome::Rejected
                }
            }
        };
        record_property_auth_attempt("login", outcome.metric_label());
        Ok(outcome)
    }

    pub(crate) fn session(
        &self,
        token: &str,
    ) -> Result<Option<store::StaffSession>, FacilitatorError> {
        self.store.session(&auth::token_digest(token))
    }

    pub(crate) fn logout(&self, token: &str, username: &str) -> Result<(), FacilitatorError> {
        self.store
            .delete_session(&auth::token_digest(token), username)
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: &Value,
    ) -> Result<ToolResponse, FacilitatorError> {
        let started = Instant::now();
        let room_arg = arguments.get("room").and_then(Value::as_str);
        let extension = arguments.get("extension").and_then(Value::as_str);
        let place = match resolve_place(&self.settings.extensions, room_arg, extension) {
            Ok(place) => place,
            Err(error) => {
                record_property_mcp_error("place");
                record_property_mcp_duration("tools_call", started.elapsed());
                let ticket = self.store.insert(
                    self.settings.pack.as_str(),
                    "unassigned",
                    name,
                    &arguments.to_string(),
                    TicketStatus::Escalated,
                    &error.to_string(),
                )?;
                record_property_request(self.settings.pack.as_str(), name, ticket.status.as_str());
                return Ok(ToolResponse {
                    spoken: "I've alerted the staff. I could not tell which room this is."
                        .to_string(),
                    is_error: true,
                    status: ticket.status,
                    ticket_id: ticket.id,
                });
            }
        };

        if self.settings.pack == Pack::Ward {
            if let core_policy::PolicyDecision::Deny(reason) = decide_ward_tool(name) {
                record_property_mcp_error("policy_deny");
                record_property_mcp_duration("tools_call", started.elapsed());
                let ticket = self.store.insert(
                    self.settings.pack.as_str(),
                    &place,
                    name,
                    &arguments.to_string(),
                    TicketStatus::Escalated,
                    &reason,
                )?;
                record_property_request(self.settings.pack.as_str(), name, ticket.status.as_str());
                return Ok(ToolResponse {
                    spoken: format!("I've alerted the staff for room {place}."),
                    is_error: true,
                    status: ticket.status,
                    ticket_id: ticket.id,
                });
            }
        }

        let known = self.tool_names().iter().any(|tool| tool == name);
        if !known {
            record_property_mcp_error("unknown_tool");
            record_property_mcp_duration("tools_call", started.elapsed());
            let ticket = self.store.insert(
                self.settings.pack.as_str(),
                &place,
                name,
                &arguments.to_string(),
                TicketStatus::Escalated,
                "unknown tool",
            )?;
            record_property_request(self.settings.pack.as_str(), name, ticket.status.as_str());
            return Ok(ToolResponse {
                spoken: format!("I've alerted the staff for room {place}."),
                is_error: true,
                status: ticket.status,
                ticket_id: ticket.id,
            });
        }

        if self.settings.pack.always_escalates(name) {
            record_property_mcp_duration("tools_call", started.elapsed());
            let ticket = self.store.insert(
                self.settings.pack.as_str(),
                &place,
                name,
                &arguments.to_string(),
                TicketStatus::Escalated,
                "mandatory escalation",
            )?;
            record_property_request(self.settings.pack.as_str(), name, ticket.status.as_str());
            return Ok(ToolResponse {
                spoken: format!("I've alerted the staff for room {place}."),
                is_error: false,
                status: ticket.status,
                ticket_id: ticket.id,
            });
        }

        let delegated = self
            .settings
            .delegated_tools
            .iter()
            .any(|tool| tool == name);
        if delegated {
            let Some(url) = self.settings.property_mcp_url.clone() else {
                record_property_mcp_error("missing_upstream");
                record_property_mcp_duration("tools_call", started.elapsed());
                let ticket = self.store.insert(
                    self.settings.pack.as_str(),
                    &place,
                    name,
                    &arguments.to_string(),
                    TicketStatus::Escalated,
                    "delegated tool has no property MCP url",
                )?;
                record_property_request(self.settings.pack.as_str(), name, ticket.status.as_str());
                return Ok(ToolResponse {
                    spoken: format!("I've alerted the staff for room {place}."),
                    is_error: true,
                    status: ticket.status,
                    ticket_id: ticket.id,
                });
            };
            let upstream_started = Instant::now();
            match call_upstream(
                &self.http,
                &url,
                self.settings.property_mcp_token.as_deref(),
                name,
                arguments,
            )
            .await
            {
                Ok(text) => {
                    record_property_mcp_duration("upstream", upstream_started.elapsed());
                    record_property_mcp_duration("tools_call", started.elapsed());
                    let detail = format!("delegated: {text}");
                    let ticket = self.store.insert(
                        self.settings.pack.as_str(),
                        &place,
                        name,
                        &arguments.to_string(),
                        TicketStatus::Open,
                        &detail,
                    )?;
                    record_property_request(
                        self.settings.pack.as_str(),
                        name,
                        ticket.status.as_str(),
                    );
                    let spoken = if text.trim().is_empty() {
                        format!("I've logged {name} for room {place}.")
                    } else {
                        text
                    };
                    return Ok(ToolResponse {
                        spoken,
                        is_error: false,
                        status: ticket.status,
                        ticket_id: ticket.id,
                    });
                }
                Err(error) => {
                    record_property_mcp_duration("upstream", upstream_started.elapsed());
                    record_property_mcp_duration("tools_call", started.elapsed());
                    record_property_mcp_error("upstream");
                    tracing::warn!(%error, tool = name, "property MCP tools/call failed");
                    let ticket = self.store.insert(
                        self.settings.pack.as_str(),
                        &place,
                        name,
                        &arguments.to_string(),
                        TicketStatus::Escalated,
                        &error.to_string(),
                    )?;
                    record_property_request(
                        self.settings.pack.as_str(),
                        name,
                        ticket.status.as_str(),
                    );
                    return Ok(ToolResponse {
                        spoken: format!("I've alerted the staff for room {place}."),
                        is_error: true,
                        status: ticket.status,
                        ticket_id: ticket.id,
                    });
                }
            }
        }

        record_property_mcp_duration("tools_call", started.elapsed());
        let ticket = self.store.insert(
            self.settings.pack.as_str(),
            &place,
            name,
            &arguments.to_string(),
            TicketStatus::Open,
            "logged on the staff desk",
        )?;
        record_property_request(self.settings.pack.as_str(), name, ticket.status.as_str());
        Ok(ToolResponse {
            spoken: format!("I've logged {name} for room {place}."),
            is_error: false,
            status: ticket.status,
            ticket_id: ticket.id,
        })
    }

    pub async fn handle_rpc(&self, body: &Value) -> Value {
        let method = body.get("method").and_then(Value::as_str).unwrap_or("");
        let id = body.get("id").cloned().unwrap_or(Value::Null);
        match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": format!("aice-{}", self.settings.pack.as_str()),
                        "version": "0.1.0"
                    }
                }
            }),
            "notifications/initialized" | "ping" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {}
            }),
            "tools/list" => {
                let tools: Vec<Value> = self
                    .tool_names()
                    .into_iter()
                    .map(|name| {
                        json!({
                            "name": name,
                            "description": format!("Property request {name}"),
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "room": { "type": "string" },
                                    "extension": { "type": "string" }
                                },
                                "additionalProperties": true
                            }
                        })
                    })
                    .collect();
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": tools }
                })
            }
            "tools/call" => {
                let params = body.get("params").cloned().unwrap_or_else(|| json!({}));
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if name.is_empty() {
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32602, "message": "tools/call requires a name" }
                    });
                }
                match self.call_tool(&name, &arguments).await {
                    Ok(response) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": response.spoken }],
                            "isError": response.is_error,
                            "structuredContent": {
                                "ticket_id": response.ticket_id,
                                "status": response.status
                            }
                        }
                    }),
                    Err(error) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32000, "message": error.to_string() }
                    }),
                }
            }
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not found" }
            }),
        }
    }
}

pub struct Running {
    pub url: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Running {
    pub async fn until_ctrl_c(mut self) -> Result<(), FacilitatorError> {
        let _ = tokio::signal::ctrl_c().await;
        self.stop().await;
        Ok(())
    }

    async fn stop(&mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub async fn serve(settings: Settings) -> Result<Running, FacilitatorError> {
    let facilitator = Facilitator::open(settings)?;
    facilitator.refresh_upstream_tools().await;
    http::listen(facilitator).await
}

pub struct FixedMcp {
    pub url: String,
    pub calls: std::sync::Arc<std::sync::atomic::AtomicU64>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for FixedMcp {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub async fn serve_fixed_mcp(
    tools: Vec<String>,
    text: String,
) -> Result<FixedMcp, FacilitatorError> {
    http::listen_fixed(tools, text).await
}

/// Result of a desk login attempt.
pub(crate) enum LoginOutcome {
    Accepted { token: String },
    Rejected,
    Locked,
}

impl LoginOutcome {
    fn metric_label(&self) -> &'static str {
        match self {
            Self::Accepted { .. } => "accepted",
            Self::Rejected => "rejected",
            Self::Locked => "locked",
        }
    }
}

/// Backend-side client for the facilitator MCP.
pub struct PropertyClient {
    http: reqwest::Client,
    mcp_url: String,
    service_token: String,
}

impl PropertyClient {
    pub fn new(
        mcp_url: impl Into<String>,
        service_token: impl Into<String>,
    ) -> Result<Self, FacilitatorError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|error| FacilitatorError::Http(error.to_string()))?;
        Ok(Self {
            http,
            mcp_url: mcp_url.into(),
            service_token: service_token.into(),
        })
    }

    pub async fn list_tools(&self) -> Result<Vec<String>, FacilitatorError> {
        list_upstream_tools(&self.http, &self.mcp_url, Some(&self.service_token)).await
    }

    pub async fn call_tool(
        &self,
        name: &str,
        room: Option<&str>,
        extension: Option<&str>,
        arguments: &Value,
    ) -> Result<ToolResponse, FacilitatorError> {
        let mut payload = if arguments.is_object() {
            arguments.clone()
        } else {
            json!({})
        };
        if let Some(object) = payload.as_object_mut() {
            if let Some(room) = room {
                object.insert("room".to_string(), Value::String(room.to_string()));
            }
            if let Some(extension) = extension {
                object.insert(
                    "extension".to_string(),
                    Value::String(extension.to_string()),
                );
            }
        }
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": payload }
        });
        let value = post_rpc(&self.http, &self.mcp_url, Some(&self.service_token), &body).await?;
        let result = value.get("result").cloned().unwrap_or_else(|| json!({}));
        let structured = result
            .get("structuredContent")
            .cloned()
            .unwrap_or_else(|| json!({}));
        Ok(ToolResponse {
            ticket_id: structured
                .get("ticket_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            status: structured
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            spoken: first_text(&result),
            is_error: result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

/// POST one JSON-RPC request; HTTP and JSON-RPC errors both become `Err`.
async fn post_rpc(
    http: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    body: &Value,
) -> Result<Value, FacilitatorError> {
    let mut request = http.post(url).json(body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|error| FacilitatorError::Http(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(FacilitatorError::Http(format!("{url} returned {status}")));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|error| FacilitatorError::Http(error.to_string()))?;
    if let Some(message) = value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
    {
        return Err(FacilitatorError::Http(message.to_string()));
    }
    Ok(value)
}

fn first_text(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

async fn list_upstream_tools(
    http: &reqwest::Client,
    url: &str,
    token: Option<&str>,
) -> Result<Vec<String>, FacilitatorError> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });
    let value = post_rpc(http, url, token, &body).await?;
    let tools = value
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut names = Vec::new();
    for tool in tools {
        if let Some(name) = tool.get("name").and_then(Value::as_str) {
            if !name.is_empty() {
                names.push(name.to_string());
            }
        }
    }
    Ok(names)
}

async fn call_upstream(
    http: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    name: &str,
    arguments: &Value,
) -> Result<String, FacilitatorError> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    });
    let value = post_rpc(http, url, token, &body).await?;
    Ok(first_text(value.get("result").unwrap_or(&Value::Null)))
}
