//! Knowledge and memory skill types and trait.

use async_trait::async_trait;

/// Structured result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct MemoryResult {
    pub summary: String,
    pub facts: Vec<MemoryFact>,
    pub stored: bool,
}

/// A recalled or stored fact.
#[derive(Clone, Debug)]
pub struct MemoryFact {
    pub key: String,
    pub value: String,
    pub when: Option<String>,
}

impl MemoryResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        let facts_str: String = self
            .facts
            .iter()
            .map(|f| format!("{}: {}", f.key, f.value))
            .collect::<Vec<_>>()
            .join("; ");
        let stored_note = if self.stored {
            " (new fact stored)"
        } else {
            ""
        };
        format!("{}.{} Facts: [{}].", self.summary, stored_note, facts_str)
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum MemorySkillError {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("retrieval error: {0}")]
    Retrieval(String),
    #[error("no matching facts")]
    NoMatch,
}

/// Knowledge/memory skill: remember (store), recall (query); query and store flag from intent.
#[async_trait]
pub trait MemorySkill: Send + Sync {
    async fn execute(
        &self,
        query: Option<&str>,
        store: Option<bool>,
    ) -> Result<MemoryResult, MemorySkillError>;

    /// Ingest a normal conversational turn for proactive memory extraction.
    async fn ingest_turn(&self, _user_text: &str) -> Result<(), MemorySkillError> {
        Ok(())
    }
}
