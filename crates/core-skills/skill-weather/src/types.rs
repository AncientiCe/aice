//! Weather skill types and trait.

use async_trait::async_trait;

/// Resolved user or default location (display name and coordinates).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedLocation {
    pub display_name: String,
    pub lat: f64,
    pub lon: f64,
}

/// Structured weather result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct WeatherResult {
    pub location_display: String,
    pub temp_c: f64,
    pub humidity_pct: Option<u8>,
    pub weather_code: u16,
    pub description: String,
}

impl WeatherResult {
    /// Format for inclusion in an LLM prompt (context for answer composition).
    pub fn to_prompt_context(&self) -> String {
        let humidity = self
            .humidity_pct
            .map(|h| format!("{}% humidity", h))
            .unwrap_or_else(|| "humidity N/A".to_string());
        format!(
            "Location: {}. Temperature: {}°C. {}. Conditions: {}.",
            self.location_display, self.temp_c, humidity, self.description
        )
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum WeatherSkillError {
    #[error("geocoding failed: {0}")]
    Geocoding(String),
    #[error("forecast request failed: {0}")]
    Forecast(String),
    #[error("no default location configured")]
    NoDefaultLocation,
}

/// Weather skill: optional location override, fallback to default (e.g. startup-resolved).
#[async_trait]
pub trait WeatherSkill: Send + Sync {
    async fn execute(
        &self,
        location: Option<&str>,
        default_location: Option<&ResolvedLocation>,
    ) -> Result<WeatherResult, WeatherSkillError>;
}
