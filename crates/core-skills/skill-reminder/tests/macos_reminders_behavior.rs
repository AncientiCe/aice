//! Behavioral tests for macOS Reminders skill (dry-run mode).

use skill_reminder::{MacOsReminderSkill, ReminderSkill};

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
async fn dry_run_creates_reminder_without_when() {
    let skill = MacOsReminderSkill::new_for_tests();
    let result = skill.execute("Buy groceries", None).await.must();
    assert_eq!(result.title, "Buy groceries");
    assert!(result.when.is_none());
    assert!(result.summary.contains("Buy groceries"));
    assert!(result.summary.contains("without due date"));
}

#[tokio::test]
async fn dry_run_creates_reminder_with_iso_datetime() {
    let skill = MacOsReminderSkill::new_for_tests();
    let result = skill
        .execute("Call mom", Some("2026-03-20T17:00"))
        .await
        .must();
    assert_eq!(result.title, "Call mom");
    assert!(result.when.is_some());
    let when = result.when.must();
    assert!(when.contains("20 Mar 2026"));
    assert!(when.contains("17:00"));
}

#[tokio::test]
async fn dry_run_creates_reminder_with_date_only() {
    let skill = MacOsReminderSkill::new_for_tests();
    let result = skill
        .execute("Team meeting", Some("2026-04-01"))
        .await
        .must();
    assert_eq!(result.title, "Team meeting");
    assert!(result.when.is_some());
    let when = result.when.must();
    assert!(when.contains("01 Apr 2026"));
}

#[tokio::test]
async fn dry_run_returns_error_for_invalid_when() {
    let skill = MacOsReminderSkill::new_for_tests();
    let result = skill.execute("Call mom", Some("not a date")).await;
    assert!(result.is_err());
    let err = result.must_err();
    assert!(err.to_string().contains("invalid due date"));
}

#[tokio::test]
async fn dry_run_returns_error_for_empty_title() {
    let skill = MacOsReminderSkill::new_for_tests();
    let result = skill.execute("", None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn to_prompt_context_includes_title_and_no_date() {
    let skill = MacOsReminderSkill::new_for_tests();
    let result = skill.execute("Buy milk", None).await.must();
    let context = result.to_prompt_context();
    assert!(context.contains("Buy milk"));
    assert!(context.contains("No due date"));
}

#[tokio::test]
async fn to_prompt_context_includes_due_date_when_set() {
    let skill = MacOsReminderSkill::new_for_tests();
    let result = skill
        .execute("Meeting", Some("2026-03-25T09:00"))
        .await
        .must();
    let context = result.to_prompt_context();
    assert!(context.contains("Meeting"));
    assert!(context.contains("Due:"));
    assert!(context.contains("25 Mar 2026"));
}
