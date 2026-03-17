use crate::types::{MemoryFact, MemoryResult, MemorySkill, MemorySkillError};
use async_trait::async_trait;
use chrono::Utc;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct SqliteMemorySkill {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteMemorySkill {
    pub fn new(path: &Path) -> Result<Self, MemorySkillError> {
        let conn = Connection::open(path).map_err(|e| MemorySkillError::Storage(e.to_string()))?;
        Self::from_connection(conn)
    }

    pub fn new_in_memory() -> Result<Self, MemorySkillError> {
        let conn =
            Connection::open_in_memory().map_err(|e| MemorySkillError::Storage(e.to_string()))?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, MemorySkillError> {
        let skill = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        skill.init_schema()?;
        Ok(skill)
    }

    fn init_schema(&self) -> Result<(), MemorySkillError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| MemorySkillError::Storage("lock poisoned".to_string()))?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS memory_facts (
                id INTEGER PRIMARY KEY,
                fact_key TEXT NOT NULL,
                fact_value TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_facts_key_value
            ON memory_facts(fact_key, fact_value);

            CREATE TABLE IF NOT EXISTS memory_turns (
                id INTEGER PRIMARY KEY,
                user_text TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS memory_facts_fts USING fts5(fact_key, fact_value, content='memory_facts', content_rowid='id');

            CREATE TRIGGER IF NOT EXISTS memory_facts_ai AFTER INSERT ON memory_facts BEGIN
                INSERT INTO memory_facts_fts(rowid, fact_key, fact_value) VALUES (new.id, new.fact_key, new.fact_value);
            END;
            CREATE TRIGGER IF NOT EXISTS memory_facts_ad AFTER DELETE ON memory_facts BEGIN
                INSERT INTO memory_facts_fts(memory_facts_fts, rowid, fact_key, fact_value) VALUES ('delete', old.id, old.fact_key, old.fact_value);
            END;
            CREATE TRIGGER IF NOT EXISTS memory_facts_au AFTER UPDATE ON memory_facts BEGIN
                INSERT INTO memory_facts_fts(memory_facts_fts, rowid, fact_key, fact_value) VALUES ('delete', old.id, old.fact_key, old.fact_value);
                INSERT INTO memory_facts_fts(rowid, fact_key, fact_value) VALUES (new.id, new.fact_key, new.fact_value);
            END;
            ",
        )
        .map_err(|e| MemorySkillError::Storage(e.to_string()))?;
        Ok(())
    }

    fn store_fact(&self, key: &str, value: &str) -> Result<(), MemorySkillError> {
        let ts = Utc::now().to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|_| MemorySkillError::Storage("lock poisoned".to_string()))?;
        conn.execute(
            "
            INSERT INTO memory_facts(fact_key, fact_value, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?3)
            ON CONFLICT(fact_key, fact_value)
            DO UPDATE SET updated_at = excluded.updated_at
            ",
            params![key, value, ts],
        )
        .map_err(|e| MemorySkillError::Storage(e.to_string()))?;
        Ok(())
    }

    fn query_facts(&self, query: &str) -> Result<Vec<MemoryFact>, MemorySkillError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| MemorySkillError::Storage("lock poisoned".to_string()))?;
        let mut items = Vec::new();

        let mut stmt = conn
            .prepare(
                "
                SELECT mf.fact_key, mf.fact_value, mf.updated_at
                FROM memory_facts_fts fts
                JOIN memory_facts mf ON mf.id = fts.rowid
                WHERE memory_facts_fts MATCH ?1
                ORDER BY bm25(memory_facts_fts), mf.updated_at DESC
                LIMIT 5
                ",
            )
            .map_err(|e| MemorySkillError::Retrieval(e.to_string()))?;
        let fts_query = sanitize_fts_query(query);
        let rows = stmt
            .query_map(params![fts_query], |row| {
                Ok(MemoryFact {
                    key: row.get(0)?,
                    value: row.get(1)?,
                    when: row.get::<_, String>(2).ok(),
                })
            })
            .map_err(|e| MemorySkillError::Retrieval(e.to_string()))?;
        for row in rows {
            items.push(row.map_err(|e| MemorySkillError::Retrieval(e.to_string()))?);
        }
        if !items.is_empty() {
            return Ok(items);
        }

        let mut stmt = conn
            .prepare(
                "
                SELECT fact_key, fact_value, updated_at
                FROM memory_facts
                ORDER BY updated_at DESC
                LIMIT 100
                ",
            )
            .map_err(|e| MemorySkillError::Retrieval(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(MemoryFact {
                    key: row.get(0)?,
                    value: row.get(1)?,
                    when: row.get::<_, String>(2).ok(),
                })
            })
            .map_err(|e| MemorySkillError::Retrieval(e.to_string()))?;
        let terms = query_terms(query);
        for row in rows {
            let fact = row.map_err(|e| MemorySkillError::Retrieval(e.to_string()))?;
            if terms.is_empty() {
                items.push(fact);
                continue;
            }
            let hay = format!("{} {}", fact.key.to_lowercase(), fact.value.to_lowercase());
            let score = terms
                .iter()
                .filter(|term| hay.contains(term.as_str()))
                .count();
            if score > 0 {
                items.push(fact);
            }
        }

        if items.is_empty() {
            return Err(MemorySkillError::NoMatch);
        }
        Ok(items)
    }

    fn persist_turn(&self, user_text: &str) -> Result<(), MemorySkillError> {
        let ts = Utc::now().to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|_| MemorySkillError::Storage("lock poisoned".to_string()))?;
        conn.execute(
            "INSERT INTO memory_turns(user_text, created_at) VALUES (?1, ?2)",
            params![user_text, ts],
        )
        .map_err(|e| MemorySkillError::Storage(e.to_string()))?;
        Ok(())
    }

    fn extract_fact_from_query(query: &str) -> Option<(String, String)> {
        let trimmed = query.trim();
        let lowered = trimmed.to_lowercase();
        let body = if let Some(rest) = lowered.strip_prefix("remember that ") {
            &trimmed[(trimmed.len() - rest.len())..]
        } else if let Some(rest) = lowered.strip_prefix("remember ") {
            &trimmed[(trimmed.len() - rest.len())..]
        } else {
            trimmed
        };

        if let Some((k, v)) = split_key_value(body) {
            return Some((normalize_key(&k), v.trim().to_string()));
        }
        Some(("note".to_string(), body.trim().to_string()))
    }

    pub async fn ingest_turn_internal(&self, user_text: &str) -> Result<(), MemorySkillError> {
        self.persist_turn(user_text)?;
        if let Some((key, value)) = extract_turn_fact(user_text) {
            self.store_fact(&key, &value)?;
        }
        Ok(())
    }
}

#[async_trait]
impl MemorySkill for SqliteMemorySkill {
    async fn execute(
        &self,
        query: Option<&str>,
        store: Option<bool>,
    ) -> Result<MemoryResult, MemorySkillError> {
        let query = query.unwrap_or("").trim();
        let should_store = store.unwrap_or(false)
            || query.to_lowercase().starts_with("remember")
            || query.to_lowercase().starts_with("note");

        if should_store {
            let (key, value) = Self::extract_fact_from_query(query)
                .ok_or_else(|| MemorySkillError::Storage("empty memory query".to_string()))?;
            self.store_fact(&key, &value)?;
            return Ok(MemoryResult {
                summary: "Stored memory".to_string(),
                facts: vec![MemoryFact {
                    key,
                    value,
                    when: Some(Utc::now().to_rfc3339()),
                }],
                stored: true,
            });
        }

        let facts = self.query_facts(query)?;
        Ok(MemoryResult {
            summary: "Memory recall".to_string(),
            facts,
            stored: false,
        })
    }

    async fn ingest_turn(&self, user_text: &str) -> Result<(), MemorySkillError> {
        self.ingest_turn_internal(user_text).await
    }
}

fn sanitize_fts_query(query: &str) -> String {
    let terms = query_terms(query);
    if terms.is_empty() {
        "note".to_string()
    } else {
        terms.join(" OR ")
    }
}

fn split_key_value(input: &str) -> Option<(String, String)> {
    if let Some((k, v)) = input.split_once(" is ") {
        return Some((k.trim().to_string(), v.trim().to_string()));
    }
    if let Some((k, v)) = input.split_once(':') {
        return Some((k.trim().to_string(), v.trim().to_string()));
    }
    None
}

fn normalize_key(input: &str) -> String {
    input
        .trim()
        .to_lowercase()
        .replace("my ", "")
        .replace("the ", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
}

fn extract_turn_fact(text: &str) -> Option<(String, String)> {
    let t = text.trim();
    let lower = t.to_lowercase();
    if lower.starts_with("i prefer ") {
        let value = t[9..].trim().to_string();
        return Some(("preference".to_string(), value));
    }
    if lower.starts_with("my ") {
        return split_key_value(t).map(|(k, v)| (normalize_key(&k), v));
    }
    None
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3)
        .filter(|t| !matches!(*t, "what" | "where" | "when" | "with" | "that" | "have"))
        .map(|s| s.to_string())
        .collect()
}
