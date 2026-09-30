# Skill: Horoscope Daily

**Crate:** `skill-horoscope-daily` · **Impl:** `HttpHoroscopeDailySkill`

**Purpose:** Retrieve daily horoscope text for a zodiac sign and optional date.

**Execution Owner (Split Runtime):** `aice-backend`

---

## Full Journey

```mermaid
sequenceDiagram
    participant LLM as Intent / LLM
    participant Backend as aice-backend
    participant Skill as HoroscopeDailySkill
    participant Provider as Horoscope API
    participant Composer as Answer Composer

    LLM->>Backend: intent=skill_horoscope_daily, horoscope_sign, horoscope_date?
    Backend->>Skill: execute(HoroscopeDailyQuery)
    Skill->>Provider: fetch horoscope
    Provider-->>Skill: horoscope payload
    Skill-->>Composer: HoroscopeDailyResult.to_prompt_context()
    Composer-->>Backend: spoken response
```

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `horoscope_sign` | `String` | Zodiac sign (required). The classifier schema limits it to the 12 signs (`aries` … `pisces`), so other words from the request ("today") cannot land here. |
| `horoscope_date` | `Option<NaiveDate>` | Optional date filter (defaults to today in backend). |

## Outputs

`HoroscopeDailyResult { sign, day, summary, mood, color, lucky_number, as_of }`

The provider is `freehoroscopeapi.com` (`GET /api/v1/get-horoscope/daily?sign=<sign>&day=TODAY|TOMORROW|YESTERDAY`). It returns only the text, so `mood`, `color`, and `lucky_number` are `None`. The earlier provider (aztro) answers 404 for every sign.

## Failure Paths

| Error | Cause |
|-------|-------|
| `InvalidSign` | Unsupported or invalid zodiac sign. |
| `UnsupportedDate` | Date is not supported by the provider. |
| `ProviderUnavailable` | Upstream provider is unavailable. |
| `UpstreamTimeout` | Provider request timed out. |
| `UpstreamParse` | Provider response could not be parsed or had no text. |

A failed lookup is spoken as a short apology on the pod instead of silence.

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| `backend_skill_execute_total` | Counter | `skill="skill_horoscope_daily"`, `result`, `error_kind` |
| `backend_skill_execute_duration_seconds` | Histogram | `skill="skill_horoscope_daily"` |

