//! Time skill: current time at a location (optional; default = current location).

mod open_meteo;
mod types;

pub use open_meteo::OpenMeteoTimeSkill;
pub use types::{TimeResult, TimeSkill, TimeSkillError};

/// Mock implementation for tests.
pub struct MockTimeSkill {
    pub result: Result<TimeResult, TimeSkillError>,
}

impl MockTimeSkill {
    pub fn ok(result: TimeResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: TimeSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl TimeSkill for MockTimeSkill {
    async fn execute(
        &self,
        _location: Option<&str>,
        _default_location: Option<&skill_weather::ResolvedLocation>,
    ) -> Result<TimeResult, TimeSkillError> {
        self.result.clone()
    }
}
