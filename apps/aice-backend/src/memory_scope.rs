//! Which Memory Palace memories a turn may read and write.
//!
//! Home installs share one palace (`Shared`). In a property, a turn from a
//! room pod reads and writes only the wing of that room's open stay (`Stay`),
//! and a room without an open stay remembers nothing (`Disabled`). Knowledge
//! graph entities of a stay are namespaced `<wing>:<name>` so facts about one
//! guest never answer a question from another.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use core_observability::{record_memory_retention_purge, record_palace_error};
use palace::palace::Palace;
use property_facilitator::PropertyClient;
use serde_json::Value;

type DynError = Box<dyn std::error::Error + Send + Sync>;

/// Context key the turn stream sets to the room's stay wing.
pub const CONTEXT_MEMORY_WING: &str = "memory_wing";
/// Context key the turn stream sets when the room has no open stay.
pub const CONTEXT_MEMORY_DISABLED: &str = "memory_disabled";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryScope {
    Shared,
    Stay(String),
    Disabled,
}

impl MemoryScope {
    pub fn from_context(context: Option<&Value>) -> Self {
        let Some(context) = context else {
            return Self::Shared;
        };
        if let Some(wing) = context
            .get(CONTEXT_MEMORY_WING)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|wing| !wing.is_empty())
        {
            return Self::Stay(wing.to_string());
        }
        if context
            .get(CONTEXT_MEMORY_DISABLED)
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Self::Disabled;
        }
        Self::Shared
    }

    /// Metric label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Shared => "global",
            Self::Stay(_) => "stay",
            Self::Disabled => "disabled",
        }
    }

    /// Knowledge-graph entity name for this scope; `None` when memory is off.
    pub fn entity(&self, name: &str) -> Option<String> {
        match self {
            Self::Shared => Some(name.to_string()),
            Self::Stay(wing) => Some(format!("{wing}:{name}")),
            Self::Disabled => None,
        }
    }

    /// Entity name without the stay namespace, for prompts.
    pub fn display<'a>(&self, name: &'a str) -> &'a str {
        match self {
            Self::Stay(wing) => name
                .strip_prefix(wing.as_str())
                .and_then(|rest| rest.strip_prefix(':'))
                .unwrap_or(name),
            Self::Shared | Self::Disabled => name,
        }
    }
}

/// Delete every drawer, knowledge-graph fact, and trace (wing registry,
/// usage events, feedback, mined-file records) of a stay wing.
/// Returns how many drawers were removed.
pub fn purge_stay_memory(palace: &Palace, wing: &str) -> Result<usize, DynError> {
    if !wing.starts_with("stay-") {
        return Err(format!("refusing to purge non-stay wing '{wing}'").into());
    }
    let conn = palace.conn();
    let transaction = conn.unchecked_transaction()?;
    transaction.execute(
        "DELETE FROM bm25_terms WHERE drawer_id IN (SELECT id FROM drawers WHERE wing = ?1)",
        [wing],
    )?;
    transaction.execute(
        "DELETE FROM bm25_doc_stats WHERE drawer_id IN (SELECT id FROM drawers WHERE wing = ?1)",
        [wing],
    )?;
    transaction.execute(
        "DELETE FROM gain_feedback WHERE drawer_id IN (SELECT id FROM drawers WHERE wing = ?1)",
        [wing],
    )?;
    transaction.execute("DELETE FROM closets WHERE wing = ?1", [wing])?;
    transaction.execute("DELETE FROM usage_events WHERE wing = ?1", [wing])?;
    transaction.execute("DELETE FROM mined_files WHERE wing = ?1", [wing])?;
    transaction.execute(
        "DELETE FROM tunnels WHERE wing_a = ?1 OR wing_b = ?1",
        [wing],
    )?;
    let drawers = transaction.execute("DELETE FROM drawers WHERE wing = ?1", [wing])?;
    // palace-rs re-registers wings from drawers on open, so this stays gone.
    transaction.execute("DELETE FROM wings WHERE name = ?1", [wing])?;
    // Entity ids are lower-cased names, so the namespace prefix is too.
    let prefix = format!("{}:", wing.to_lowercase().replace(' ', "_"));
    transaction.execute(
        "DELETE FROM triples WHERE substr(subject, 1, length(?1)) = ?1
            OR substr(object, 1, length(?1)) = ?1",
        [prefix.as_str()],
    )?;
    transaction.execute(
        "DELETE FROM entities WHERE substr(id, 1, length(?1)) = ?1",
        [prefix.as_str()],
    )?;
    transaction.commit()?;
    Ok(drawers)
}

/// Ask the facilitator which stay memories are due, delete them, and confirm.
/// Returns how many stays were purged.
pub async fn run_memory_retention_once(
    palace: &Arc<Mutex<Palace>>,
    client: &PropertyClient,
) -> Result<usize, DynError> {
    let due = client.purge_due().await?;
    let mut purged = 0;
    for stay in due {
        let handle = Arc::clone(palace);
        let wing = stay.memory_wing.clone();
        let result = tokio::task::spawn_blocking(move || {
            let guard = handle
                .lock()
                .map_err(|error| -> DynError { format!("palace lock: {error}").into() })?;
            purge_stay_memory(&guard, &wing)
        })
        .await
        .map_err(|error| -> DynError { format!("purge task: {error}").into() })
        .and_then(|value| value);
        match result {
            Ok(drawers) => {
                client.mark_stay_purged(&stay.id).await?;
                record_memory_retention_purge("purged");
                tracing::info!(stay = %stay.id, drawers, "stay memory purged");
                purged += 1;
            }
            Err(error) => {
                record_memory_retention_purge("error");
                record_palace_error("retention_purge");
                tracing::warn!(%error, stay = %stay.id, "stay memory purge failed");
            }
        }
    }
    Ok(purged)
}

/// Run the retention job every `interval` until the task is dropped.
pub fn spawn_memory_retention(
    palace: Arc<Mutex<Palace>>,
    client: PropertyClient,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let started = Instant::now();
            if let Err(error) = run_memory_retention_once(&palace, &client).await {
                record_memory_retention_purge("unavailable");
                tracing::warn!(%error, "memory retention pass failed");
            }
            tokio::time::sleep(interval.saturating_sub(started.elapsed())).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{purge_stay_memory, MemoryScope};
    use palace::palace::Palace;
    use serde_json::json;

    #[test]
    fn scope_comes_from_the_server_built_context() {
        assert_eq!(MemoryScope::from_context(None), MemoryScope::Shared);
        assert_eq!(
            MemoryScope::from_context(Some(&json!({"memory_wing": "stay-s1"}))),
            MemoryScope::Stay("stay-s1".to_string())
        );
        assert_eq!(
            MemoryScope::from_context(Some(&json!({"memory_disabled": true}))),
            MemoryScope::Disabled
        );
        assert_eq!(
            MemoryScope::from_context(Some(&json!({"memory_wing": "  "}))),
            MemoryScope::Shared
        );
    }

    #[test]
    fn stay_entities_are_namespaced_and_displayed_plainly() {
        let stay = MemoryScope::Stay("stay-s1".to_string());
        assert_eq!(stay.entity("Alice").as_deref(), Some("stay-s1:Alice"));
        assert_eq!(stay.display("stay-s1:Alice"), "Alice");
        assert_eq!(
            MemoryScope::Shared.entity("Alice").as_deref(),
            Some("Alice")
        );
        assert_eq!(MemoryScope::Disabled.entity("Alice"), None);
    }

    #[test]
    fn purge_refuses_wings_that_are_not_stays() {
        let palace = match Palace::open_in_memory() {
            Ok(palace) => palace,
            Err(error) => panic!("palace: {error}"),
        };
        assert!(purge_stay_memory(&palace, "journal").is_err());
        assert!(purge_stay_memory(&palace, "conversations").is_err());
        assert!(matches!(purge_stay_memory(&palace, "stay-s1"), Ok(0)));
    }
}
