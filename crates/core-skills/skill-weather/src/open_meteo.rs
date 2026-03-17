//! Open-Meteo API: geocoding and forecast (no API key).

use crate::types::{ResolvedLocation, WeatherResult, WeatherSkill, WeatherSkillError};
use async_trait::async_trait;
use serde::Deserialize;

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
    current: Option<CurrentWeather>,
}

#[derive(Deserialize)]
struct CurrentWeather {
    temperature_2m: f64,
    relative_humidity_2m: Option<f64>,
    weather_code: u16,
}

/// Open-Meteo weather skill implementation.
pub struct OpenMeteoWeatherSkill {
    client: reqwest::Client,
}

impl OpenMeteoWeatherSkill {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Resolve a place name to coordinates (e.g. for startup default location).
    pub async fn geocode_place(&self, name: &str) -> Result<ResolvedLocation, WeatherSkillError> {
        self.geocode(name).await
    }

    async fn geocode(&self, name: &str) -> Result<ResolvedLocation, WeatherSkillError> {
        let res = self
            .client
            .get(GEOCODING_URL)
            .query(&[("name", name), ("count", "1")])
            .send()
            .await
            .map_err(|e| WeatherSkillError::Geocoding(e.to_string()))?;
        if !res.status().is_success() {
            return Err(WeatherSkillError::Geocoding(format!(
                "status {}",
                res.status()
            )));
        }
        let body: GeocodingResponse = res
            .json()
            .await
            .map_err(|e| WeatherSkillError::Geocoding(e.to_string()))?;
        let first = body
            .results
            .and_then(|r| r.into_iter().next())
            .ok_or_else(|| WeatherSkillError::Geocoding("no results".to_string()))?;
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

    async fn fetch_forecast(
        &self,
        loc: &ResolvedLocation,
    ) -> Result<WeatherResult, WeatherSkillError> {
        let res = self
            .client
            .get(FORECAST_URL)
            .query(&[
                ("latitude", loc.lat.to_string()),
                ("longitude", loc.lon.to_string()),
                (
                    "current",
                    "temperature_2m,relative_humidity_2m,weather_code".to_string(),
                ),
            ])
            .send()
            .await
            .map_err(|e| WeatherSkillError::Forecast(e.to_string()))?;
        if !res.status().is_success() {
            return Err(WeatherSkillError::Forecast(format!(
                "status {}",
                res.status()
            )));
        }
        let body: ForecastResponse = res
            .json()
            .await
            .map_err(|e| WeatherSkillError::Forecast(e.to_string()))?;
        let current = body
            .current
            .ok_or_else(|| WeatherSkillError::Forecast("missing current".to_string()))?;
        let description = weather_code_to_description(current.weather_code);
        let humidity_pct = current
            .relative_humidity_2m
            .map(|h| h.clamp(0.0, 100.0) as u8);
        Ok(WeatherResult {
            location_display: loc.display_name.clone(),
            temp_c: current.temperature_2m,
            humidity_pct,
            weather_code: current.weather_code,
            description,
        })
    }
}

impl Default for OpenMeteoWeatherSkill {
    fn default() -> Self {
        Self::new()
    }
}

fn weather_code_to_description(code: u16) -> String {
    // WMO weather codes (Open-Meteo)
    match code {
        0 => "Clear sky",
        1..=3 => "Partly cloudy",
        45 | 48 => "Foggy",
        51 | 53 | 55 => "Drizzle",
        61 | 63 | 65 => "Rain",
        71 | 73 | 75 => "Snow",
        80..=82 => "Rain showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm with hail",
        _ => "Variable",
    }
    .to_string()
}

/// When location is None we use default_location: the app's resolved location from location
/// services (IP geolocation or config at startup). Geocode is only used when the user names a place.
#[async_trait]
impl WeatherSkill for OpenMeteoWeatherSkill {
    async fn execute(
        &self,
        location: Option<&str>,
        default_location: Option<&ResolvedLocation>,
    ) -> Result<WeatherResult, WeatherSkillError> {
        let loc = if let Some(name) = location {
            self.geocode(name.trim()).await?
        } else if let Some(default) = default_location {
            default.clone()
        } else {
            return Err(WeatherSkillError::NoDefaultLocation);
        };
        self.fetch_forecast(&loc).await
    }
}
