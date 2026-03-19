//! Behavioral tests for macOS app switcher skill (dry-run mode).

use skill_app_switcher::{AppSwitcherSkill, AppSwitcherSkillError, MacOsAppSwitcherSkill};

pub trait TestResultExt<T, E> {
    fn must(self) -> T;
}

impl<T, E: std::fmt::Debug> TestResultExt<T, E> for Result<T, E> {
    fn must(self) -> T {
        match self {
            Ok(value) => value,
            Err(error) => panic!("expected Ok(..) in test, got Err: {:?}", error),
        }
    }
}

#[tokio::test]
async fn dry_run_switch_target_succeeds() {
    let skill = MacOsAppSwitcherSkill::new_for_tests();
    let result = skill.execute(Some("switch"), Some("Safari")).await.must();
    assert_eq!(result.target.as_deref(), Some("Safari"));
    assert!(result.summary.to_lowercase().contains("switch"));
}

#[tokio::test]
async fn missing_target_for_targeted_action_returns_execution_error() {
    let skill = MacOsAppSwitcherSkill::new_for_tests();
    let result = skill.execute(Some("switch"), None).await;
    assert!(matches!(result, Err(AppSwitcherSkillError::Execution(_))));
}

#[tokio::test]
async fn unsupported_action_returns_error() {
    let skill = MacOsAppSwitcherSkill::new_for_tests();
    let result = skill.execute(Some("teleport"), Some("Safari")).await;
    assert!(matches!(
        result,
        Err(AppSwitcherSkillError::UnsupportedAction(action)) if action == "teleport"
    ));
}

#[tokio::test]
async fn dry_run_quit_action_succeeds() {
    let skill = MacOsAppSwitcherSkill::new_for_tests();
    let result = skill.execute(Some("quit"), Some("Safari")).await.must();
    assert_eq!(result.target.as_deref(), Some("Safari"));
    assert!(result.action_done.to_lowercase().contains("quit"));
}

#[tokio::test]
async fn dry_run_force_quit_action_succeeds() {
    let skill = MacOsAppSwitcherSkill::new_for_tests();
    let result = skill
        .execute(Some("force_quit"), Some("Safari"))
        .await
        .must();
    assert_eq!(result.target.as_deref(), Some("Safari"));
    assert!(result.action_done.to_lowercase().contains("force quit"));
}
