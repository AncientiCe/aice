//! Personal assistant skill: calendar, reminders, messages.

mod types;

pub use types::{AssistantItem, AssistantResult, AssistantSkill, AssistantSkillError};

/// Mock implementation for tests.
pub struct MockAssistantSkill {
    pub result: Result<AssistantResult, AssistantSkillError>,
}

impl MockAssistantSkill {
    pub fn ok(result: AssistantResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: AssistantSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl AssistantSkill for MockAssistantSkill {
    async fn execute(&self, _kind: Option<&str>) -> Result<AssistantResult, AssistantSkillError> {
        self.result.clone()
    }
}
