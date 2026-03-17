//! Computer-use skill: browser, apps, files.

mod types;

pub use types::{ComputerResult, ComputerSkill, ComputerSkillError};

/// Mock implementation for tests.
pub struct MockComputerSkill {
    pub result: Result<ComputerResult, ComputerSkillError>,
}

impl MockComputerSkill {
    pub fn ok(result: ComputerResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: ComputerSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl ComputerSkill for MockComputerSkill {
    async fn execute(
        &self,
        _action: Option<&str>,
        _target: Option<&str>,
    ) -> Result<ComputerResult, ComputerSkillError> {
        self.result.clone()
    }
}
