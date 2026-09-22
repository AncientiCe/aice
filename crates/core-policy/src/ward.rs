//! Non-clinical allow-list for hospital ward requests.

use crate::types::PolicyDecision;

/// Tools a ward pack may execute. Anything else is denied.
pub const WARD_NON_CLINICAL_TOOLS: &[&str] = &[
    "wayfinding",
    "appointment_time",
    "request_porter",
    "request_wheelchair",
    "meal_logistics",
    "get_a_nurse",
];

/// Allow a ward tool only when it is on the non-clinical list.
pub fn decide_ward_tool(tool: &str) -> PolicyDecision {
    if WARD_NON_CLINICAL_TOOLS.contains(&tool) {
        PolicyDecision::Allow
    } else {
        PolicyDecision::Deny(format!(
            "ward tool '{tool}' is outside the non-clinical list"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{decide_ward_tool, PolicyDecision, WARD_NON_CLINICAL_TOOLS};

    #[test]
    fn ward_allows_wayfinding_and_denies_medication() {
        assert_eq!(decide_ward_tool("wayfinding"), PolicyDecision::Allow);
        assert!(WARD_NON_CLINICAL_TOOLS.contains(&"get_a_nurse"));
        assert_eq!(
            decide_ward_tool("prescribe_medication"),
            PolicyDecision::Deny(
                "ward tool 'prescribe_medication' is outside the non-clinical list".to_string()
            )
        );
    }
}
