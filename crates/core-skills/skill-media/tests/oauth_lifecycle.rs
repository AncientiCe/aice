use skill_media::{AppleMusicAuthConfig, AppleMusicAuthManager};

#[test]
fn auth_config_is_minimal_and_oauth_free() {
    let cfg = AppleMusicAuthConfig {
        team_id: None,
        key_id: None,
        private_key_path: None,
        storefront: "us".to_string(),
    };
    let manager = AppleMusicAuthManager::new_for_tests(cfg);
    let token = manager.developer_token().expect("developer token check");
    assert!(token.is_none());
}
