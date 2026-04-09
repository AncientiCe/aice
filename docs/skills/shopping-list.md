# Skill: Shopping List

**Crate:** `skill-shopping-list` · **Impl:** `MacOsNotesShoppingListSkill`

**Purpose:** Add or remove items from a dated shopping list note in macOS Notes.app via AppleScript. Each shopping list is a separate note named `"Shopping List DD Mon YYYY"`.

**Execution Owner (Split Runtime):** External macOS frontend service ([`AncientiCe/aice-macos`](https://github.com/AncientiCe/aice-macos))

---

## Full Journey

```mermaid
sequenceDiagram
    participant LLM as Intent / LLM
    participant Skill as ShoppingListSkill
    participant DateRes as Date Resolver (local)
    participant AS as AppleScript (Notes.app)
    participant Composer as AnswerComposerLLM

    LLM->>Skill: execute(action, items, when)
    Skill->>Skill: increment shopping_list_skill_execute_total{action}
    Skill->>Skill: start shopping_list_skill_execute_duration_seconds timer

    alt action not "add" or "remove"
        Skill-->>LLM: Err(InvalidAction)
    end

    Skill->>DateRes: resolve when (today / tomorrow / ISO date / weekday name)
    DateRes-->>Skill: note_title = "Shopping List DD Mon YYYY"
    Skill->>Skill: parse_items(items) → split on commas and " and "

    Skill->>AS: read note body for note_title
    alt note exists
        AS-->>Skill: existing note body text
    else note not found (AICE_NOTE_NOT_FOUND sentinel)
        AS-->>Skill: empty body
    end

    alt action = "add"
        Skill->>Skill: apply_add(body, items) → append "□ item" lines, skip duplicates
    else action = "remove"
        Skill->>Skill: apply_remove(body, items) → remove matching lines (case-insensitive)
    end

    alt dry_run = false
        Skill->>AS: write updated body (creates note if absent)
        alt AppleScript succeeds
            AS-->>Skill: ok
        else execution error
            Skill->>Skill: increment shopping_list_skill_errors_total{action}
            Skill-->>LLM: Err(Execution)
        end
    end

    Skill->>Skill: record shopping_list_skill_execute_duration_seconds
    Skill-->>Composer: ShoppingListResult { summary, note_title, added, already_present, removed, not_found }
    Composer-->>LLM: to_prompt_context() injected into answer prompt
```

---

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `action` | `&str` | `"add"` or `"remove"`. |
| `items` | `&str` | Comma- and/or `" and "`-separated item names. Oxford comma supported. |
| `when` | `Option<&str>` | Date for the note: `"today"`, `"tomorrow"`, ISO date, or weekday name. Defaults to today. |

## Outputs

`ShoppingListResult { summary, note_title, added, already_present, removed, not_found }`

## Item Format in Notes

| Symbol | Meaning |
|--------|---------|
| `□ item` | Unchecked item |
| `☑ item` | Checked (purchased) item — recognised when reading; not produced by this skill |

## Failure Paths

| Error | Cause |
|-------|-------|
| `InvalidAction` | Action is not `"add"` or `"remove"`. |
| `Execution` | AppleScript fails (Notes.app closed, automation permission denied, etc.). |
| `Unavailable` | macOS Notes integration is not available on this platform. |

## Notes

- Two AppleScript calls per operation: one **read** (returns `AICE_NOTE_NOT_FOUND` if absent) and one **write** (creates the note if it did not exist).
- `apply_add` and `apply_remove` are **pure functions** — no side effects, tested independently.
- `dry_run = true` (used in tests via `new_for_tests()`) skips AppleScript execution.

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| `shopping_list_skill_execute_total` | Counter | `action` |
| `shopping_list_skill_errors_total` | Counter | `action` |
| `shopping_list_skill_execute_duration_seconds` | Histogram | `action` |

