//! Computer-use skill types and trait.

use async_trait::async_trait;

/// Structured result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct ComputerResult {
    pub summary: String,
    pub action_done: String,
    pub output: Option<String>,
}

impl ComputerResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        let out = self.output.as_deref().unwrap_or("(no output)");
        format!(
            "{}. Action: {}. Output: {}.",
            self.summary, self.action_done, out
        )
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ComputerSkillError {
    #[error("execution error: {0}")]
    Execution(String),
    #[error("permission denied for action: {0}")]
    PermissionDenied(String),
    #[error("timeout")]
    Timeout,
}

/// Computer-use skill: browser, apps, files; action and optional target from intent.
#[async_trait]
pub trait ComputerSkill: Send + Sync {
    async fn execute(
        &self,
        action: Option<&str>,
        target: Option<&str>,
    ) -> Result<ComputerResult, ComputerSkillError>;
}
