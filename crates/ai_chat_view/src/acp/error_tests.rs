use super::error::{extract_rpc_error_detail, redact_secrets, sanitize_detail};

#[test]
fn extracts_nested_provider_message_and_http_status() {
    let data = serde_json::json!({
        "message": "unexpected status 401 Unauthorized: Invalid token",
        "codexErrorInfo": {
            "responseStreamDisconnected": {"httpStatusCode": 401}
        }
    });

    let detail = extract_rpc_error_detail("Internal error", Some(&data));

    assert!(detail.contains("401"));
    assert!(detail.contains("Invalid token"));
}

#[test]
fn redacts_secret_assignments_and_authorization_headers() {
    let text = "OPENAI_API_KEY=sk-live Authorization: Bearer abc123";

    assert_eq!(
        "OPENAI_API_KEY=[REDACTED] Authorization: [REDACTED]",
        redact_secrets(text)
    );
}

#[test]
fn removes_ansi_before_presenting_error() {
    assert_eq!(
        "authentication failed",
        sanitize_detail("\u{1b}[31mauthentication failed\u{1b}[0m")
    );
}
