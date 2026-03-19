//! Shopping list skill types and trait.

use async_trait::async_trait;

/// Structured result for LLM to turn into a spoken answer.
#[derive(Clone, Debug)]
pub struct ShoppingListResult {
    pub summary: String,
    pub note_title: String,
    pub added: Vec<String>,
    pub already_present: Vec<String>,
    pub removed: Vec<String>,
    pub not_found: Vec<String>,
}

impl ShoppingListResult {
    /// Format for inclusion in an LLM prompt.
    pub fn to_prompt_context(&self) -> String {
        let mut parts: Vec<String> = vec![self.summary.clone()];
        parts.push(format!("Note: \"{}\".", self.note_title));
        if !self.added.is_empty() {
            parts.push(format!("Added: {}.", self.added.join(", ")));
        }
        if !self.already_present.is_empty() {
            parts.push(format!(
                "Already on list: {}.",
                self.already_present.join(", ")
            ));
        }
        if !self.removed.is_empty() {
            parts.push(format!("Removed: {}.", self.removed.join(", ")));
        }
        if !self.not_found.is_empty() {
            parts.push(format!("Not found: {}.", self.not_found.join(", ")));
        }
        parts.join(" ")
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ShoppingListSkillError {
    #[error("execution error: {0}")]
    Execution(String),
    #[error("invalid action: {0}")]
    InvalidAction(String),
    #[error("notes app unavailable")]
    Unavailable,
}

/// Shopping list skill: add/remove items in an Apple Notes note titled
/// "Shopping List <date>"; `when` defaults to today.
#[async_trait]
pub trait ShoppingListSkill: Send + Sync {
    async fn execute(
        &self,
        action: &str,
        items: &str,
        when: Option<&str>,
    ) -> Result<ShoppingListResult, ShoppingListSkillError>;
}
