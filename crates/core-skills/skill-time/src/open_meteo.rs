//! Time skill: geocoding via Open-Meteo, then IANA timezone from forecast (timezone=auto).
//! We ignore the API's "time" value and compute local time ourselves with chrono + chrono-tz
//! from Utc::now(), so the time is always correct for the location.

use crate::types::{TimeResult, TimeSkill, TimeSkillError};
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use skill_weather::ResolvedLocation;

const GEOCODING_URL: &str = "https://geocoding-api.open-meteo.com/v1/search";
const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";

#[derive(Deserialize)]
struct GeocodingResponse {
    results: Option<Vec<GeocodingResult>>,
}

#[derive(Deserialize)]
struct GeocodingResult {
    name: String,
    latitude: f64,
    longitude: f64,
    country: Option<String>,
}

#[derive(Deserialize)]
struct ForecastResponse {
    timezone: Option<String>,
    timezone_abbreviation: Option<String>,
}

/// Parse IANA timezone (e.g. "Europe/London") and return current time in that zone.
/// Falls back to UTC if the string is not a valid IANA name (e.g. "GMT" from API).
fn parse_iana_and_now(tz_name: &str) -> chrono::DateTime<chrono_tz::Tz> {
    use chrono_tz::Tz;
    tz_name
        .parse::<Tz>()
        .map(|tz| Utc::now().with_timezone(&tz))
        .unwrap_or_else(|_| Utc::now().with_timezone(&chrono_tz::UTC))
}

/// Time skill using Open-Meteo (geocoding + timezone lookup), time from system clock.
pub struct OpenMeteoTimeSkill {
    client: reqwest::Client,
}

impl OpenMeteoTimeSkill {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    async fn geocode(&self, name: &str) -> Result<ResolvedLocation, TimeSkillError> {
        let res = self
            .client
            .get(GEOCODING_URL)
            .query(&[("name", name), ("count", "1")])
            .send()
            .await
            .map_err(|e| TimeSkillError::Geocoding(e.to_string()))?;
        if !res.status().is_success() {
            return Err(TimeSkillError::Geocoding(format!(
                "status {}",
                res.status()
            )));
        }
        let body: GeocodingResponse = res
            .json()
            .await
            .map_err(|e| TimeSkillError::Geocoding(e.to_string()))?;
        let first = body
            .results
            .and_then(|r| r.into_iter().next())
            .ok_or_else(|| TimeSkillError::Geocoding("no results".to_string()))?;
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

    async fn fetch_time(&self, loc: &ResolvedLocation) -> Result<TimeResult, TimeSkillError> {
        let res = self
            .client
            .get(FORECAST_URL)
            .query(&[
                ("latitude", loc.lat.to_string()),
                ("longitude", loc.lon.to_string()),
                ("timezone", "auto".to_string()),
            ])
            .send()
            .await
            .map_err(|e| TimeSkillError::TimeRequest(e.to_string()))?;
        if !res.status().is_success() {
            return Err(TimeSkillError::TimeRequest(format!(
                "status {}",
                res.status()
            )));
        }
        let body: ForecastResponse = res
            .json()
            .await
            .map_err(|e| TimeSkillError::TimeRequest(e.to_string()))?;
        let tz_name = body.timezone.as_deref().unwrap_or("UTC");
        let timezone_abbr = body
            .timezone_abbreviation
            .clone()
            .unwrap_or_else(|| tz_name.to_string());
        let local_dt = parse_iana_and_now(tz_name);
        let local_time = local_dt.format("%H:%M").to_string();
        Ok(TimeResult {
            location_display: loc.display_name.clone(),
            local_time,
            timezone: timezone_abbr,
        })
    }
}

impl Default for OpenMeteoTimeSkill {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TimeSkill for OpenMeteoTimeSkill {
    async fn execute(
        &self,
        location: Option<&str>,
        default_location: Option<&ResolvedLocation>,
    ) -> Result<TimeResult, TimeSkillError> {
        let loc = if let Some(name) = location {
            self.geocode(name.trim()).await?
        } else if let Some(default) = default_location {
            default.clone()
        } else {
            return Err(TimeSkillError::NoDefaultLocation);
        };
        self.fetch_time(&loc).await
    }
}
