//! Error Mapping Unit Tests
//!
//! Verified that HTTP status codes and API error messages are correctly mapped to LlmConnectorError.

use llm_connector::core::Protocol;
use llm_connector::error::LlmConnectorError;
use llm_connector::protocols::adapters::openai::OpenAIProtocol;

#[test]
fn test_from_status_code() {
    let err = LlmConnectorError::from_status_code(401, "Unauthorized".to_string());
    assert!(matches!(err, LlmConnectorError::AuthenticationError(_)));

    let err = LlmConnectorError::from_status_code(429, "Too Many Requests".to_string());
    assert!(matches!(err, LlmConnectorError::RateLimitError(_)));
}

#[test]
fn test_openai_error_mapping() {
    let protocol = OpenAIProtocol::new("test");
    let err = protocol.map_error(400, "{\"error\":{\"message\":\"Invalid model\"}}");
    assert!(matches!(err, LlmConnectorError::InvalidRequest(_)));
}
