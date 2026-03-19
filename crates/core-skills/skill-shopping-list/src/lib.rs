//! Shopping list skill: add/remove items in Apple Notes.

mod macos_notes;
mod types;

pub use macos_notes::MacOsNotesShoppingListSkill;
pub use types::{ShoppingListResult, ShoppingListSkill, ShoppingListSkillError};

/// Stub implementation for tests and wiring without a real macOS Notes backend.
pub struct MockShoppingListSkill {
    pub result: Result<ShoppingListResult, ShoppingListSkillError>,
}

impl MockShoppingListSkill {
    pub fn ok(result: ShoppingListResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: ShoppingListSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl ShoppingListSkill for MockShoppingListSkill {
    async fn execute(
        &self,
        _action: &str,
        _items: &str,
        _when: Option<&str>,
    ) -> Result<ShoppingListResult, ShoppingListSkillError> {
        self.result.clone()
    }
}
