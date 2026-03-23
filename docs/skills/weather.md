# Skill: Weather

**Crate:** `skill-weather` · **Impl:** `OpenMeteoWeatherSkill`

**Purpose:** Fetch the current weather conditions for a named place or the user's default location. No API key required; uses the Open-Meteo public API.

**Execution Owner (Split Runtime):** `aice-backend`

---

## Full Journey

```mermaid
sequenceDiagram
    participant LLM as Intent / LLM
    participant Runtime as DesktopRuntime
    participant Normalizer as LocationContractLLM
    participant Skill as WeatherSkill
    participant Geo as Open-Meteo Geocoding
    participant Wx as Open-Meteo Forecast
    participant Composer as AnswerComposerLLM

    LLM->>Runtime: intent=skill_weather, location?
    alt location provided
        Runtime->>Normalizer: normalize to City, Country JSON
        alt normalization ok
            Runtime->>Skill: execute(normalized_location, resolved_location)
        else ambiguous/unknown
            Runtime-->>LLM: clarification prompt to user
        end
    else no location, resolved_location present
        Runtime->>Skill: execute(None, resolved_location)
    else neither provided
        Runtime->>Skill: execute(None, None)
        Skill-->>LLM: Err(NoDefaultLocation)
    end
    alt skill executes
        Skill->>Geo: GET /v1/search?name=<location candidate> (retries normalized candidates)
        Geo-->>Skill: lat, lon, display_name
        Skill->>Wx: GET /v1/forecast?latitude=…&longitude=…&current_weather=true&hourly=relativehumidity_2m
        Wx-->>Skill: temp_c, weather_code, humidity_pct
        Skill->>Skill: map WMO code → description ("Clear sky", "Rain", …)
        Skill-->>Composer: WeatherResult { location_display, temp_c, humidity_pct, description }
        Composer-->>LLM: to_prompt_context() injected into answer prompt
    else geocoding no results
        Runtime-->>LLM: short clarification ("city and country")
    end
```

---

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `location` | `Option<&str>` | Named place from intent route; runtime first normalizes with LLM location contract to `City, Country`. |
| `resolved_location` | `Option<&ResolvedLocation>` | Pre-resolved lat/lon from startup; used when `location` is absent. |

## Outputs

`WeatherResult { location_display, temp_c, humidity_pct, weather_code, description }`

## Failure Paths

| Error | Cause |
|-------|-------|
| `Geocoding` | Geocoding API unreachable or returns no results after candidate retries (punctuation-stripped phrase, known alias expansions such as `LA`). |
| `Forecast` | Forecast API unreachable or returns unexpected shape. |
| `NoDefaultLocation` | Neither `location` nor `resolved_location` is available. |

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| `voice_weather_skill_total` | Counter | `result` (`success`,`error`) |
| `voice_skill_duration_seconds` | Histogram | `skill="skill_weather"` |
| `voice_location_contract_total` | Counter | `intent`, `result` (`normalized`,`clarify`,`error`) |
| `voice_location_contract_duration_seconds` | Histogram | `intent` |
