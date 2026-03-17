//! Distance skill: distance between two places (origin/destination; current location as default).

mod geocode;
mod types;

pub use types::{DistanceResult, DistanceSkill, DistanceSkillError};

use async_trait::async_trait;
use skill_weather::ResolvedLocation;

/// Distance skill using Open-Meteo geocoding and Haversine formula.
pub struct OpenMeteoDistanceSkill {
    client: reqwest::Client,
}

impl OpenMeteoDistanceSkill {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    async fn resolve_place(
        &self,
        place: Option<&str>,
        default: Option<&ResolvedLocation>,
    ) -> Result<ResolvedLocation, DistanceSkillError> {
        match (place, default) {
            (Some(name), _) => geocode::geocode_place(&self.client, name.trim()).await,
            (None, Some(loc)) => Ok(loc.clone()),
            (None, None) => Err(DistanceSkillError::NoDefaultLocation),
        }
    }
}

impl Default for OpenMeteoDistanceSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DistanceSkill for OpenMeteoDistanceSkill {
    async fn execute(
        &self,
        origin: Option<&str>,
        destination: Option<&str>,
        default_location: Option<&ResolvedLocation>,
    ) -> Result<DistanceResult, DistanceSkillError> {
        if origin.is_none() && destination.is_none() {
            return Err(DistanceSkillError::MissingPlaces);
        }
        let origin_loc = self.resolve_place(origin, default_location).await?;
        let dest_loc = self.resolve_place(destination, default_location).await?;
        let distance_km =
            geocode::haversine_km(origin_loc.lat, origin_loc.lon, dest_loc.lat, dest_loc.lon);
        Ok(DistanceResult {
            origin_display: origin_loc.display_name,
            destination_display: dest_loc.display_name,
            distance_km,
        })
    }
}

/// Mock implementation for tests.
pub struct MockDistanceSkill {
    pub result: Result<DistanceResult, DistanceSkillError>,
}

impl MockDistanceSkill {
    pub fn ok(result: DistanceResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: DistanceSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait]
impl DistanceSkill for MockDistanceSkill {
    async fn execute(
        &self,
        _origin: Option<&str>,
        _destination: Option<&str>,
        _default_location: Option<&ResolvedLocation>,
    ) -> Result<DistanceResult, DistanceSkillError> {
        self.result.clone()
    }
}
