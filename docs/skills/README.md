# Skills Documentation

One document per skill. Each document contains a Mermaid journey diagram, input/output specification, failure paths, and metric names.

| Skill | Crate | Status | Document |
|-------|-------|--------|----------|
| Assistant | `skill-assistant` | Trait only (no macOS impl) | [assistant.md](assistant.md) |
| Computer | `skill-computer` | Trait only (no impl) | [computer.md](computer.md) |
| Distance | `skill-distance` | `OpenMeteoDistanceSkill` | [distance.md](distance.md) |
| Media | `skill-media` | `MacOsMusicSkill` | [media.md](media.md) |
| Memory | `skill-memory` | `SqliteMemorySkill` | [memory.md](memory.md) |
| Reminder | `skill-reminder` | `MacOsReminderSkill` | [reminder.md](reminder.md) |
| Shopping List | `skill-shopping-list` | `MacOsNotesShoppingListSkill` | [shopping-list.md](shopping-list.md) |
| Smart Home | `skill-smart-home` | `HueSmartHomeSkill` | [smart-home.md](smart-home.md) |
| Time | `skill-time` | `OpenMeteoTimeSkill` | [time.md](time.md) |
| Timer | `skill-timer` | `MacOsClockTimerSkill` | [timer.md](timer.md) |
| Weather | `skill-weather` | `OpenMeteoWeatherSkill` | [weather.md](weather.md) |

## Document Structure

Each skill document follows this structure:

1. **Header** — crate name, implementation struct, one-line purpose.
2. **Full Journey** — one or more Mermaid `sequenceDiagram` or `flowchart` diagrams covering the complete request/response path from intent classification through to the answer composer.
3. **Inputs** — parameter table.
4. **Outputs** — result type description.
5. **Failure Paths** — all `Error` variants and their causes.
6. **Notes** — implementation details, platform constraints, edge cases.
7. **Metrics** — every metric emitted, its kind, and its labels.
