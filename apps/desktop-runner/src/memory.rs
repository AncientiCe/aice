//! Persistent memory store: recent turns and profile facts for assistant context.

use core_config::MemoryConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// One turn in conversation history.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Turn {
    pub user: String,
    pub assistant: String,
}

/// A stored fact with optional timestamp.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fact {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub ts: Option<u64>,
}

/// On-disk memory file format.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemoryFile {
    #[serde(default)]
    pub recent_turns: Vec<Turn>,
    #[serde(default)]
    pub profile: HashMap<String, String>,
    #[serde(default)]
    pub facts: Vec<Fact>,
}

/// In-memory store with bounded recent turns and facts; load/save from JSON file.
#[derive(Clone, Debug)]
pub struct MemoryStore {
    recent_turns: Vec<Turn>,
    profile: HashMap<String, String>,
    facts: Vec<Fact>,
    max_recent_turns: usize,
    max_facts: usize,
}

impl MemoryStore {
    /// Create an empty store with the given limits.
    pub fn new(limits: &MemoryConfig) -> Self {
        Self {
            recent_turns: Vec::new(),
            profile: HashMap::new(),
            facts: Vec::new(),
            max_recent_turns: limits.max_recent_turns.max(1),
            max_facts: limits.max_facts.max(1),
        }
    }

    /// Load store from path. On missing or invalid file, returns an empty store (graceful degradation).
    pub fn load(path: &Path, limits: &MemoryConfig) -> Self {
        let contents = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => return Self::new(limits),
        };
        let file: MemoryFile = match serde_json::from_str(&contents) {
            Ok(f) => f,
            Err(_) => return Self::new(limits),
        };
        let max_recent = limits.max_recent_turns.max(1);
        let max_facts = limits.max_facts.max(1);
        let recent_turns = file.recent_turns.into_iter().take(max_recent).collect();
        let facts = file.facts.into_iter().take(max_facts).collect();
        Self {
            recent_turns,
            profile: file.profile,
            facts,
            max_recent_turns: max_recent,
            max_facts,
        }
    }

    /// Save store to path using atomic write (write to temp file then rename).
    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        let file = MemoryFile {
            recent_turns: self.recent_turns.clone(),
            profile: self.profile.clone(),
            facts: self.facts.clone(),
        };
        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let parent = path.parent();
        let tmp = match parent {
            Some(p) => p.join(".memory.json.tmp"),
            None => Path::new(".memory.json.tmp").to_path_buf(),
        };
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Append a turn and trim to max_recent_turns (drop oldest).
    pub fn push_turn(&mut self, user: &str, assistant: &str) {
        self.recent_turns.push(Turn {
            user: user.to_string(),
            assistant: assistant.to_string(),
        });
        while self.recent_turns.len() > self.max_recent_turns {
            self.recent_turns.remove(0);
        }
    }

    /// Set a profile fact (e.g. "user_name" -> "Ancie", "unit_system" -> "metric").
    pub fn set_profile(&mut self, key: &str, value: &str) {
        if value.is_empty() {
            self.profile.remove(key);
        } else {
            self.profile.insert(key.to_string(), value.to_string());
        }
    }

    /// Add a fact with timestamp; trim to max_facts if over.
    pub fn add_fact(&mut self, key: &str, value: &str) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.facts.push(Fact {
            key: key.to_string(),
            value: value.to_string(),
            ts: Some(ts),
        });
        while self.facts.len() > self.max_facts {
            self.facts.remove(0);
        }
    }

    /// Return recent turns as (user, assistant) slices for LLM history.
    pub fn recent_turns(&self) -> &[Turn] {
        &self.recent_turns
    }

    /// History in the format expected by LlmStream::chat_stream: &[(String, String)].
    pub fn history(&self) -> Vec<(String, String)> {
        self.recent_turns
            .iter()
            .map(|t| (t.user.clone(), t.assistant.clone()))
            .collect()
    }

    /// Profile map for prompt context.
    pub fn profile(&self) -> &HashMap<String, String> {
        &self.profile
    }

    /// Facts list (e.g. for summarising into prompt).
    pub fn facts(&self) -> &[Fact] {
        &self.facts
    }
}

#[cfg(test)]
mod tests {
    use super::{MemoryConfig, MemoryStore};
    use std::path::Path;

    fn test_limits() -> MemoryConfig {
        MemoryConfig {
            enabled: true,
            path: "memory.json".to_string(),
            max_recent_turns: 3,
            max_facts: 5,
            autosave: false,
            sqlite_path: "memory.sqlite".to_string(),
        }
    }

    #[test]
    fn new_store_is_empty() {
        let limits = test_limits();
        let store = MemoryStore::new(&limits);
        assert!(store.recent_turns().is_empty());
        assert!(store.profile().is_empty());
        assert!(store.facts().is_empty());
        assert!(store.history().is_empty());
    }

    #[test]
    fn push_turn_bounds_recent_turns() {
        let limits = test_limits();
        let mut store = MemoryStore::new(&limits);
        store.push_turn("u1", "a1");
        store.push_turn("u2", "a2");
        store.push_turn("u3", "a3");
        assert_eq!(store.history().len(), 3);
        store.push_turn("u4", "a4");
        assert_eq!(store.history().len(), 3);
        let h = store.history();
        assert_eq!(h[0].0, "u2");
        assert_eq!(h[2].0, "u4");
    }

    #[test]
    fn load_missing_file_returns_empty_store() {
        let limits = test_limits();
        let store = MemoryStore::load(Path::new("nonexistent_memory_12345.json"), &limits);
        assert!(store.recent_turns().is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let limits = test_limits();
        let mut store = MemoryStore::new(&limits);
        store.push_turn("hello", "hi there");
        store.set_profile("user_name", "Ancie");
        let dir = std::env::temp_dir();
        let path = dir.join("aice_memory_roundtrip_test.json");
        store.save(&path).unwrap();
        let loaded = MemoryStore::load(&path, &limits);
        assert_eq!(loaded.history().len(), 1);
        assert_eq!(loaded.history()[0].0, "hello");
        assert_eq!(
            loaded.profile().get("user_name").map(String::as_str),
            Some("Ancie")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_invalid_json_returns_empty_store() {
        let limits = test_limits();
        let dir = std::env::temp_dir();
        let path = dir.join("aice_memory_invalid_test.json");
        std::fs::write(&path, b"{ invalid }").unwrap();
        let store = MemoryStore::load(&path, &limits);
        assert!(store.recent_turns().is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
