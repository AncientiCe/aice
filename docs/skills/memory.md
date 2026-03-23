# Skill: Memory

**Crate:** `skill-memory` · **Impl:** `SqliteMemorySkill`

**Purpose:** Persistent key-value fact store backed by SQLite with FTS5 full-text search. Supports explicit recall/store via `execute` and proactive fact extraction from every conversation turn via `ingest_turn`.

**Execution Owner (Split Runtime):** `aice-backend`

---

## Full Journey

### Explicit Recall / Store (`execute`)

```mermaid
sequenceDiagram
    participant LLM as Intent / LLM
    participant Skill as MemorySkill
    participant DB as SQLite (memory_facts + FTS5)
    participant Composer as AnswerComposerLLM

    LLM->>Skill: execute(query, store)

    alt store = true OR query starts with "remember" / "note"
        Skill->>Skill: parse "X is Y" or "X: Y" pattern
        Skill->>DB: UPSERT memory_facts (fact_key, fact_value)
        DB->>DB: FTS5 trigger syncs memory_facts_fts
        DB-->>Skill: rows affected
        Skill-->>Composer: MemoryResult { summary, facts, stored=true }
    else recall
        Skill->>DB: FTS5 search (BM25 ranked, LIMIT 5)
        alt FTS5 returns results
            DB-->>Skill: matched MemoryFact rows
        else no FTS5 results
            Skill->>DB: LIKE scan on 100 most-recent facts
            DB-->>Skill: matched rows or empty
        end
        alt no matches found
            Skill-->>LLM: Err(NoMatch)
        end
        Skill-->>Composer: MemoryResult { summary, facts, stored=false }
    end

    Composer-->>LLM: to_prompt_context() injected into answer prompt
```

### Proactive Fact Extraction (`ingest_turn`)

```mermaid
sequenceDiagram
    participant Engine as ConversationEngine
    participant Skill as MemorySkill
    participant DB as SQLite

    Engine->>Skill: ingest_turn(user_text)
    Skill->>DB: INSERT memory_turns (raw text, timestamp)
    Skill->>Skill: scan for patterns ("I prefer …", "my X is Y", etc.)
    loop each matched pattern
        Skill->>DB: UPSERT memory_facts (fact_key, fact_value)
        DB->>DB: FTS5 trigger syncs memory_facts_fts
    end
```

---

## Inputs

| Parameter | Type | Description |
|-----------|------|-------------|
| `query` | `Option<&str>` | Recall search term, or a `"remember X is Y"` store command. |
| `store` | `Option<bool>` | Explicitly request a store operation when `true`. |
| *(ingest)* `user_text` | `&str` | Raw user turn text for proactive extraction. |

## Outputs

`MemoryResult { summary, facts: Vec<MemoryFact>, stored }`

`MemoryFact { key, value, when: Option<String> }`

## Schema

| Table | Purpose |
|-------|---------|
| `memory_facts` | Unique `(fact_key, fact_value)` pairs with timestamp. |
| `memory_turns` | Raw conversation turns for audit and future extraction. |
| `memory_facts_fts` | FTS5 virtual table; kept in sync with `memory_facts` via insert/delete/update triggers. |

## Failure Paths

| Error | Cause |
|-------|-------|
| `Storage` | SQLite write failure. |
| `Retrieval` | SQLite read failure. |
| `NoMatch` | Recall query finds no matching facts. |

## Metrics

| Metric | Kind | Labels |
|--------|------|--------|
| *(none instrumented yet — add when touching this skill)* | — | — |
