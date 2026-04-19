//! Policy engine implementation: emergency stop, allow/deny rules, action budget.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::types::{ActionRequest, PolicyDecision, RiskTier, SkillId};

/// Engine that decides whether an action is allowed (autonomy policy).
pub trait PolicyEngine: Send + Sync {
    /// Returns true if all actions should be blocked (e.g. user-triggered emergency stop).
    fn emergency_stop(&self) -> bool;

    /// Check if the given action is allowed. Call before executing any side-effecting skill.
    fn allow_action(&self, request: &ActionRequest) -> PolicyDecision;

    /// Record that an action was executed (for budget accounting). No-op if no budget.
    fn record_action(&self) {}
}

/// Standard policy: configurable emergency stop, optional allow-list and action budget.
pub struct StandardPolicyEngine {
    emergency_stop: AtomicBool,
    /// Max actions per window; None = unlimited.
    budget_max: Option<u64>,
    budget_used: AtomicU64,
}

impl StandardPolicyEngine {
    pub fn new(budget_max: Option<u64>) -> Self {
        Self {
            emergency_stop: AtomicBool::new(false),
            budget_max,
            budget_used: AtomicU64::new(0),
        }
    }

    /// Set or clear emergency stop (e.g. from voice or API).
    pub fn set_emergency_stop(&self, stop: bool) {
        self.emergency_stop.store(stop, Ordering::SeqCst);
    }

    /// Reset the action budget counter (e.g. at start of a new window).
    pub fn reset_budget(&self) {
        self.budget_used.store(0, Ordering::SeqCst);
    }
}

impl Default for StandardPolicyEngine {
    fn default() -> Self {
        Self::new(None)
    }
}

impl PolicyEngine for StandardPolicyEngine {
    fn emergency_stop(&self) -> bool {
        self.emergency_stop.load(Ordering::SeqCst)
    }

    fn allow_action(&self, request: &ActionRequest) -> PolicyDecision {
        if self.emergency_stop() {
            return PolicyDecision::Deny("emergency stop active".to_string());
        }
        if let Some(max) = self.budget_max {
            let used = self.budget_used.load(Ordering::SeqCst);
            if used >= max {
                return PolicyDecision::Deny("action budget exhausted".to_string());
            }
        }
        // Default: allow all skills. Can be extended with allow/deny lists per SkillId or RiskTier.
        match request.risk_tier {
            RiskTier::Low | RiskTier::Medium | RiskTier::High | RiskTier::Critical => {
                PolicyDecision::Allow
            }
        }
    }

    fn record_action(&self) {
        if self.budget_max.is_some() {
            self.budget_used.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// Maps IntentDecision skill to SkillId and default RiskTier for policy checks.
pub fn skill_id_and_risk(skill_name: &str) -> (SkillId, RiskTier) {
    match skill_name {
        "skill_weather" => (SkillId::Weather, RiskTier::Low),
        "skill_time" => (SkillId::Time, RiskTier::Low),
        "skill_distance" => (SkillId::Distance, RiskTier::Low),
        "skill_smart_home" => (SkillId::SmartHome, RiskTier::Medium),
        "skill_media" => (SkillId::Media, RiskTier::Low),
        "skill_computer" => (SkillId::Computer, RiskTier::High),
        "skill_app_switcher" => (SkillId::AppSwitcher, RiskTier::High),
        "skill_reminder" => (SkillId::Reminder, RiskTier::Medium),
        "skill_message" => (SkillId::Message, RiskTier::High),
        "skill_timer" => (SkillId::Timer, RiskTier::Low),
        "skill_shopping_list" => (SkillId::ShoppingList, RiskTier::Medium),
        "skill_volume" => (SkillId::Volume, RiskTier::Low),
        // --- new skills ---
        "skill_calculator" => (SkillId::Calculator, RiskTier::Low),
        "skill_unit_conversion" => (SkillId::UnitConversion, RiskTier::Low),
        "skill_currency" => (SkillId::Currency, RiskTier::Low),
        "skill_air_quality" => (SkillId::AirQuality, RiskTier::Low),
        "skill_dictionary" => (SkillId::Dictionary, RiskTier::Low),
        "skill_translate" => (SkillId::Translate, RiskTier::Low),
        "skill_calendar" => (SkillId::Calendar, RiskTier::Medium),
        "skill_meeting_notes" => (SkillId::MeetingNotes, RiskTier::Medium),
        "skill_email" => (SkillId::Email, RiskTier::Medium),
        "skill_briefing" => (SkillId::Briefing, RiskTier::Low),
        "skill_journal" => (SkillId::Journal, RiskTier::Low),
        "skill_screen_ocr" => (SkillId::ScreenOcr, RiskTier::Medium),
        _ => (SkillId::Weather, RiskTier::Low), // fallback
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActionRequest, PolicyDecision, PolicyEngine, RiskTier, SkillId, StandardPolicyEngine,
    };

    #[test]
    fn default_engine_allows_action() {
        let engine = StandardPolicyEngine::default();
        let req = ActionRequest {
            skill: SkillId::SmartHome,
            action_hint: Some("turn_off".to_string()),
            risk_tier: RiskTier::Medium,
        };
        assert!(engine.allow_action(&req).is_allowed());
    }

    #[test]
    fn emergency_stop_denies_action() {
        let engine = StandardPolicyEngine::default();
        engine.set_emergency_stop(true);
        let req = ActionRequest {
            skill: SkillId::SmartHome,
            action_hint: None,
            risk_tier: RiskTier::Medium,
        };
        let decision = engine.allow_action(&req);
        assert!(!decision.is_allowed());
        assert_eq!(
            decision,
            PolicyDecision::Deny("emergency stop active".to_string())
        );
    }

    #[test]
    fn budget_exhausted_denies_action() {
        let engine = StandardPolicyEngine::new(Some(2));
        let req = ActionRequest {
            skill: SkillId::Media,
            action_hint: None,
            risk_tier: RiskTier::Low,
        };
        assert!(engine.allow_action(&req).is_allowed());
        engine.record_action();
        assert!(engine.allow_action(&req).is_allowed());
        engine.record_action();
        assert!(!engine.allow_action(&req).is_allowed());
        assert_eq!(
            engine.allow_action(&req),
            PolicyDecision::Deny("action budget exhausted".to_string())
        );
    }
}
