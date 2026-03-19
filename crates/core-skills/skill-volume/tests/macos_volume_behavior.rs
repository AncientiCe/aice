//! Behavioral tests for macOS volume skill (dry-run mode).

use skill_volume::{MacOsVolumeSkill, VolumeSkill, VolumeSkillError};

pub trait TestOptionExt<T> {
    fn must(self) -> T;
}

impl<T> TestOptionExt<T> for Option<T> {
    fn must(self) -> T {
        match self {
            Some(value) => value,
            None => panic!("expected Some(..) in test"),
        }
    }
}

pub trait TestResultExt<T, E> {
    fn must(self) -> T;
    fn must_err(self) -> E;
}

impl<T, E: std::fmt::Debug> TestResultExt<T, E> for Result<T, E> {
    fn must(self) -> T {
        match self {
            Ok(value) => value,
            Err(error) => panic!("expected Ok(..) in test, got Err: {:?}", error),
        }
    }

    fn must_err(self) -> E {
        match self {
            Ok(_) => panic!("expected Err(..) in test, got Ok"),
            Err(error) => error,
        }
    }
}

#[tokio::test]
async fn dry_run_set_volume_succeeds() {
    let skill = MacOsVolumeSkill::new_for_tests();
    let result = skill.execute(Some("set"), Some(40)).await.must();
    assert_eq!(result.resulting_level, Some(40));
    assert!(result.summary.contains("40"));
}

#[tokio::test]
async fn dry_run_increase_volume_succeeds() {
    let skill = MacOsVolumeSkill::new_for_tests();
    let result = skill.execute(Some("up"), None).await.must();
    assert_eq!(result.resulting_level, Some(10));
    assert!(result.summary.contains("10"));
}

#[tokio::test]
async fn dry_run_decrease_volume_clamps_at_zero() {
    let skill = MacOsVolumeSkill::new_for_tests();
    let result = skill.execute(Some("down"), None).await.must();
    assert_eq!(result.resulting_level, Some(0));
}

#[tokio::test]
async fn dry_run_mute_succeeds() {
    let skill = MacOsVolumeSkill::new_for_tests();
    let result = skill.execute(Some("mute"), None).await.must();
    assert_eq!(result.resulting_level, None);
    assert!(result.summary.to_lowercase().contains("muted"));
}

#[tokio::test]
async fn dry_run_unmute_succeeds() {
    let skill = MacOsVolumeSkill::new_for_tests();
    let result = skill.execute(Some("unmute"), None).await.must();
    assert_eq!(result.resulting_level, None);
    assert!(result.summary.to_lowercase().contains("unmuted"));
}

#[tokio::test]
async fn dry_run_get_volume_succeeds() {
    let skill = MacOsVolumeSkill::new_for_tests();
    let result = skill.execute(Some("get"), None).await.must();
    assert_eq!(result.resulting_level, Some(50));
    assert!(result.summary.contains("50"));
}

#[tokio::test]
async fn set_rejects_invalid_level() {
    let skill = MacOsVolumeSkill::new_for_tests();
    let result = skill.execute(Some("set"), Some(150)).await;
    assert!(matches!(result, Err(VolumeSkillError::InvalidLevel(150))));
}
