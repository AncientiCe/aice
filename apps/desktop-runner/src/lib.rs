//! Desktop runner library: runtime and composition.

pub mod intent_classifier;
pub mod location;
pub mod memory;
pub mod pod_capture;
pub mod routing_tts;
pub mod runtime;
pub mod shutdown;

pub use intent_classifier::LlmIntentClassifier;
pub use location::{
    build_effective_system_prompt, llm_system_prompt_with_location, resolve_startup_location,
};
pub use memory::{Fact, MemoryStore, Turn};
pub use pod_capture::PodIngestCapture;
pub use routing_tts::RoutingTtsSink;
pub use runtime::{
    ContinuousRunOptions, DesktopRuntime, RuntimeLoopStats, RuntimeTurnOutcome, SkillRunContext,
    UserConfirmFn,
};
pub use shutdown::{install_ctrlc_shutdown_handler, install_ctrlc_shutdown_handler_with_cleanup};
