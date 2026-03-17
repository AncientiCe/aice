//! Media orchestration skill: playback, multi-room, source.

mod apple_windows;
mod auth;
mod musickit_bridge;
mod types;

pub use apple_windows::AppleMusicWindowsSkill;
pub use auth::{AppleMusicAuthConfig, AppleMusicAuthManager};
pub use types::{MediaResult, MediaSkill, MediaSkillError};

/// Mock implementation retained for existing tests that inject explicit outcomes.
pub struct MockMediaSkill {
    pub result: Result<MediaResult, MediaSkillError>,
}

impl MockMediaSkill {
    pub fn ok(result: MediaResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: MediaSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl MediaSkill for MockMediaSkill {
    async fn execute(
        &self,
        _action: Option<&str>,
        _target: Option<&str>,
    ) -> Result<MediaResult, MediaSkillError> {
        self.result.clone()
    }
}
