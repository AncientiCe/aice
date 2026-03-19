//! Timer skill: create macOS Clock timers with optional name.

mod macos_clock;
mod types;

pub use macos_clock::MacOsClockTimerSkill;
pub use types::{TimerResult, TimerSkill, TimerSkillError};

/// Stub implementation for tests and wiring without a real macOS Clock backend.
pub struct MockTimerSkill {
    pub result: Result<TimerResult, TimerSkillError>,
}

impl MockTimerSkill {
    pub fn ok(result: TimerResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: TimerSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl TimerSkill for MockTimerSkill {
    async fn execute(
        &self,
        _duration: &str,
        _name: Option<&str>,
    ) -> Result<TimerResult, TimerSkillError> {
        self.result.clone()
    }
}
