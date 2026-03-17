//! Distance skill types and trait.

use async_trait::async_trait;
use skill_weather::ResolvedLocation;

/// Structured distance result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct DistanceResult {
    pub origin_display: String,
    pub destination_display: String,
    /// Straight-line distance in kilometres.
    pub distance_km: f64,
}

impl DistanceResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        format!(
            "From {} to {}: {} km (straight-line).",
            self.origin_display, self.destination_display, self.distance_km
        )
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum DistanceSkillError {
    #[error("geocoding failed: {0}")]
    Geocoding(String),
    #[error("no default location when origin or destination missing")]
    NoDefaultLocation,
    #[error("need at least one of origin or destination")]
    MissingPlaces,
}

/// Distance skill: origin and/or destination; missing one defaults to current location.
#[async_trait]
pub trait DistanceSkill: Send + Sync {
    async fn execute(
        &self,
        origin: Option<&str>,
        destination: Option<&str>,
        default_location: Option<&ResolvedLocation>,
    ) -> Result<DistanceResult, DistanceSkillError>;
}
