//! Wake-word gate: sensitivity/cooldown behavior.

use core_config::WakeWordConfig;
use core_vad::WakeWordGate;
use std::time::{Duration, Instant};

#[test]
fn when_disabled_should_listen_always_true() {
    let config = WakeWordConfig {
        enabled: false,
        phrases: vec!["computer".to_string()],
        ..Default::default()
    };
    let gate = WakeWordGate::new(config);
    assert!(gate.should_listen(Instant::now()));
}

#[test]
fn when_enabled_without_activation_should_listen_false() {
    let config = WakeWordConfig {
        enabled: true,
        phrases: vec!["computer".to_string()],
        sensitivity: 0.5,
        cooldown_secs: 2,
    };
    let gate = WakeWordGate::new(config);
    assert!(!gate.should_listen(Instant::now()));
}

#[test]
fn when_enabled_empty_phrases_is_not_enabled() {
    let config = WakeWordConfig {
        enabled: true,
        phrases: vec![],
        ..Default::default()
    };
    let gate = WakeWordGate::new(config);
    assert!(!gate.is_enabled());
    assert!(gate.should_listen(Instant::now()));
}

#[test]
fn after_activate_should_listen_true_during_cooldown() {
    let config = WakeWordConfig {
        enabled: true,
        phrases: vec!["computer".to_string()],
        cooldown_secs: 2,
        ..Default::default()
    };
    let mut gate = WakeWordGate::new(config);
    let t0 = Instant::now();
    gate.activate(t0);
    assert!(gate.should_listen(t0));
    assert!(gate.should_listen(t0 + Duration::from_secs(1)));
}

#[test]
fn after_cooldown_should_listen_false_until_next_activation() {
    let config = WakeWordConfig {
        enabled: true,
        phrases: vec!["computer".to_string()],
        cooldown_secs: 1,
        ..Default::default()
    };
    let mut gate = WakeWordGate::new(config);
    let t0 = Instant::now();
    gate.activate(t0);
    assert!(gate.should_listen(t0 + Duration::from_millis(500)));
    assert!(!gate.should_listen(t0 + Duration::from_secs(2)));
    gate.activate(t0 + Duration::from_secs(2));
    assert!(gate.should_listen(t0 + Duration::from_secs(2)));
}

#[test]
fn cooldown_remaining_secs_decreases() {
    let config = WakeWordConfig {
        enabled: true,
        phrases: vec!["hey".to_string()],
        cooldown_secs: 3,
        ..Default::default()
    };
    let mut gate = WakeWordGate::new(config);
    let t0 = Instant::now();
    gate.activate(t0);
    assert!(gate.cooldown_remaining_secs(t0) <= 3);
    assert_eq!(gate.cooldown_remaining_secs(t0 + Duration::from_secs(4)), 0);
}
