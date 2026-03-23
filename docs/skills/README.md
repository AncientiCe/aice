# Skills Documentation

One document per skill. Each document contains a Mermaid journey diagram, input/output specification, failure paths, and metric names.

| Skill | Crate | Status | Document |
|-------|-------|--------|----------|
| Assistant | `skill-assistant` | Frontend-owned (`aice-macos`); trait only currently | [assistant.md](assistant.md) |
| App Switcher | `skill-app-switcher` | Frontend-owned (`aice-macos`), `MacOsAppSwitcherSkill` | [app-switcher.md](app-switcher.md) |
| Computer | `skill-computer` | Frontend-owned (`aice-macos`), `MacOsComputerSkill` | [computer.md](computer.md) |
| Distance | `skill-distance` | Backend-owned (`aice-backend`), `OpenMeteoDistanceSkill` | [distance.md](distance.md) |
| Media | `skill-media` | Frontend-owned (`aice-macos`), `MacOsMusicSkill` | [media.md](media.md) |
| Message | `skill-message` | Frontend-owned (`aice-macos`), `MacOsMessagesSkill` | [message.md](message.md) |
| Memory | `skill-memory` | Backend-owned (`aice-backend`), `SqliteMemorySkill` | [memory.md](memory.md) |
| Reminder | `skill-reminder` | Frontend-owned (`aice-macos`), `MacOsReminderSkill` | [reminder.md](reminder.md) |
| Screenshot | `skill-screenshot` | Frontend-owned (`aice-macos`), `MacOsScreenshotSkill` | [screenshot.md](screenshot.md) |
| Shopping List | `skill-shopping-list` | Frontend-owned (`aice-macos`), `MacOsNotesShoppingListSkill` | [shopping-list.md](shopping-list.md) |
| Smart Home | `skill-smart-home` | Backend-owned (`aice-backend`), `HueSmartHomeSkill` | [smart-home.md](smart-home.md) |
| Time | `skill-time` | Backend-owned (`aice-backend`), `OpenMeteoTimeSkill` | [time.md](time.md) |
| Timer | `skill-timer` | Frontend-owned (`aice-macos`), `MacOsClockTimerSkill` | [timer.md](timer.md) |
| Volume | `skill-volume` | Frontend-owned (`aice-macos`), `MacOsVolumeSkill` | [volume.md](volume.md) |
| Weather | `skill-weather` | Backend-owned (`aice-backend`), `OpenMeteoWeatherSkill` | [weather.md](weather.md) |

Backend-owned skills (`weather`, `time`, `distance`, `smart-home`, `memory`) emit shared backend metrics:
`backend_skill_execute_total`, `backend_skill_execute_duration_seconds`, and where applicable dependency metrics (`backend_dependency_requests_total`, `backend_dependency_request_duration_seconds`).

## Document Structure

Each skill document follows this structure:

1. **Header** — crate name, implementation struct, one-line purpose.
2. **Full Journey** — one or more Mermaid `sequenceDiagram` or `flowchart` diagrams covering the complete request/response path from intent classification through to the answer composer.
3. **Inputs** — parameter table.
4. **Outputs** — result type description.
5. **Failure Paths** — all `Error` variants and their causes.
6. **Notes** — implementation details, platform constraints, edge cases.
7. **Metrics** — every metric emitted, its kind, and its labels.
