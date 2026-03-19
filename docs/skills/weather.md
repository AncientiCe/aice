# Skill: Weather

**Crate:** `skill-weather` · **Impl:** `OpenMeteoWeatherSkill`

**Purpose:** Fetch the current weather conditions for a named place or the user's default location. No API key required; uses the Open-Meteo public API.

---

## Full Journey

```mermaid
sequenceDiagram
    participant LLM as Intent / LLM
    participant Skill as WeatherSkill
    participant Geo as Open-Meteo Geocoding
    participant Wx as Open-Meteo Forecast
    participant Composer as AnswerComposerLLM

    LLM->>Skill: execute(location, default_location)
    alt location provided
        Skill->>Geo: GET /v1/search?name=<location>
        Geo-->>Skill: lat, lon, display_name
    else no location, default_location present
        Skill->>Skill: use default_location (lat, lon)
    else neither provided
        Skill-->>LLM: Err(NoDefaultLocation)
    end
    Skill->>Wx: GET /v1/forecast?latitude=…&longitude=…&current_weather=true&hourly=relativehumidity_2m
    Wx-->>Skill: temp_c, weather_code, humidity_pct
    Skill->>Skill: map WMO code → description ("Clear sky", "Rain", …)
    Skill-->>Composer: WeatherResult { location_display, temp_c, humidity_pct, description }
    Composer-->>LLM: to_prompt_context() injected into answer prompt
```

---

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `location` | `Option<&str>` | Named place to look up (e.g. `"London"`). |
| `default_location` | `Option<&ResolvedLocation>` | Pre-resolved lat/lon from startup; used when `location` is absent. |

## Outputs

`WeatherResult { location_display, temp_c, humidity_pct, weather_code, description }`

## Failure Paths

| Error | Cause |
|-------|-------|
| `Geocoding` | Geocoding API unreachable or returns no results. |
| `Forecast` | Forecast API unreachable or returns unexpected shape. |
| `NoDefaultLocation` | Neither `location` nor `default_location` is available. |

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| *(none instrumented yet — add when touching this skill)* | — | — |
