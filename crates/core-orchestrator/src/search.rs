//! Fallback search: parse uncertainty marker from LLM response and confirm with user.

/// Marker in LLM response when the model is uncertain and suggests web search.
pub const NEED_SEARCH_MARKER: &str = "[NEED_SEARCH:";

/// Parses "[NEED_SEARCH: query]" from the end of the response.
/// Returns (local_answer_without_marker, query) if present.
pub fn parse_need_search(response: &str) -> Option<(String, String)> {
    let response = response.trim();
    let start = response.find(NEED_SEARCH_MARKER)?;
    let after_marker = response.get((start + NEED_SEARCH_MARKER.len())..)?;
    let end = after_marker.find(']').unwrap_or(after_marker.len());
    let query = after_marker[..end].trim().to_string();
    let local_answer = response[..start].trim().to_string();
    if query.is_empty() {
        return None;
    }
    Some((local_answer, query))
}

#[cfg(test)]
mod tests {
    pub trait TestOptionExt<T> {
        fn must(self) -> T;
    }

    impl<T> TestOptionExt<T> for Option<T> {
        fn must(self) -> T {
            match self {
                Some(value) => value,
                None => panic!("expected Some(..) in test"),
            }
        }
    }

    use super::*;

    #[test]
    fn parse_need_search_extracts_query_and_local_answer() {
        let r = "I'm not sure about that. [NEED_SEARCH: weather in NYC]";
        let (local, query) = parse_need_search(r).must();
        assert_eq!(local, "I'm not sure about that.");
        assert_eq!(query, "weather in NYC");
    }

    #[test]
    fn parse_need_search_none_when_no_marker() {
        assert!(parse_need_search("Just an answer.").is_none());
    }

    #[test]
    fn parse_need_search_none_when_empty_query() {
        assert!(parse_need_search("Uncertain. [NEED_SEARCH: ]").is_none());
    }
}
