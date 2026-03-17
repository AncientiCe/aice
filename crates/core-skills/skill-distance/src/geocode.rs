//! Open-Meteo geocoding (same API as skill-weather).

use crate::types::DistanceSkillError;
use skill_weather::ResolvedLocation;

const GEOCODING_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";

#[derive(serde::Deserialize)]
struct GeocodingResponse {
    results: Option<Vec<GeocodingResult>>,
}

#[derive(serde::Deserialize)]
struct GeocodingResult {
    name: String,
    latitude: f64,
    longitude: f64,
    country: Option<String>,
}

pub async fn geocode_place(
    client: &reqwest::Client,
    name: &str,
) -> Result<ResolvedLocation, DistanceSkillError> {
    let res = client
        .get(GEOCODING_URL)
        .query(&[("name", name), ("count", "1")])
        .send()
        .await
        .map_err(|e| DistanceSkillError::Geocoding(e.to_string()))?;
    if !res.status().is_success() {
        return Err(DistanceSkillError::Geocoding(format!(
            "status {}",
            res.status()
        )));
    }
    let body: GeocodingResponse = res
        .json()
        .await
        .map_err(|e| DistanceSkillError::Geocoding(e.to_string()))?;
    let first = body
        .results
        .and_then(|r| r.into_iter().next())
        .ok_or_else(|| DistanceSkillError::Geocoding("no results".to_string()))?;
    let display_name = if let Some(ref c) = first.country {
        format!("{}, {}", first.name, c)
    } else {
        first.name.clone()
    };
    Ok(ResolvedLocation {
        display_name,
        lat: first.latitude,
        lon: first.longitude,
    })
}

/// Haversine distance in kilometres.
pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6371.0; // Earth radius in km
    let lat1 = lat1.to_radians();
    let lat2 = lat2.to_radians();
    let dlat = lat2 - lat1;
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().mul_add(
        (dlat / 2.0).sin(),
        lat1.cos() * lat2.cos() * (dlon / 2.0).sin() * (dlon / 2.0).sin(),
    );
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    R * c
}
