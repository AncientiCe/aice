//! App switcher skill types and trait.

use async_trait::async_trait;

/// Structured result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct AppSwitcherResult {
    pub summary: String,
    pub action_done: String,
    pub target: Option<String>,
}

impl AppSwitcherResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        let target = self.target.as_deref().unwrap_or("(none)");
        format!(
            "{}. Action: {}. Target: {}.",
            self.summary, self.action_done, target
        )
    }
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum AppSwitcherSkillError {
    #[error("execution error: {0}")]
    Execution(String),
    #[error("unsupported action: {0}")]
    UnsupportedAction(String),
}

/// App switcher skill: macOS app switching and control actions.
#[async_trait]
pub trait AppSwitcherSkill: Send + Sync {
    async fn execute(
        &self,
        action: Option<&str>,
        target: Option<&str>,
    ) -> Result<AppSwitcherResult, AppSwitcherSkillError>;
}
