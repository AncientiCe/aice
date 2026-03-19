//! Policy types: risk tiers, action requests, decisions.

/// Risk tier for an action; used by policy rules.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RiskTier {
    /// Read-only or low-impact (e.g. status query).
    Low,
    /// Reversible or scoped (e.g. turn off one light).
    Medium,
    /// Higher impact (e.g. unlock door, send message).
    High,
    /// Critical (e.g. financial, security-sensitive).
    Critical,
}

/// Identifies a skill for policy checks.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SkillId {
    Weather,
    Time,
    Distance,
    SmartHome,
    Assistant,
    Media,
    Memory,
    Computer,
    Reminder,
    Timer,
    ShoppingList,
}

/// Request to perform an action; passed to the policy engine.
#[derive(Clone, Debug)]
pub struct ActionRequest {
    pub skill: SkillId,
    pub action_hint: Option<String>,
    pub risk_tier: RiskTier,
}

/// Result of a policy check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyDecision {
    /// Action is allowed.
    Allow,
    /// Action is denied (reason for logging/UX).
    Deny(String),
}

impl PolicyDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, PolicyDecision::Allow)
    }
}
