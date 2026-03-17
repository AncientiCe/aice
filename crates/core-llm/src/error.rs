//! LLM errors.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum LlmError {
    #[error("LLM request failed: {0}")]
    Request(String),
}
