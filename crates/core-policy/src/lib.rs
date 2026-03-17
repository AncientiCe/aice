//! Autonomy policy engine for Jarvis: risk tiers, allow/deny rules, emergency stop, action budgets.
//!
//! Every side-effecting skill execution should be gated by `PolicyEngine::allow_action`.
//! When `emergency_stop()` is true, no actions are allowed.

mod engine;
mod types;

pub use engine::{skill_id_and_risk, PolicyEngine, StandardPolicyEngine};
pub use types::{ActionRequest, PolicyDecision, RiskTier, SkillId};
