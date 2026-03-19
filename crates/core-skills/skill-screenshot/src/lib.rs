//! Screenshot skill: capture and save local screenshots.

mod macos_screenshot;
mod types;

pub use macos_screenshot::MacOsScreenshotSkill;
pub use types::{ScreenshotResult, ScreenshotSkill, ScreenshotSkillError};

/// Mock implementation for tests.
pub struct MockScreenshotSkill {
    pub result: Result<ScreenshotResult, ScreenshotSkillError>,
}

impl MockScreenshotSkill {
    pub fn ok(result: ScreenshotResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: ScreenshotSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl ScreenshotSkill for MockScreenshotSkill {
    async fn execute(
        &self,
        _filename: Option<&str>,
    ) -> Result<ScreenshotResult, ScreenshotSkillError> {
        self.result.clone()
    }
}
