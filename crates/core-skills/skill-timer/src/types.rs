//! Timer skill types and trait.

use async_trait::async_trait;

/// Structured result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct TimerResult {
    pub summary: String,
    pub timer_name: String,
    pub duration_display: String,
    pub duration_seconds: u64,
}

impl TimerResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        format!(
            "{}. Timer '{}' started for {}.",
            self.summary, self.timer_name, self.duration_display
        )
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum TimerSkillError {
    #[error("execution error: {0}")]
    Execution(String),
    #[error("invalid duration: {0}")]
    InvalidDuration(String),
    #[error("timer app unavailable")]
    Unavailable,
}

/// Timer skill: create macOS Clock timers; duration required, name is optional.
/// When name is not supplied, an ordinal name is derived from currently active timers.
#[async_trait]
pub trait TimerSkill: Send + Sync {
    async fn execute(
        &self,
        duration: &str,
        name: Option<&str>,
    ) -> Result<TimerResult, TimerSkillError>;
}
