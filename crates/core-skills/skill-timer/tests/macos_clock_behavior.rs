//! Behavioral tests for macOS Clock timer skill (dry-run mode).

use skill_timer::{MacOsClockTimerSkill, TimerSkill};

#[tokio::test]
async fn dry_run_creates_named_timer() {
    let skill = MacOsClockTimerSkill::new_for_tests();
    let result = skill
        .execute("10 minutes", Some("pasta timer"))
        .await
        .unwrap();
    assert_eq!(result.timer_name, "pasta timer");
    assert_eq!(result.duration_seconds, 600);
    assert!(result.duration_display.contains("10 minutes"));
    assert!(result.summary.contains("pasta timer"));
}

#[tokio::test]
async fn dry_run_creates_unnamed_timer_with_ordinal_first() {
    let skill = MacOsClockTimerSkill::new_for_tests();
    // dry_run returns None for active count → ordinal = first
    let result = skill.execute("5 minutes", None).await.unwrap();
    assert_eq!(result.timer_name, "first timer");
    assert_eq!(result.duration_seconds, 300);
}

#[tokio::test]
async fn dry_run_handles_empty_name_as_unnamed() {
    let skill = MacOsClockTimerSkill::new_for_tests();
    let result = skill.execute("30 seconds", Some("")).await.unwrap();
    assert_eq!(result.timer_name, "first timer");
    assert_eq!(result.duration_seconds, 30);
}

#[tokio::test]
async fn dry_run_rejects_invalid_duration() {
    let skill = MacOsClockTimerSkill::new_for_tests();
    let result = skill.execute("soon", None).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("invalid duration"));
}

#[tokio::test]
async fn dry_run_duration_with_hours_and_minutes() {
    let skill = MacOsClockTimerSkill::new_for_tests();
    let result = skill
        .execute("1 hour 30 minutes", Some("long bake"))
        .await
        .unwrap();
    assert_eq!(result.duration_seconds, 5400);
    assert!(result.duration_display.contains("1 hour"));
    assert!(result.duration_display.contains("30 minutes"));
}

#[tokio::test]
async fn to_prompt_context_contains_name_and_duration() {
    let skill = MacOsClockTimerSkill::new_for_tests();
    let result = skill
        .execute("20 minutes", Some("egg timer"))
        .await
        .unwrap();
    let context = result.to_prompt_context();
    assert!(context.contains("egg timer"));
    assert!(context.contains("20 minutes"));
}
