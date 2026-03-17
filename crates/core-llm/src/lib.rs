//! Ollama streaming LLM client and tool-call hooks.

pub mod error;
pub mod ollama;

pub use error::LlmError;
pub use ollama::OllamaLlmStream;
