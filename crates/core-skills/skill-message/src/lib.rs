//! Message skill: send iMessage messages to contacts via macOS integrations.

mod macos_messages;
mod types;

pub use macos_messages::MacOsMessagesSkill;
pub use types::{MessageResult, MessageSkill, MessageSkillError};

/// Stub implementation for tests and wiring without a real macOS Messages backend.
pub struct MockMessageSkill {
    pub result: Result<MessageResult, MessageSkillError>,
}

impl MockMessageSkill {
    pub fn ok(result: MessageResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: MessageSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl MessageSkill for MockMessageSkill {
    async fn execute(
        &self,
        _contact: &str,
        _message: &str,
    ) -> Result<MessageResult, MessageSkillError> {
        self.result.clone()
    }
}
