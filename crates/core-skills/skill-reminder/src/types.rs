//! Reminder skill types and trait.

use async_trait::async_trait;

/// Structured result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct ReminderResult {
    pub summary: String,
    pub title: String,
    pub when: Option<String>,
}

impl ReminderResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        match &self.when {
            Some(w) => format!("{}. Title: \"{}\". Due: {}.", self.summary, self.title, w),
            None => format!("{}. Title: \"{}\". No due date.", self.summary, self.title),
        }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ReminderSkillError {
    #[error("execution error: {0}")]
    Execution(String),
    #[error("invalid due date: {0}")]
    InvalidDate(String),
    #[error("reminder app unavailable")]
    Unavailable,
}

/// Reminder skill: create macOS Reminders entries; title required, when is optional.
#[async_trait]
pub trait ReminderSkill: Send + Sync {
    async fn execute(
        &self,
        title: &str,
        when: Option<&str>,
    ) -> Result<ReminderResult, ReminderSkillError>;
}
