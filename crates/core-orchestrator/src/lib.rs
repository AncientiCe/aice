//! Conversation state machine, barge-in, and pipeline orchestration.

mod classifier_contract;
mod engine;
mod intent;
mod search;
mod traits;

pub use classifier_contract::{intent_classifier_few_shots, intent_classifier_system_prompt};
pub use engine::{ConversationEngine, TurnOutcome};
pub use intent::{parse_intent, IntentClassifier, IntentDecision, ParseIntentError};
pub use search::{parse_need_search, NEED_SEARCH_MARKER};
pub use traits::{LlmStream, SttStream, TtsSink};
