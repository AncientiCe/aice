# Skill: Reminder

**Crate:** `skill-reminder` · **Impl:** `MacOsReminderSkill`

**Purpose:** Create reminders in macOS Reminders.app via AppleScript. Supports optional due date/time in ISO 8601 format.

---

## Full Journey

```mermaid
sequenceDiagram
    participant LLM as Intent / LLM
    participant Skill as ReminderSkill
    participant Parser as Date Parser (local)
    participant AS as AppleScript (Reminders.app)
    participant Composer as AnswerComposerLLM

    LLM->>Skill: execute(title, when)
    Skill->>Skill: increment reminder_skill_execute_total
    Skill->>Skill: start reminder_skill_execute_duration_seconds timer

    alt when provided
        Skill->>Parser: parse ISO 8601 ("YYYY-MM-DDTHH:MM" / "YYYY-MM-DD HH:MM:SS" / "YYYY-MM-DD")
        alt parse succeeds
            Parser-->>Skill: NaiveDateTime (date components)
        else parse fails
            Skill-->>LLM: Err(InvalidDate)
        end
    end

    Skill->>Skill: build_create_script(title, date_components)
    note over Skill: AppleScript sets due date and remind me date<br/>via year/month/day/hour/minute construction

    alt dry_run = false
        Skill->>AS: osascript -e <script>
        alt AppleScript succeeds
            AS-->>Skill: ok
        else execution error
            Skill->>Skill: increment reminder_skill_errors_total
            Skill-->>LLM: Err(Execution)
        end
    end

    Skill->>Skill: record reminder_skill_execute_duration_seconds
    Skill-->>Composer: ReminderResult { summary, title, when }
    Composer-->>LLM: to_prompt_context() injected into answer prompt
```

---

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `title` | `&str` | Reminder title text. |
| `when` | `Option<&str>` | Optional due datetime. Accepts `"YYYY-MM-DDTHH:MM"`, `"YYYY-MM-DD HH:MM:SS"`, or `"YYYY-MM-DD"` (midnight). |

## Outputs

`ReminderResult { summary, title, when: Option<String> }`

## Failure Paths

| Error | Cause |
|-------|-------|
| `InvalidDate` | `when` string does not match any accepted ISO 8601 format. |
| `Execution` | AppleScript fails (Reminders.app closed, automation permission denied, etc.). |
| `Unavailable` | macOS Reminders integration is not available on this platform. |

## Notes

- `dry_run = true` (used in tests via `new_for_tests()`) skips AppleScript execution entirely.
- Date construction in AppleScript uses individual components (`year`, `month`, `day`, `hours`, `minutes`) to avoid locale-specific date parsing issues.
- Reminder is created in the **default list** in Reminders.app.

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| `reminder_skill_execute_total` | Counter | *(none)* |
| `reminder_skill_errors_total` | Counter | *(none)* |
| `reminder_skill_execute_duration_seconds` | Histogram | *(none)* |
