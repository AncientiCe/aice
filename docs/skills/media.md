# Skill: Media

**Crate:** `skill-media` · **Impl:** `MacOsMusicSkill`

**Purpose:** Control macOS Music.app playback via AppleScript. Supports play (with library search and iTunes catalog fallback), pause, stop, resume, next/previous track, shuffle, and status queries.

**Execution Owner (Split Runtime):** `aice-macos`

---

## Full Journey

```mermaid
sequenceDiagram
    participant LLM as Intent / LLM
    participant Skill as MediaSkill
    participant AS as AppleScript (Music.app)
    participant iTunes as iTunes Search API
    participant Composer as AnswerComposerLLM

    LLM->>Skill: execute(action, target)
    Skill->>Skill: increment media_skill_execute_total{action}
    Skill->>Skill: start media_skill_execute_duration_seconds timer

    alt action = "play"
        Skill->>AS: search library (playlist name → album exact → album partial → track exact → track partial)
        alt library match found
            AS-->>Skill: play matched item
        else no library match
            Skill->>iTunes: GET https://itunes.apple.com/search?term=<target>
            iTunes-->>Skill: top result
            AS-->>Skill: play iTunes result URL
        end
    else action = "pause" / "stop" / "resume"
        Skill->>AS: pause / stop / play
        AS-->>Skill: ok
    else action = "next" / "previous"
        Skill->>AS: next track / back track
        AS-->>Skill: ok
    else action = "shuffle_on" / "shuffle_off"
        Skill->>AS: set shuffle enabled true/false
        AS-->>Skill: ok
    else action = "status"
        Skill->>AS: get player state + current track info
        AS-->>Skill: state, track, artist
    else unsupported action
        Skill-->>LLM: Err(UnsupportedAction)
    end

    Skill->>AS: read_status() → now_playing, state
    AS-->>Skill: current track/artist or nil
    Skill->>Skill: record media_skill_execute_duration_seconds
    Skill-->>Composer: MediaResult { summary, now_playing, state }
    Composer-->>LLM: to_prompt_context() injected into answer prompt
```

---

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `action` | `Option<&str>` | `play`, `pause`, `stop`, `resume`, `next`, `previous`, `shuffle_on`, `shuffle_off`, `status`. |
| `target` | `Option<&str>` | Search query for `play` (artist, album, track, or playlist name). |

## Outputs

`MediaResult { summary, now_playing: Option<String>, state }`

## Failure Paths

| Error | Cause |
|-------|-------|
| `UnsupportedAction` | Action not in supported set, or running on non-macOS platform. |
| `NoSource` | `play` action given with no `target` and nothing is queued. |
| `Playback` | AppleScript execution fails (Music.app closed, permissions denied, etc.). |
| `Auth` | Automation permission not granted for Music.app. |

## Notes

- macOS-only; returns `UnsupportedAction` immediately on other platforms.
- Library search order: playlist name → exact album → partial album → exact track → partial track.
- iTunes catalog fallback only fires when the library search yields no result.

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| `media_skill_execute_total` | Counter | `action` |
| `media_skill_errors_total` | Counter | `action` |
| `media_skill_execute_duration_seconds` | Histogram | `action` |
