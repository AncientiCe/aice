//! Reminder skill: create macOS Reminders entries with optional due date.

mod macos_reminders;
mod types;

pub use macos_reminders::MacOsReminderSkill;
pub use types::{ReminderResult, ReminderSkill, ReminderSkillError};

/// Stub implementation for tests and wiring without a real macOS Reminders backend.
pub struct MockReminderSkill {
    pub result: Result<ReminderResult, ReminderSkillError>,
}

impl MockReminderSkill {
    pub fn ok(result: ReminderResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: ReminderSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl ReminderSkill for MockReminderSkill {
    async fn execute(
        &self,
        _title: &str,
        _when: Option<&str>,
    ) -> Result<ReminderResult, ReminderSkillError> {
        self.result.clone()
    }
}
