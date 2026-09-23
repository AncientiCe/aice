//! Device-token checks for `/turns/stream`.
//!
//! Tokens are verified against the property facilitator on every connection
//! and every turn, because the answer carries the room's current stay. When the
//! facilitator cannot be reached, a recently verified identity keeps the room
//! working with memory switched off, so a desk restart does not silence rooms.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use core_observability::{record_backend_auth_rejection, record_backend_device_auth_duration};
use property_facilitator::{token_digest, DeviceIdentity, PropertyClient};
use tokio::sync::Mutex;

/// How long a verified token keeps working while the facilitator is unreachable.
const STALE_FOR: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, thiserror::Error)]
pub enum DeviceAuthError {
    #[error("device token required")]
    Missing,
    #[error("device token not recognised")]
    Rejected,
    #[error("device verification unavailable: {0}")]
    Unavailable(String),
}

pub struct DeviceAuth {
    client: PropertyClient,
    /// Last successful verification per token digest, for offline fallback only.
    last_verified: Mutex<HashMap<String, (DeviceIdentity, Instant)>>,
}

impl DeviceAuth {
    pub fn new(client: PropertyClient) -> Self {
        Self {
            client,
            last_verified: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve a bearer token to the pod and room it was issued for.
    pub async fn authenticate(
        &self,
        token: Option<&str>,
    ) -> Result<DeviceIdentity, DeviceAuthError> {
        let started = Instant::now();
        let result = self.resolve(token).await;
        let label = match &result {
            Ok(_) => "accepted",
            Err(DeviceAuthError::Missing) => "missing",
            Err(DeviceAuthError::Rejected) => "rejected",
            Err(DeviceAuthError::Unavailable(_)) => "unavailable",
        };
        record_backend_device_auth_duration(label, started.elapsed());
        if result.is_err() {
            record_backend_auth_rejection(label);
        }
        result
    }

    async fn resolve(&self, token: Option<&str>) -> Result<DeviceIdentity, DeviceAuthError> {
        let token = token
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or(DeviceAuthError::Missing)?;
        let key = token_digest(token);
        match self.client.verify_device(token).await {
            Ok(Some(identity)) => {
                self.last_verified
                    .lock()
                    .await
                    .insert(key, (identity.clone(), Instant::now()));
                Ok(identity)
            }
            Ok(None) => {
                self.last_verified.lock().await.remove(&key);
                Err(DeviceAuthError::Rejected)
            }
            Err(error) => {
                let cached = self.last_verified.lock().await.get(&key).cloned();
                match cached {
                    Some((mut identity, verified_at)) if verified_at.elapsed() < STALE_FOR => {
                        tracing::warn!(%error, device_id = %identity.device_id, "facilitator unreachable; using recent device verification without memory");
                        // The stay may have changed since; remember nothing until we know.
                        identity.memory_wing = None;
                        Ok(identity)
                    }
                    _ => Err(DeviceAuthError::Unavailable(error.to_string())),
                }
            }
        }
    }
}
