# Skill: Dictionary

**Crate:** `skill-dictionary` · **Impl:** `HttpDictionarySkill` (backend-owned, `DictionaryApiDevProvider`)

**Purpose:** Look up definitions, parts of speech, and example sentences for a single English word.

## Full Journey

```mermaid
sequenceDiagram
    participant User
    participant Backend as AiceBackendEngine
    participant Skill as HttpDictionarySkill
    participant API as dictionaryapi.dev
    User->>Backend: SkillDictionary { word }
    Backend->>Skill: execute(word)
    Skill->>API: GET /api/v2/entries/en/{word}
    API-->>Skill: entries[]
    Skill-->>Backend: DictionaryResult { word, phonetic, entries }
    Backend-->>User: "ephemeral: (adjective) lasting a short time; ..."
```

## Inputs

| Field | Type | Notes |
|-------|------|-------|
| `word` | `Option<String>` | Single word to look up; required. |

## Outputs

`DictionaryResult` with `word`, `phonetic`, and up to N `DictionaryEntry { part_of_speech, definition, example, synonyms, antonyms }` entries. Composer trims to the first 3 entries.

## Failure Paths

`DictionaryError`: `InvalidQuery`, `NotFound`, `ProviderUnavailable`, `UpstreamTimeout`, `UpstreamParse`.

## Notes

- 24h fresh TTL, 7d stale TTL.
- Free public API; no key required.

## Metrics

- `voice_dictionary_skill_total{result}`.
- Standard `backend_skill_execute_*` and `backend_dependency_*`.
