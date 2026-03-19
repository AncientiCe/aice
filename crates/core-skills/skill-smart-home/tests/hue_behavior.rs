use skill_smart_home::{HueSmartHomeSkill, SmartHomeSkill};

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
async fn action_parser_supports_on_off_status() {
    assert_eq!(
        HueSmartHomeSkill::normalize_action("turn on").as_deref(),
        Some("on")
    );
    assert_eq!(
        HueSmartHomeSkill::normalize_action("turn off").as_deref(),
        Some("off")
    );
    assert_eq!(
        HueSmartHomeSkill::normalize_action("status").as_deref(),
        Some("status")
    );
}

#[tokio::test]
async fn rejects_unknown_action() {
    let skill = HueSmartHomeSkill::new_for_tests("http://127.0.0.1:1", "key", "lamp");
    let err = skill.execute(Some("lamp"), Some("dance")).await.must_err();
    assert!(format!("{err}").to_lowercase().contains("unsupported"));
}

#[tokio::test]
async fn real_hue_status_when_env_present() {
    let host = match std::env::var("AICE_HUE_BRIDGE_HOST") {
        Ok(v) => v,
        Err(_) => return,
    };
    let key = match std::env::var("AICE_HUE_APP_KEY") {
        Ok(v) => v,
        Err(_) => return,
    };
    let light = std::env::var("AICE_HUE_LIGHT_NAME")
        .unwrap_or_else(|_| "Philips Hue White & Colour Ambience LED Table Light".to_string());
    let skill = HueSmartHomeSkill::new(&host, &key, &light);
    let result = skill.execute(Some(&light), Some("status")).await.must();
    assert!(!result.device_states.is_empty());
}
