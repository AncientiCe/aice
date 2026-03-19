//! Screenshot skill types and trait.

use async_trait::async_trait;
use std::path::PathBuf;

/// Structured screenshot result used by the answer composer.
#[derive(Clone, Debug)]
pub struct ScreenshotResult {
    pub path: PathBuf,
}

impl ScreenshotResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        format!("Screenshot saved to {}.", self.path.display())
    }
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum ScreenshotSkillError {
    #[error("execution error: {0}")]
    Execution(String),
}

/// Screenshot skill: capture and save a screenshot to local disk.
#[async_trait]
pub trait ScreenshotSkill: Send + Sync {
    async fn execute(
        &self,
        filename: Option<&str>,
    ) -> Result<ScreenshotResult, ScreenshotSkillError>;
}
