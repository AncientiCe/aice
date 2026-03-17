//! Personal assistant skill types and trait.

use async_trait::async_trait;

/// Structured result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct AssistantResult {
    pub summary: String,
    pub items: Vec<AssistantItem>,
}

/// Single calendar/reminder/message item.
#[derive(Clone, Debug)]
pub struct AssistantItem {
    pub kind: String,
    pub title: String,
    pub when: Option<String>,
    pub detail: Option<String>,
}

impl AssistantResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        let items_str: String = self
            .items
            .iter()
            .map(|i| {
                let when = i.when.as_deref().unwrap_or("(no time)");
                format!("{}: {} at {}", i.kind, i.title, when)
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!("{}. Items: [{}].", self.summary, items_str)
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum AssistantSkillError {
    #[error("calendar/reminder provider error: {0}")]
    Provider(String),
    #[error("no items found")]
    NoItems,
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

/// Personal assistant skill: calendar, reminders, messages; kind = calendar | reminder | message.
#[async_trait]
pub trait AssistantSkill: Send + Sync {
    async fn execute(&self, kind: Option<&str>) -> Result<AssistantResult, AssistantSkillError>;
}
