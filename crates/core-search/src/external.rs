//! External search backend (real API or mock).

use crate::SearchError;
use async_trait::async_trait;

/// Backend for web search. Call only after user confirms.
#[async_trait]
pub trait ExternalSearch: Send + Sync {
    async fn execute(&self, query: &str) -> Result<String, SearchError>;
}

/// Mock implementation for tests; returns a fixed string.
pub struct MockSearch {
    pub result: String,
}

impl MockSearch {
    pub fn new(result: impl Into<String>) -> Self {
        Self {
            result: result.into(),
        }
    }
}

#[async_trait]
impl ExternalSearch for MockSearch {
    async fn execute(&self, query: &str) -> Result<String, SearchError> {
        let _ = query;
        Ok(self.result.clone())
    }
}
