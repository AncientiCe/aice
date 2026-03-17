//! Fallback web search: execute only after user confirms.

mod error;
mod external;
mod provider;

pub use error::SearchError;
pub use external::{ExternalSearch, MockSearch};
pub use provider::HttpSearchProvider;
