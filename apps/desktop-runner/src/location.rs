//! Startup location: IP geolocation only.
//! This location is used for weather, time, and distance when the user does not name a place.

use core_config::Config;
use core_observability::record_location_preload;
use core_skills::{OpenMeteoWeatherSkill, ResolvedLocation};
use tracing::info;

use crate::memory::MemoryStore;

const IP_API_URL: &str = "http://ip-api.com/json/?fields=status,city,country,lat,lon";

/// Resolve user location at startup from IP geolocation.
pub async fn resolve_startup_location(
    _config: &Config,
    weather_skill: &OpenMeteoWeatherSkill,
) -> Option<ResolvedLocation> {
    let _ = weather_skill;
    if let Some(loc) = try_ip_geolocation().await {
        info!(display_name = %loc.display_name, "startup location from IP geolocation");
        record_location_preload("success");
        return Some(loc);
    }

    info!("no startup location (IP geolocation unavailable)");
    record_location_preload("failure");
    None
}

/// Build the LLM system prompt with the resolved location so the model knows the user's place.
/// Called after resolve_startup_location; feed the result into OllamaLlmStream::new.
pub fn llm_system_prompt_with_location(
    base_system_prompt: Option<&str>,
    resolved_location: Option<&ResolvedLocation>,
) -> Option<String> {
    let base = base_system_prompt.unwrap_or("").trim();
    let location_line =
        resolved_location.map(|loc| format!("User location: {}.", loc.display_name));
    match (base.is_empty(), location_line) {
        (true, None) => None,
        (true, Some(line)) => Some(line),
        (false, None) => Some(base.to_string()),
        (false, Some(line)) => Some(format!("{}\n\n{}", base, line)),
    }
}

/// Build effective system prompt from base, config assistant_profile, location, and memory summary.
/// Use this at startup so the LLM gets identity, units, user name, and stored profile facts.
pub fn build_effective_system_prompt(
    config: &Config,
    base_system_prompt: Option<&str>,
    resolved_location: Option<&ResolvedLocation>,
    memory: Option<&MemoryStore>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    let base = base_system_prompt.unwrap_or("").trim();
    if !base.is_empty() {
        parts.push(base.to_string());
    }

    if let Some(loc) = resolved_location {
        parts.push(format!("User location: {}.", loc.display_name));
    }

    let profile = &config.assistant_profile;
    let mut profile_lines: Vec<String> = Vec::new();
    if let Some(ref name) = profile.name {
        if !name.is_empty() {
            profile_lines.push(format!("You are {}.", name));
        }
    }
    if let Some(ref persona) = profile.persona {
        if !persona.is_empty() {
            profile_lines.push(persona.clone());
        }
    }
    if !profile.unit_system.is_empty() {
        profile_lines.push(format!(
            "Use {} units (e.g. Celsius, kilometers, 24h time unless user asks otherwise).",
            profile.unit_system
        ));
    }
    if profile.time_format == "12h" {
        profile_lines.push("Use 12-hour time format when saying times.".to_string());
    }
    if let Some(ref user_name) = profile.user_name {
        if !user_name.is_empty() {
            profile_lines.push(format!("The user's name is {}.", user_name));
        }
    }
    if !profile_lines.is_empty() {
        parts.push(profile_lines.join(" "));
    }

    if let Some(mem) = memory {
        let mem_parts: Vec<String> = mem
            .profile()
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect();
        if !mem_parts.is_empty() {
            parts.push(format!("Remembered preferences: {}.", mem_parts.join("; ")));
        }
        let fact_lines: Vec<&str> = mem
            .facts()
            .iter()
            .map(|f| f.value.as_str())
            .take(5)
            .collect();
        if !fact_lines.is_empty() {
            parts.push(format!("Known facts: {}.", fact_lines.join("; ")));
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

async fn try_ip_geolocation() -> Option<ResolvedLocation> {
    #[derive(serde::Deserialize)]
    struct IpApiResponse {
        status: String,
        city: Option<String>,
        country: Option<String>,
        lat: Option<f64>,
        lon: Option<f64>,
    }
    let client = reqwest::Client::new();
    let res = client.get(IP_API_URL).send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    let body: IpApiResponse = res.json().await.ok()?;
    if body.status != "success" {
        return None;
    }
    let lat = body.lat?;
    let lon = body.lon?;
    let display_name = match (&body.city, &body.country) {
        (Some(city), Some(country)) => format!("{}, {}", city, country),
        (Some(city), None) => city.clone(),
        (None, Some(country)) => country.clone(),
        (None, None) => format!("{:.4}, {:.4}", lat, lon),
    };
    Some(ResolvedLocation {
        display_name,
        lat,
        lon,
    })
}

#[cfg(test)]
mod tests {
    use super::build_effective_system_prompt;
    use core_config::Config;
    use core_skills::ResolvedLocation;

    use crate::memory::MemoryStore;

    #[test]
    fn build_effective_system_prompt_includes_profile_and_location() {
        let config = Config::default();
        let loc = ResolvedLocation {
            display_name: "London, UK".to_string(),
            lat: 51.5,
            lon: -0.1,
        };
        let out = build_effective_system_prompt(
            &config,
            Some("You are helpful."),
            Some(&loc),
            None::<&MemoryStore>,
        );
        let Some(s) = out else {
            panic!("expected prompt to be present");
        };
        assert!(s.contains("You are helpful."));
        assert!(s.contains("User location: London, UK"));
        assert!(s.contains("metric"));
    }

    #[test]
    fn build_effective_system_prompt_with_memory_includes_profile_facts() {
        let config = Config::default();
        let limits = core_config::MemoryConfig::default();
        let mut store = MemoryStore::new(&limits);
        store.set_profile("user_name", "Ancie");
        let out = build_effective_system_prompt(&config, None, None, Some(&store));
        let Some(s) = out else {
            panic!("expected prompt to be present");
        };
        assert!(s.contains("Remembered preferences:"));
        assert!(s.contains("user_name: Ancie"));
    }
}
