# Skill: Computer

**Crate:** `skill-computer` · **Impl:** `MockComputerSkill` (no concrete implementation yet)

**Purpose:** Trait and type scaffolding for computer-use actions — controlling browsers, launching apps, and interacting with files. Defines the interface that a real automation backend (e.g. AppleScript, Accessibility API, or headless browser) will fulfil.

---

## Full Journey

```mermaid
sequenceDiagram
    participant LLM as Intent / LLM
    participant Skill as ComputerSkill
    participant OS as OS Automation (future: AppleScript / Accessibility API)
    participant Composer as AnswerComposerLLM

    LLM->>Skill: execute(action, target)

    alt action involves browser
        Skill->>OS: open / navigate browser to target URL or search
        OS-->>Skill: success or error
    else action involves app launch
        Skill->>OS: open application <target>
        OS-->>Skill: success or error
    else action involves file operation
        Skill->>OS: file system action on <target>
        OS-->>Skill: success or error
    else action not recognised
        Skill-->>LLM: Err(Execution)
    end

    alt OS permission denied
        Skill-->>LLM: Err(PermissionDenied)
    end
    alt OS operation times out
        Skill-->>LLM: Err(Timeout)
    end

    Skill-->>Composer: ComputerResult { summary, action_done, output }
    Composer-->>LLM: to_prompt_context() injected into answer prompt
```

---

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `action` | `Option<&str>` | Automation action to perform (e.g. `"open browser"`, `"launch app"`, `"create file"`). |
| `target` | `Option<&str>` | Subject of the action (URL, app name, file path, etc.). |

## Outputs

`ComputerResult { summary, action_done: bool, output: Option<String> }`

## Failure Paths

| Error | Cause |
|-------|-------|
| `Execution` | Automation backend fails or action is not recognised. |
| `PermissionDenied` | OS denies Accessibility or Automation permission. |
| `Timeout` | Automation operation does not complete within the allowed time. |

## Notes

- No concrete implementation exists yet; only `MockComputerSkill` is available.
- When a real implementation is added, update this document with the actual automation calls and permission requirements.

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| *(none instrumented yet — add when implementing this skill)* | — | — |
