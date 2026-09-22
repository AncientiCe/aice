//! On-prem facilitator MCP for a hotel, care home, or ward.
//!
//! The voice runtime calls this server. Every tool opens a staff ticket.
//! Tools named in `delegated_tools` also call the property's own MCP once.

mod http;
mod pack;
mod phone;
mod store;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use core_observability::{
    record_property_mcp_duration, record_property_mcp_error, record_property_request,
};
use core_policy::decide_ward_tool;
use serde::Deserialize;
use serde_json::{json, Value};

pub use core_policy::WARD_NON_CLINICAL_TOOLS as WARD_TOOLS;
pub use pack::Pack;
pub use phone::resolve_place;
pub use store::{Ticket, TicketStatus};

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
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub pack: Pack,
    pub bind: String,
    pub database_path: PathBuf,
    pub property_mcp_url: Option<String>,
    pub delegated_tools: Vec<String>,
    pub extensions: HashMap<String, String>,
}

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
    delegated_tools: Vec<String>,
    #[serde(default)]
    extensions: HashMap<String, String>,
}

impl Settings {
    pub fn load_or_default(pack: Pack, path: &Path) -> Result<Self, FacilitatorError> {
        if !path.exists() {
            return Ok(Self::default_for(pack));
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
        let database_path = file
            .database_path
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("property.sqlite"));
        Ok(Self {
            pack,
            bind,
            database_path,
            property_mcp_url: file
                .property_mcp_url
                .filter(|value| !value.trim().is_empty()),
            delegated_tools: file.delegated_tools,
            extensions: file.extensions,
        })
    }

    pub fn default_for(pack: Pack) -> Self {
        Self {
            pack,
            bind: pack.default_bind().to_string(),
            database_path: PathBuf::from("property.sqlite"),
            property_mcp_url: None,
            delegated_tools: Vec::new(),
            extensions: HashMap::new(),
        }
    }
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
        match list_upstream_tools(&self.http, &url).await {
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

    pub fn set_status(&self, id: &str, status: TicketStatus) -> Result<Ticket, FacilitatorError> {
        self.store.set_status(id, status)
    }

    pub fn desk_html(&self) -> Result<String, FacilitatorError> {
        let tickets = self.store.list()?;
        let mut rows = String::new();
        for ticket in &tickets {
            let room = escape_html(&ticket.room);
            let tool = escape_html(&ticket.tool_name);
            let status = escape_html(&ticket.status);
            let id = escape_html(&ticket.id);
            rows.push_str(&format!(
                "<tr><td>{room}</td><td>{tool}</td><td>{status}</td><td>\
                <form method=\"post\" action=\"/api/tickets/{id}/acknowledge\"><button>Acknowledge</button></form>\
                <form method=\"post\" action=\"/api/tickets/{id}/done\"><button>Done</button></form>\
                <form method=\"post\" action=\"/api/tickets/{id}/escalate\"><button>Escalate</button></form>\
                </td></tr>"
            ));
        }
        Ok(format!(
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{}</title></head><body>\
            <h1>{}</h1><table><tr><th>Room</th><th>Request</th><th>Status</th><th></th></tr>{}</table></body></html>",
            escape_html(self.settings.pack.desk_title()),
            escape_html(self.settings.pack.desk_title()),
            rows
        ))
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
            match call_upstream(&self.http, &url, name, arguments).await {
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

pub struct PropertyClient {
    http: reqwest::Client,
    mcp_url: String,
}

impl PropertyClient {
    pub fn new(mcp_url: impl Into<String>) -> Result<Self, FacilitatorError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|error| FacilitatorError::Http(error.to_string()))?;
        Ok(Self {
            http,
            mcp_url: mcp_url.into(),
        })
    }

    pub async fn list_tools(&self) -> Result<Vec<String>, FacilitatorError> {
        list_upstream_tools(&self.http, &self.mcp_url).await
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
        let response = self
            .http
            .post(&self.mcp_url)
            .json(&body)
            .send()
            .await
            .map_err(|error| FacilitatorError::Http(error.to_string()))?;
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
        let result = value.get("result").cloned().unwrap_or_else(|| json!({}));
        let spoken = result
            .get("content")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
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
            spoken,
            is_error: result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

async fn list_upstream_tools(
    http: &reqwest::Client,
    url: &str,
) -> Result<Vec<String>, FacilitatorError> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    });
    let response = http
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|error| FacilitatorError::Http(error.to_string()))?;
    let value: Value = response
        .json()
        .await
        .map_err(|error| FacilitatorError::Http(error.to_string()))?;
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
    name: &str,
    arguments: &Value,
) -> Result<String, FacilitatorError> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    });
    let response = http
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|error| FacilitatorError::Http(error.to_string()))?;
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
    let text = value
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(text)
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
