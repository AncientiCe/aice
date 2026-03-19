use skill_media::{MacOsMusicSkill, MediaSkill};

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
async fn parse_play_action_with_target() {
    let skill = MacOsMusicSkill::new_for_tests();
    if cfg!(target_os = "macos") {
        let result = skill
            .execute(Some("play"), Some("daft punk around the world"))
            .await
            .must();
        assert_eq!(result.state, "playing");
        assert!(result.summary.to_lowercase().contains("play"));
    } else {
        let err = skill
            .execute(Some("play"), Some("daft punk around the world"))
            .await
            .must_err();
        assert!(format!("{err}").contains("requires macOS"));
    }
}

#[tokio::test]
async fn transport_actions_are_supported() {
    let skill = MacOsMusicSkill::new_for_tests();
    for action in ["pause", "resume", "stop", "next", "previous"] {
        if cfg!(target_os = "macos") {
            let result = skill.execute(Some(action), None).await.must();
            assert!(!result.summary.is_empty());
        } else {
            let err = skill.execute(Some(action), None).await.must_err();
            assert!(format!("{err}").contains("requires macOS"));
        }
    }
}

#[tokio::test]
async fn shuffle_actions_are_supported() {
    let skill = MacOsMusicSkill::new_for_tests();
    if cfg!(target_os = "macos") {
        let on = skill.execute(Some("shuffle_on"), None).await.must();
        assert_eq!(on.state, "playing");
        let off = skill.execute(Some("shuffle_off"), None).await.must();
        assert_eq!(off.state, "playing");
    } else {
        let err = skill.execute(Some("shuffle_on"), None).await.must_err();
        assert!(format!("{err}").contains("requires macOS"));
    }
}

#[tokio::test]
async fn status_returns_structured_response() {
    let skill = MacOsMusicSkill::new_for_tests();
    if cfg!(target_os = "macos") {
        let result = skill.execute(Some("status"), None).await.must();
        assert!(!result.summary.is_empty());
        assert!(!result.state.is_empty());
    } else {
        let err = skill.execute(Some("status"), None).await.must_err();
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
        .must_err();
    assert!(format!("{err}").contains("requires macOS"));
}
