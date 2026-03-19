//! Volume skill types and trait.

use async_trait::async_trait;

/// Structured result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct VolumeResult {
    pub summary: String,
    pub action_done: String,
    pub resulting_level: Option<u8>,
}

impl VolumeResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        let level = self
            .resulting_level
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        format!(
            "{}. Action: {}. Resulting level: {}.",
            self.summary, self.action_done, level
        )
    }
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum VolumeSkillError {
    #[error("execution error: {0}")]
    Execution(String),
    #[error("invalid level: {0}")]
    InvalidLevel(u8),
    #[error("unsupported action: {0}")]
    UnsupportedAction(String),
}

/// Volume skill: set, adjust, mute/unmute, and query system output volume.
#[async_trait]
pub trait VolumeSkill: Send + Sync {
    async fn execute(
        &self,
        action: Option<&str>,
        level: Option<u8>,
    ) -> Result<VolumeResult, VolumeSkillError>;
}
