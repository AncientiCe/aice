//! Contract tests: ExternalSearch implementations return Result<String, SearchError>.

use core_search::{ExternalSearch, HttpSearchProvider, MockSearch, SearchError};

#[tokio::test]
async fn mock_search_returns_configured_result() {
    let search = MockSearch::new("Sunny, 72°F");
    let out = search.execute("weather").await.unwrap();
    assert_eq!(out, "Sunny, 72°F");
}

#[tokio::test]
async fn mock_search_implements_trait_for_any_query() {
    let search = MockSearch::new("result");
    assert!(search.execute("").await.is_ok());
    assert!(search.execute("anything").await.is_ok());
}

#[tokio::test]
async fn http_search_provider_from_options_builds() {
    let provider =
        HttpSearchProvider::from_options("https://api.example.com/search", Some("secret"), 5)
            .unwrap();
    // Execute against invalid/unreachable URL yields request error
    let res = provider.execute("test").await;
    assert!(res.is_err());
    if let Err(SearchError::Request(_)) = res {
    } else {
        panic!("expected Request error");
    }
}

#[tokio::test]
async fn http_search_provider_without_key_builds() {
    let provider =
        HttpSearchProvider::from_options("https://api.example.com/search", None, 1).unwrap();
    let res = provider.execute("q").await;
    assert!(res.is_err());
}
