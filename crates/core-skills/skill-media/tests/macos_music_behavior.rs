use skill_media::{MacOsMusicSkill, MediaSkill};

#[tokio::test]
async fn parse_play_action_with_target() {
    let skill = MacOsMusicSkill::new_for_tests();
    if cfg!(target_os = "macos") {
        let result = skill
            .execute(Some("play"), Some("daft punk around the world"))
            .await
            .expect("play command should be accepted");
        assert_eq!(result.state, "playing");
        assert!(result.summary.to_lowercase().contains("play"));
    } else {
        let err = skill
            .execute(Some("play"), Some("daft punk around the world"))
            .await
            .expect_err("non-macos should fail");
        assert!(format!("{err}").contains("requires macOS"));
    }
}

#[tokio::test]
async fn transport_actions_are_supported() {
    let skill = MacOsMusicSkill::new_for_tests();
    for action in ["pause", "resume", "stop", "next", "previous"] {
        if cfg!(target_os = "macos") {
            let result = skill
                .execute(Some(action), None)
                .await
                .expect("transport action should be accepted");
            assert!(!result.summary.is_empty());
        } else {
            let err = skill
                .execute(Some(action), None)
                .await
                .expect_err("non-macos should fail");
            assert!(format!("{err}").contains("requires macOS"));
        }
    }
}

#[tokio::test]
async fn shuffle_actions_are_supported() {
    let skill = MacOsMusicSkill::new_for_tests();
    if cfg!(target_os = "macos") {
        let on = skill
            .execute(Some("shuffle_on"), None)
            .await
            .expect("shuffle_on should be accepted");
        assert_eq!(on.state, "playing");
        let off = skill
            .execute(Some("shuffle_off"), None)
            .await
            .expect("shuffle_off should be accepted");
        assert_eq!(off.state, "playing");
    } else {
        let err = skill
            .execute(Some("shuffle_on"), None)
            .await
            .expect_err("non-macos should fail");
        assert!(format!("{err}").contains("requires macOS"));
    }
}

#[tokio::test]
async fn status_returns_structured_response() {
    let skill = MacOsMusicSkill::new_for_tests();
    if cfg!(target_os = "macos") {
        let result = skill.execute(Some("status"), None).await.expect("status");
        assert!(!result.summary.is_empty());
        assert!(!result.state.is_empty());
    } else {
        let err = skill
            .execute(Some("status"), None)
            .await
            .expect_err("non-macos should fail");
        assert!(format!("{err}").contains("requires macOS"));
    }
}

#[tokio::test]
async fn non_macos_returns_unsupported_error() {
    if cfg!(target_os = "macos") {
        return;
    }
    let skill = MacOsMusicSkill::new_for_tests();
    let err = skill
        .execute(Some("play"), Some("test song"))
        .await
        .expect_err("non-macos should fail");
    assert!(format!("{err}").contains("requires macOS"));
}
