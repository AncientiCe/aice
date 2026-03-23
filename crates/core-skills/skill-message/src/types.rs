//! Message skill types and trait.

use async_trait::async_trait;

#[derive(Clone, Debug)]
pub struct MessageResult {
    pub summary: String,
    pub recipient_name: String,
    pub recipient_handle: String,
    pub message: String,
}

impl MessageResult {
    pub fn to_prompt_context(&self) -> String {
        format!("Sent \"{}\" to {}.", self.message, self.recipient_name)
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum MessageSkillError {
    #[error("contact not found: {0}")]
    ContactNotFound(String),
    #[error("send failed: {0}")]
    SendFailed(String),
    #[error("execution error: {0}")]
    Execution(String),
    #[error("messages unavailable")]
    Unavailable,
}

#[async_trait]
pub trait MessageSkill: Send + Sync {
    async fn execute(
        &self,
        contact: &str,
        message: &str,
    ) -> Result<MessageResult, MessageSkillError>;
}
