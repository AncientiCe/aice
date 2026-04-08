# Memory (superseded — now core infrastructure)

> **Note:** Memory is no longer a skill. The `skill-memory` / `SqliteMemorySkill` crate has been
> replaced by the **Memory Palace** (`mempalace-rs`), which is embedded directly in `aice-backend`
> as core infrastructure. See [§7 Memory Palace in the architecture docs](../architecture/README.md#7-memory-palace-core-persistent-memory)
> for the current design, flow diagrams, inputs, outputs, failure paths, and metrics.

The Memory Palace provides:

- **4-layer semantic memory** (L0 working, L1 episodic, L2 semantic, L3 archival) with 384-dim local embeddings via `fastembed`.
- **Automatic context enrichment** — every chat turn calls `palace.wake_up()` to inject L0/L1 context into the LLM system prompt.
- **Automatic turn ingestion** — every completed chat turn is persisted via `palace.ingest_turn()`.
- **Explicit store/search** — handled directly by the backend when the classifier emits `IntentDecision::SkillMemory`, without routing through the external skill system.
- **Knowledge graph** — entity/relation triples stored alongside memory entries for structured recall.
- **Persistent SQLite storage** — configurable via `memory.palace_db_path` and `memory.palace_identity_path`.

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| `palace_open_total` | Counter | `result` |
| `palace_open_duration_seconds` | Histogram | — |
| `palace_wake_up_total` | Counter | `result` |
| `palace_wake_up_duration_seconds` | Histogram | — |
| `palace_search_total` | Counter | `result` |
| `palace_search_duration_seconds` | Histogram | — |
| `palace_ingest_total` | Counter | `result` |
| `palace_ingest_duration_seconds` | Histogram | — |
| `palace_add_memory_total` | Counter | `result` |
| `palace_add_memory_duration_seconds` | Histogram | — |
| `palace_errors_total` | Counter | `operation` |
