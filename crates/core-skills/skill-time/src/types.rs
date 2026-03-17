//! Time skill types and trait.

use async_trait::async_trait;
use skill_weather::ResolvedLocation;

/// Structured time result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct TimeResult {
    pub location_display: String,
    pub local_time: String,
    pub timezone: String,
}

impl TimeResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        format!(
            "Location: {}. Local time: {}. Timezone: {}.",
            self.location_display, self.local_time, self.timezone
        )
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum TimeSkillError {
    #[error("geocoding failed: {0}")]
    Geocoding(String),
    #[error("time request failed: {0}")]
    TimeRequest(String),
    #[error("no default location configured")]
    NoDefaultLocation,
}

/// Time skill: optional location override, fallback to default (e.g. startup-resolved).
#[async_trait]
pub trait TimeSkill: Send + Sync {
    async fn execute(
        &self,
        location: Option<&str>,
        default_location: Option<&ResolvedLocation>,
    ) -> Result<TimeResult, TimeSkillError>;
}
