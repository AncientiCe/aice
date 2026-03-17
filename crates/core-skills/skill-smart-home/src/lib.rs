//! Smart home skill: trait and types for lights, climate, scenes.

mod hue;
mod types;

pub use hue::HueSmartHomeSkill;
pub use types::{DeviceState, SmartHomeResult, SmartHomeSkill, SmartHomeSkillError};

/// Mock implementation retained for existing tests that inject explicit outcomes.
pub struct MockSmartHomeSkill {
    pub result: Result<SmartHomeResult, SmartHomeSkillError>,
}

impl MockSmartHomeSkill {
    pub fn ok(result: SmartHomeResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: SmartHomeSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl SmartHomeSkill for MockSmartHomeSkill {
    async fn execute(
        &self,
        _target: Option<&str>,
        _action: Option<&str>,
    ) -> Result<SmartHomeResult, SmartHomeSkillError> {
        self.result.clone()
    }
}
