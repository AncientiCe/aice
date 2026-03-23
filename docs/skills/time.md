# Skill: Time

**Crate:** `skill-time` · **Impl:** `OpenMeteoTimeSkill`

**Purpose:** Return the current local time at a named or default location. Uses Open-Meteo to resolve an IANA timezone, then derives the accurate local time from the system clock.

**Execution Owner (Split Runtime):** `aice-backend`

---

## Full Journey

```mermaid
sequenceDiagram
    participant LLM as Intent / LLM
    participant Skill as TimeSkill
    participant Geo as Open-Meteo Geocoding
    participant API as Open-Meteo Forecast (timezone=auto)
    participant Clock as System Clock (Utc::now)
    participant Composer as AnswerComposerLLM

    LLM->>Skill: execute(location, resolved_location)
    alt location provided
        Skill->>Geo: GET /v1/search?name=<location>
        Geo-->>Skill: lat, lon, display_name
    else no location, resolved_location present
        Skill->>Skill: use resolved_location (lat, lon)
    else neither provided
        Skill-->>LLM: Err(NoDefaultLocation)
    end
    Skill->>API: GET /v1/forecast?latitude=…&longitude=…&timezone=auto
    API-->>Skill: timezone string (IANA, e.g. "Europe/London")
    Skill->>Clock: Utc::now().with_timezone(&tz)
    alt valid IANA timezone
        Clock-->>Skill: local DateTime
    else invalid timezone string
        Clock-->>Skill: UTC fallback
    end
    Skill-->>Composer: TimeResult { location_display, local_time, timezone }
    Composer-->>LLM: to_prompt_context() injected into answer prompt
```

---

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `location` | `Option<&str>` | Named place (e.g. `"Tokyo"`). |
| `resolved_location` | `Option<&ResolvedLocation>` | Pre-resolved lat/lon from startup. |

## Outputs

`TimeResult { location_display, local_time, timezone }`

## Failure Paths

| Error | Cause |
|-------|-------|
| `Geocoding` | Geocoding API unreachable or no results. |
| `TimeRequest` | Forecast API unreachable or timezone field missing. |
| `NoDefaultLocation` | Neither argument is available. |

## Notes

- The API's returned `current_weather.time` value is **ignored**; local time is always computed from `Utc::now()` for accuracy.
- Falls back to UTC silently if the timezone string is not recognised by `chrono-tz`.

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| *(none instrumented yet — add when touching this skill)* | — | — |
