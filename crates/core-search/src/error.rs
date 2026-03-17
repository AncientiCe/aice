use thiserror::Error;

#[derive(Error, Debug)]
pub enum SearchError {
    #[error("search request failed: {0}")]
    Request(String),
}
