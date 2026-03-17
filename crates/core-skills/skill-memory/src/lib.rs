//! Knowledge and memory skill: remember, recall, personal knowledge.

mod sqlite;
mod types;

pub use sqlite::SqliteMemorySkill;
pub use types::{MemoryFact, MemoryResult, MemorySkill, MemorySkillError};

/// Mock implementation retained for existing tests that inject explicit skill outcomes.
pub struct MockMemorySkill {
    pub result: Result<MemoryResult, MemorySkillError>,
}

impl MockMemorySkill {
    pub fn ok(result: MemoryResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: MemorySkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl MemorySkill for MockMemorySkill {
    async fn execute(
        &self,
        _query: Option<&str>,
        _store: Option<bool>,
    ) -> Result<MemoryResult, MemorySkillError> {
        self.result.clone()
    }
}
