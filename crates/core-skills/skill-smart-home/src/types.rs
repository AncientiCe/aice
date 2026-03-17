//! Smart home skill types and trait.

use async_trait::async_trait;

/// Structured result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct SmartHomeResult {
    pub summary: String,
    pub device_states: Vec<DeviceState>,
}

/// State of a single device or group.
#[derive(Clone, Debug)]
pub struct DeviceState {
    pub id: String,
    pub name: String,
    pub state: String,
}

impl SmartHomeResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        let states: String = self
            .device_states
            .iter()
            .map(|d| format!("{}: {}", d.name, d.state))
            .collect::<Vec<_>>()
            .join("; ");
        format!("{}. Devices: [{}].", self.summary, states)
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum SmartHomeSkillError {
    #[error("device or adapter error: {0}")]
    Device(String),
    #[error("unsupported action: {0}")]
    UnsupportedAction(String),
    #[error("timeout communicating with devices")]
    Timeout,
}

/// Smart home skill: control lights, climate, scenes; optional target and action from intent.
#[async_trait]
pub trait SmartHomeSkill: Send + Sync {
    async fn execute(
        &self,
        target: Option<&str>,
        action: Option<&str>,
    ) -> Result<SmartHomeResult, SmartHomeSkillError>;
}
