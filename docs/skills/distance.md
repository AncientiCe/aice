# Skill: Distance

**Crate:** `skill-distance` · **Impl:** `OpenMeteoDistanceSkill`

**Purpose:** Compute the straight-line (Haversine) distance in kilometres between two named places. Either endpoint can be omitted and will fall back to the user's default (current) location.

---

## Full Journey

```mermaid
sequenceDiagram
    participant LLM as Intent / LLM
    participant Skill as DistanceSkill
    participant Geo as Open-Meteo Geocoding
    participant Math as Haversine (local)
    participant Composer as AnswerComposerLLM

    LLM->>Skill: execute(origin, destination, default_location)

    alt both origin and destination absent
        Skill-->>LLM: Err(MissingPlaces)
    end

    alt origin provided
        Skill->>Geo: GET /v1/search?name=<origin>
        Geo-->>Skill: origin lat/lon, display_name
    else origin absent, default_location present
        Skill->>Skill: use default_location as origin
    else origin absent, no default
        Skill-->>LLM: Err(NoDefaultLocation)
    end

    alt destination provided
        Skill->>Geo: GET /v1/search?name=<destination>
        Geo-->>Skill: destination lat/lon, display_name
    else destination absent, default_location present
        Skill->>Skill: use default_location as destination
    else destination absent, no default
        Skill-->>LLM: Err(NoDefaultLocation)
    end

    Skill->>Math: haversine_km(lat1, lon1, lat2, lon2)
    Math-->>Skill: distance_km
    Skill-->>Composer: DistanceResult { origin_display, destination_display, distance_km }
    Composer-->>LLM: to_prompt_context() injected into answer prompt
```

---

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `origin` | `Option<&str>` | Starting place name. Falls back to `default_location`. |
| `destination` | `Option<&str>` | Ending place name. Falls back to `default_location`. |
| `default_location` | `Option<&ResolvedLocation>` | Pre-resolved lat/lon used when an endpoint is omitted. |

## Outputs

`DistanceResult { origin_display, destination_display, distance_km }`

## Failure Paths

| Error | Cause |
|-------|-------|
| `MissingPlaces` | Both `origin` and `destination` are absent with no fallback logic possible. |
| `NoDefaultLocation` | One endpoint is absent and no `default_location` is available. |
| `Geocoding` | Open-Meteo geocoding returns no results for a named place. |

## Notes

- Uses Earth radius **6371 km** for the Haversine formula.
- Distance is straight-line only; does not account for roads or travel routes.

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| *(none instrumented yet — add when touching this skill)* | — | — |
