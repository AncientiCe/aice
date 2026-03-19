//! App switcher skill: macOS app focus and control.

mod macos_app_switcher;
mod types;

pub use macos_app_switcher::MacOsAppSwitcherSkill;
pub use types::{AppSwitcherResult, AppSwitcherSkill, AppSwitcherSkillError};

/// Mock implementation for tests.
pub struct MockAppSwitcherSkill {
    pub result: Result<AppSwitcherResult, AppSwitcherSkillError>,
}

impl MockAppSwitcherSkill {
    pub fn ok(result: AppSwitcherResult) -> Self {
        Self { result: Ok(result) }
    }

    pub fn err(e: AppSwitcherSkillError) -> Self {
        Self { result: Err(e) }
    }
}

#[async_trait::async_trait]
impl AppSwitcherSkill for MockAppSwitcherSkill {
    async fn execute(
        &self,
        _action: Option<&str>,
        _target: Option<&str>,
    ) -> Result<AppSwitcherResult, AppSwitcherSkillError> {
        self.result.clone()
    }
}
