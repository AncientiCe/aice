# Skill: Smart Home

**Crate:** `skill-smart-home` · **Impl:** `HueSmartHomeSkill`

**Purpose:** Control Philips Hue lights via the Hue CLIP v2 API. Supports status queries, power toggling, brightness adjustment, and colour temperature changes.

**Execution Owner (Split Runtime):** `aice-backend`

---

## Full Journey

```mermaid
sequenceDiagram
    participant LLM as Intent / LLM
    participant Skill as SmartHomeSkill
    participant Ping as ping_bridge (Hue CLIP v2)
    participant Hue as Hue Bridge (CLIP v2 API)
    participant Composer as AnswerComposerLLM

    LLM->>Skill: execute(target, action)
    Skill->>Skill: normalize_action(action) → canonical action
    alt unsupported / unrecognised action
        Skill-->>LLM: Err(UnsupportedAction)
    end
    Skill->>Ping: GET /clip/v2/resource/light (connectivity check)
    alt bridge unreachable / timeout
        Skill-->>LLM: Err(Timeout)
    end
    Ping-->>Skill: light list
    Skill->>Skill: resolve light by name (fuzzy match target → default_light_name → first light)
    alt action = "status"
        Skill->>Hue: GET /clip/v2/resource/light/<id>
        Hue-->>Skill: power, brightness, mirek
    else action = "on" / "off"
        Skill->>Hue: PUT /clip/v2/resource/light/<id> { on: { on: true/false } }
        Hue-->>Skill: ok
    else action = "brightness_up" / "brightness_down"
        Skill->>Hue: GET current brightness
        Hue-->>Skill: current dimming.brightness
        Skill->>Hue: PUT /clip/v2/resource/light/<id> { dimming: { brightness: current ± 20 } }
        Hue-->>Skill: ok
    else action = "warm" / "cool"
        Skill->>Hue: PUT /clip/v2/resource/light/<id> { color_temperature: { mirek: 400/180 } }
        Hue-->>Skill: ok
    end
    Skill-->>Composer: SmartHomeResult { summary, device_states }
    Composer-->>LLM: to_prompt_context() injected into answer prompt
```

---

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `target` | `Option<&str>` | Light name or room. Falls back to `default_light_name`, then first available light. |
| `action` | `Option<&str>` | Natural language action (e.g. `"turn on"`, `"brighten up"`, `"status"`). |

## Supported Actions (after normalisation)

| Canonical | Natural language aliases |
|-----------|--------------------------|
| `status` | status, what's on |
| `on` | turn on, switch on |
| `off` | turn off, switch off |
| `brightness_up` | brighter, brighten up, increase brightness |
| `brightness_down` | dimmer, dim, decrease brightness |
| `warm` | warm light, warmer |
| `cool` | cool light, cooler |

## Outputs

`SmartHomeResult { summary, device_states: Vec<DeviceState> }`

`DeviceState { id, name, state }` — state formatted as `"power=on, brightness=80%, mirek=250"`.

## Failure Paths

| Error | Cause |
|-------|-------|
| `UnsupportedAction` | Action is not in the supported set after normalisation. |
| `Timeout` | Bridge is unreachable within the request timeout. |
| `Device` | Bridge returns an error for the light operation. |

## Setup / Provisioning

- `discover_bridge()` — calls `https://discovery.meethue.com/` to find bridge IP.
- `create_app_key(bridge_host)` — one-time provisioning; requires the physical bridge link button to be pressed first.

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| *(none instrumented yet — add when touching this skill)* | — | — |
