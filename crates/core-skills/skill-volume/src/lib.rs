//! Volume skill: system output volume controls.

mod macos_volume;
mod types;

pub use macos_volume::MacOsVolumeSkill;
pub use types::{VolumeResult, VolumeSkill, VolumeSkillError};

/// Mock implementation for tests.
pub struct MockVolumeSkill {
    pub result: Result<VolumeResult, VolumeSkillError>,
}

impl MockVolumeSkill {
    pub fn ok(result: VolumeResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: VolumeSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl VolumeSkill for MockVolumeSkill {
    async fn execute(
        &self,
        _action: Option<&str>,
        _level: Option<u8>,
    ) -> Result<VolumeResult, VolumeSkillError> {
        self.result.clone()
    }
}
