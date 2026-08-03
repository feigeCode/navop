//! Streaming Logic Tests
//!
//! Verified the SSE parsing and streaming handlers.

use llm_connector::sse::parse_sse_line;

#[test]
fn test_sse_line_parsing() {
    let line = "data: {\"id\":\"1\",\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}";
    let parsed = parse_sse_line(line).unwrap().unwrap();
    assert_eq!(parsed.get("id").unwrap().as_str().unwrap(), "1");
}

#[test]
fn test_sse_done_line() {
    let line = "data: [DONE]";
    let parsed = parse_sse_line(line).unwrap();
    assert!(parsed.is_none());
}
