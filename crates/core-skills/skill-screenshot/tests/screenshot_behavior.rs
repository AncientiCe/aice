//! Behavioral tests for macOS screenshot skill (dry-run mode).

use skill_screenshot::{MacOsScreenshotSkill, ScreenshotSkill};

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
async fn dry_run_screenshot_saves_file() {
    let skill = MacOsScreenshotSkill::new_for_tests();
    let result = skill.execute(None).await.must();
    let path = result.path.to_string_lossy().to_string();
    assert!(path.contains("/Pictures/aice/"));
    assert!(path.ends_with(".png"));
}

#[tokio::test]
async fn dry_run_screenshot_custom_filename_used() {
    let skill = MacOsScreenshotSkill::new_for_tests();
    let result = skill.execute(Some("daily-shot.png")).await.must();
    assert!(result.path.ends_with("daily-shot.png"));
}

#[tokio::test]
async fn dry_run_screenshot_default_filename_pattern() {
    let skill = MacOsScreenshotSkill::new_for_tests();
    let result = skill.execute(None).await.must();
    let file_name = result
        .path
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or("");
    assert!(file_name.starts_with("screenshot-"));
    assert!(file_name.ends_with(".png"));
}
