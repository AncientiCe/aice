use skill_media::{AppleMusicWindowsSkill, MediaSkill};

#[tokio::test]
async fn parse_play_action_with_target() {
    let skill = AppleMusicWindowsSkill::new_for_tests();
    let result = skill
        .execute(Some("play"), Some("daft punk around the world"))
        .await
        .expect("play command should be accepted");
    assert!(result.summary.to_lowercase().contains("play"));
}

#[tokio::test]
async fn unsupported_action_errors() {
    let skill = AppleMusicWindowsSkill::new_for_tests();
    let err = skill
        .execute(Some("shuffle_party"), None)
        .await
        .expect_err("must fail");
    assert!(format!("{err}").to_lowercase().contains("unsupported"));
}
#[tokio::test]
async fn shuffle_on_action_is_supported() {
    let skill = AppleMusicWindowsSkill::new_for_tests();
    let result = skill
        .execute(Some("shuffle_on"), None)
        .await
        .expect("shuffle_on should be accepted");
    assert_eq!(result.state, "playing");
    assert!(result.summary.to_lowercase().contains("shuffle"));
}
#[tokio::test]
async fn shuffle_off_action_is_supported() {
    let skill = AppleMusicWindowsSkill::new_for_tests();
    let result = skill
        .execute(Some("shuffle_off"), None)
        .await
        .expect("shuffle_off should be accepted");
    assert_eq!(result.state, "playing");
    assert!(result.summary.to_lowercase().contains("shuffle"));
}

#[tokio::test]
async fn real_catalog_search_when_network_allowed() {
    if std::env::var("AICE_ENABLE_MEDIA_NET_TEST").is_err() {
        return;
    }
    let skill = AppleMusicWindowsSkill::new();
    let result = skill
        .execute(Some("play"), Some("aerosmith dream on"))
        .await
        .expect("catalog lookup");
    assert_eq!(result.state, "playing");
}
