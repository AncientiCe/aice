# Skill: Message

**Crate:** `skill-message` · **Impl:** `MacOsMessagesSkill`

**Purpose:** Resolve a natural-language contact phrase (for example, `"my wife"`) via native macOS Contacts APIs and send an iMessage through Messages.app.

---

## Full Journey

```mermaid
sequenceDiagram
    participant LLM as IntentLLM
    participant Runtime as DesktopRuntime
    participant Skill as MessageSkill
    participant NativeContacts as ContactsFramework
    participant Messages as MessagesApp
    participant Composer as AnswerComposerLLM
    participant TTS

    LLM->>Runtime: SkillMessage { message_contact, message_text }
    Runtime->>Skill: execute(contact, message)
    Skill->>Skill: increment message_skill_execute_total
    Skill->>Skill: start message_skill_execute_duration_seconds timer

    alt dry_run
        Skill-->>Runtime: MessageResult (canned)
    else live mode
        Skill->>NativeContacts: query CNContactStore for contact phrase
        Skill->>Skill: cache resolved phrase -> contact mapping (TTL)
        alt contact found
            Skill->>Messages: send iMessage(message) to resolved handle
        else contact missing
            Skill->>Skill: increment message_skill_errors_total{error_kind}
            Skill-->>Runtime: Err(ContactNotFound)
        end
        alt send succeeds
            Messages-->>Skill: ok
            Skill-->>Runtime: MessageResult { summary, recipient_name, recipient_handle, message }
        else send fails
            Skill->>Skill: increment message_skill_errors_total{error_kind}
            Skill-->>Runtime: Err(SendFailed)
        end
    end

    alt result ok
        Runtime->>Composer: skill_answer_prompt + to_prompt_context()
        Composer-->>TTS: short spoken confirmation
    else ContactNotFound
        Runtime-->>TTS: "I'm sorry, I couldn't tell who '<contact>' is."
    else error
        Runtime-->>TTS: deterministic send/unavailable failure sentence
    end
```

---

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `contact` | `&str` | Contact phrase to resolve (for example, `"my wife"`, `"Jane Doe"`). |
| `message` | `&str` | Final text to send as an iMessage. |

## Outputs

`MessageResult { summary, recipient_name, recipient_handle, message }`

## Failure Paths

| Error | Cause |
|-------|-------|
| `ContactNotFound` | Contact phrase not found in Contacts query results. |
| `SendFailed` | Messages.app AppleScript send failed for the resolved handle. |
| `Execution` | Input validation failure or AppleScript execution setup failure. |
| `Unavailable` | macOS-only integration invoked on a non-macOS platform. |

## Notes

- Contacts lookup uses native `CNContactStore` (via helper), not Contacts AppleScript.
- Results are cached per resolved phrase with TTL refresh.
- Matching strips common prefixes like `"my "` and `"the "`.
- `new_for_tests()` runs in dry-run mode and does not call Messages.app.
- Runtime intentionally uses deterministic spoken errors for all message-skill failures, so this flow never asks the user for a phone number.

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| `message_skill_execute_total` | Counter | `result=success|error` |
| `message_skill_errors_total` | Counter | `error_kind=<error string>` |
| `message_skill_execute_duration_seconds` | Histogram | *(none)* |
