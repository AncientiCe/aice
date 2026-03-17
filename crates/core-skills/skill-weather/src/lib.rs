//! Weather skill: Open-Meteo integration and trait for routing.

mod open_meteo;
mod types;

pub use open_meteo::OpenMeteoWeatherSkill;
pub use types::{ResolvedLocation, WeatherResult, WeatherSkill, WeatherSkillError};

/// Mock implementation for tests.
pub struct MockWeatherSkill {
    pub result: Result<WeatherResult, WeatherSkillError>,
}

impl MockWeatherSkill {
    pub fn ok(result: WeatherResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: WeatherSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl WeatherSkill for MockWeatherSkill {
    async fn execute(
        &self,
        _location: Option<&str>,
        _default_location: Option<&ResolvedLocation>,
    ) -> Result<WeatherResult, WeatherSkillError> {
        self.result.clone()
    }
}
