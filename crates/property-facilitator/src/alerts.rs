//! Staff alerting and SLA re-escalation.
//!
//! Every ticket whose tool matches a rule is armed when it is created. A
//! database-driven loop sends the first alert to tier 0, and each time the
//! rule's acknowledgement window passes without an acknowledgement it marks
//! the ticket escalated and pages the next tier. Because the schedule lives
//! in SQLite, a restart picks up exactly where the last process stopped.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use core_observability::{record_property_alert_sent, record_property_sla_breach};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::store::DueAlert;
use crate::{Facilitator, FacilitatorError, Pack};

/// How often the loop looks for due alerts.
const TICK: Duration = Duration::from_millis(100);
/// Retry delay when every channel for a tier failed.
const RETRY_AFTER: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct AlertSettings {
    /// Escalation order, lowest first.
    pub tiers: Vec<String>,
    pub channels: Vec<AlertChannel>,
    /// First matching rule wins; `"*"` matches every tool.
    pub rules: Vec<AlertRule>,
}

#[derive(Clone, Debug)]
pub struct AlertChannel {
    pub name: String,
    pub kind: AlertChannelKind,
    /// Tiers this channel is paged for.
    pub tiers: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum AlertChannelKind {
    /// POST the alert as JSON (pagers, SMS gateways, chat, on-call services).
    Webhook { url: String, bearer: Option<String> },
    /// Call a tool on the property's own MCP (nurse call, DECT, PMS).
    PropertyMcp { tool: String },
}

#[derive(Clone, Debug)]
pub struct AlertRule {
    pub tools: Vec<String>,
    pub ack_within: Duration,
}

impl AlertSettings {
    /// Pack defaults: no channels (the desk still shows escalations), three tiers,
    /// and acknowledgement windows sized to each setting.
    pub fn default_for(pack: Pack) -> Self {
        let rule = |tools: &[&str], secs: u64| AlertRule {
            tools: tools.iter().map(|tool| tool.to_string()).collect(),
            ack_within: Duration::from_secs(secs),
        };
        let rules = match pack {
            Pack::Care => vec![
                rule(&["report_fall", "report_distress"], 60),
                rule(&["request_bathroom_help", "report_pain"], 180),
                rule(&["*"], 600),
            ],
            Pack::Ward => vec![rule(&["get_a_nurse"], 120), rule(&["*"], 900)],
            Pack::Hotels => vec![rule(&["*"], 900)],
        };
        Self {
            tiers: vec![
                "staff".to_string(),
                "supervisor".to_string(),
                "on_call".to_string(),
            ],
            channels: Vec::new(),
            rules,
        }
    }

    /// Acknowledgement window for `tool`, or `None` when no rule applies.
    pub fn ack_within(&self, tool: &str) -> Option<Duration> {
        self.rules
            .iter()
            .find(|rule| rule.tools.iter().any(|name| name == "*" || name == tool))
            .map(|rule| rule.ack_within)
    }

    fn tier_name(&self, tier: usize) -> &str {
        self.tiers
            .get(tier)
            .or_else(|| self.tiers.last())
            .map(String::as_str)
            .unwrap_or("staff")
    }

    fn last_tier(&self) -> usize {
        self.tiers.len().saturating_sub(1)
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct AlertsFile {
    #[serde(default)]
    tiers: Option<Vec<String>>,
    #[serde(default)]
    channels: Vec<ChannelFile>,
    #[serde(default)]
    rules: Option<Vec<RuleFile>>,
}

#[derive(Debug, Deserialize)]
struct ChannelFile {
    name: String,
    kind: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    token_file: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    tiers: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RuleFile {
    tools: Vec<String>,
    ack_within_secs: u64,
}

pub(crate) fn resolve_alerts(
    pack: Pack,
    base_dir: &Path,
    file: Option<AlertsFile>,
) -> Result<AlertSettings, FacilitatorError> {
    let mut settings = AlertSettings::default_for(pack);
    let Some(file) = file else {
        return Ok(settings);
    };
    if let Some(tiers) = file.tiers.filter(|tiers| !tiers.is_empty()) {
        settings.tiers = tiers;
    }
    if let Some(rules) = file.rules {
        settings.rules = rules
            .into_iter()
            .map(|rule| AlertRule {
                tools: rule.tools,
                ack_within: Duration::from_secs(rule.ack_within_secs.max(1)),
            })
            .collect();
    }
    for channel in file.channels {
        let invalid = |message: &str| {
            FacilitatorError::Alert(format!("channel '{}': {message}", channel.name))
        };
        if let Some(unknown) = channel
            .tiers
            .iter()
            .find(|tier| !settings.tiers.contains(tier))
        {
            return Err(invalid(&format!("unknown tier '{unknown}'")));
        }
        let kind = match channel.kind.as_str() {
            "webhook" => AlertChannelKind::Webhook {
                url: channel
                    .url
                    .clone()
                    .filter(|url| !url.trim().is_empty())
                    .ok_or_else(|| invalid("webhook needs a url"))?,
                bearer: match channel.token_file.as_deref() {
                    Some(name) => {
                        Some(crate::read_token_file(&crate::relative_to(base_dir, name))?)
                    }
                    None => None,
                },
            },
            "property_mcp" => AlertChannelKind::PropertyMcp {
                tool: channel
                    .tool
                    .clone()
                    .filter(|tool| !tool.trim().is_empty())
                    .ok_or_else(|| invalid("property_mcp needs a tool"))?,
            },
            other => {
                return Err(invalid(&format!(
                    "kind must be webhook or property_mcp, got '{other}'"
                )))
            }
        };
        settings.channels.push(AlertChannel {
            name: channel.name,
            kind,
            tiers: channel.tiers,
        });
    }
    Ok(settings)
}

/// Run the alert loop until the returned task is aborted.
pub(crate) fn spawn(facilitator: Arc<Facilitator>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = run_due(&facilitator).await {
                tracing::warn!(%error, "alert pass failed");
            }
        }
    })
}

async fn run_due(facilitator: &Arc<Facilitator>) -> Result<(), FacilitatorError> {
    let due = facilitator.store.due_alerts()?;
    for alert in due {
        handle(facilitator, alert).await?;
    }
    Ok(())
}

async fn handle(facilitator: &Arc<Facilitator>, alert: DueAlert) -> Result<(), FacilitatorError> {
    let settings = &facilitator.settings.alerts;
    let Some(ack_within) = settings.ack_within(&alert.ticket.tool_name) else {
        facilitator.store.disarm_alert(&alert.ticket.id)?;
        return Ok(());
    };
    let (tier, reason) = match alert.tier {
        None => (0, "new"),
        Some(current) => {
            let next = (current + 1).min(settings.last_tier());
            facilitator
                .store
                .record_sla_breach(&alert.ticket.id, settings.tier_name(next))?;
            record_property_sla_breach(&alert.ticket.pack, &alert.ticket.tool_name);
            (next, "sla_breach")
        }
    };
    let payload = json!({
        "ticket_id": alert.ticket.id,
        "pack": alert.ticket.pack,
        "room": alert.ticket.room,
        "tool": alert.ticket.tool_name,
        "status": if reason == "sla_breach" { "escalated" } else { alert.ticket.status.as_str() },
        "tier": settings.tier_name(tier),
        "reason": reason,
        "created_millis": alert.ticket.created_millis,
        "ack_within_secs": ack_within.as_secs(),
    });
    let delivered = deliver(facilitator, settings.tier_name(tier), &payload).await;
    let next_in = if delivered { ack_within } else { RETRY_AFTER };
    // A tier-0 alert that reached nobody is retried as `new`; a breach always
    // advances so the next pass pages the tier above.
    let stored_tier = if delivered || reason == "sla_breach" {
        Some(tier)
    } else {
        None
    };
    facilitator
        .store
        .rearm_alert(&alert.ticket.id, stored_tier, next_in)?;
    Ok(())
}

/// Send to every channel of `tier`. True when at least one channel took it,
/// or when no channel is configured for the tier (the desk is the channel).
async fn deliver(facilitator: &Facilitator, tier: &str, payload: &Value) -> bool {
    let channels: Vec<&AlertChannel> = facilitator
        .settings
        .alerts
        .channels
        .iter()
        .filter(|channel| channel.tiers.iter().any(|name| name == tier))
        .collect();
    if channels.is_empty() {
        return true;
    }
    let mut any = false;
    for channel in channels {
        let result = send(facilitator, channel, payload).await;
        match result {
            Ok(()) => {
                record_property_alert_sent(&channel.name, "delivered");
                any = true;
            }
            Err(error) => {
                record_property_alert_sent(&channel.name, "failed");
                tracing::warn!(%error, channel = %channel.name, tier, "alert channel failed");
            }
        }
    }
    any
}

async fn send(
    facilitator: &Facilitator,
    channel: &AlertChannel,
    payload: &Value,
) -> Result<(), FacilitatorError> {
    match &channel.kind {
        AlertChannelKind::Webhook { url, bearer } => {
            let mut request = facilitator.http.post(url).json(payload);
            if let Some(token) = bearer {
                request = request.bearer_auth(token);
            }
            let response = request
                .send()
                .await
                .map_err(|error| FacilitatorError::Http(error.to_string()))?;
            if response.status().is_success() {
                Ok(())
            } else {
                Err(FacilitatorError::Http(format!(
                    "{url} returned {}",
                    response.status()
                )))
            }
        }
        AlertChannelKind::PropertyMcp { tool } => {
            let url = facilitator
                .settings
                .property_mcp_url
                .as_deref()
                .ok_or_else(|| {
                    FacilitatorError::Alert("property_mcp_url is not set".to_string())
                })?;
            crate::call_upstream(
                &facilitator.http,
                url,
                facilitator.settings.property_mcp_token.as_deref(),
                tool,
                payload,
            )
            .await
            .map(|_| ())
        }
    }
}
