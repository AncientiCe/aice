# Skill: Assistant

**Crate:** `skill-assistant` · **Impl:** `MockAssistantSkill` (no concrete macOS implementation yet)

**Purpose:** Trait and type scaffolding for querying calendar events, reminders, and messages. Defines the interface and result types that a real macOS EventKit / Messages integration will fulfil.

**Execution Owner (Split Runtime):** `aice-macos`

---

## Full Journey

```mermaid
sequenceDiagram
    participant LLM as Intent / LLM
    participant Skill as AssistantSkill
    participant Provider as Calendar / Reminders / Messages (future)
    participant Composer as AnswerComposerLLM

    LLM->>Skill: execute(kind)

    alt kind = "calendar"
        Skill->>Provider: fetch upcoming calendar events
        Provider-->>Skill: Vec<AssistantItem { kind="calendar", title, when, detail }>
    else kind = "reminder"
        Skill->>Provider: fetch pending reminders
        Provider-->>Skill: Vec<AssistantItem { kind="reminder", title, when, detail }>
    else kind = "message"
        Skill->>Provider: fetch recent messages
        Provider-->>Skill: Vec<AssistantItem { kind="message", title, when, detail }>
    else invalid kind
        Skill-->>LLM: Err(InvalidRequest)
    end

    alt no items returned
        Skill-->>LLM: Err(NoItems)
    end

    alt provider error
        Skill-->>LLM: Err(Provider)
    end

    Skill-->>Composer: AssistantResult { summary, items }
    Composer-->>LLM: to_prompt_context() injected into answer prompt
```

---

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `kind` | `Option<&str>` | `"calendar"`, `"reminder"`, or `"message"`. |

## Outputs

`AssistantResult { summary, items: Vec<AssistantItem> }`

`AssistantItem { kind, title, when: Option<String>, detail: Option<String> }`

## Failure Paths

| Error | Cause |
|-------|-------|
| `InvalidRequest` | `kind` is not one of the accepted values. |
| `NoItems` | Provider returned an empty result set. |
| `Provider` | Provider integration failed (permissions, unavailable service, etc.). |

## Notes

- No concrete macOS implementation exists yet; only `MockAssistantSkill` is available.
- When a real implementation is added, update this document with the actual provider API calls and AppleScript / EventKit flow.

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| *(none instrumented yet — add when implementing this skill)* | — | — |
