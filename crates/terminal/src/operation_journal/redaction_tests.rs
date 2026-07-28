use super::{
    OPERATION_PAYLOAD_PREVIEW_CHARS, OperationPayloadCompleteness, OperationPayloadFormat,
    RedactedOperationPayload, SensitiveOperationPayload,
};
use serde_json::{Value, json};

#[test]
fn opaque_payloads_never_preserve_or_debug_raw_bytes() {
    let secret = b"password=plain-secret\n".to_vec();
    let payload = SensitiveOperationPayload::opaque(secret.clone());

    let debug = format!("{payload:?}");
    assert!(debug.contains("byte_len"));
    assert!(!debug.contains("plain-secret"));

    let redacted = payload.redact();
    assert_eq!(redacted.format(), OperationPayloadFormat::OpaqueSummary);
    assert_eq!(
        redacted.completeness(),
        OperationPayloadCompleteness::SummaryOnly
    );
    assert_eq!(redacted.original_byte_len(), secret.len() as u64);
    assert!(redacted.redaction_applied());
    assert!(redacted.structured_value().is_none());
    assert!(redacted.preview().contains("opaque payload redacted"));

    let serialized = serde_json::to_string(&redacted).expect("serialize redacted payload");
    assert!(!serialized.contains("plain-secret"));
}

#[test]
fn structured_payloads_redact_sensitive_keys_recursively() {
    let redacted = SensitiveOperationPayload::structured(json!({
        "headers": {
            "Authorization": "Bearer secret-token",
            "cookie": "sid=abc"
        },
        "nested": [
            {"password": "p@ss"},
            {"api_key": "key-123"},
            {"private-key": "private-key-body"},
            {"access_key": "access-key-value"},
            {"query": "select 1"}
        ]
    }))
    .redact();

    assert_eq!(redacted.format(), OperationPayloadFormat::StructuredJson);
    assert_eq!(
        redacted.completeness(),
        OperationPayloadCompleteness::Redacted
    );
    assert!(redacted.redaction_applied());

    let value = redacted
        .structured_value()
        .expect("structured redacted value");
    assert_eq!(value["headers"]["Authorization"], "***");
    assert_eq!(value["headers"]["cookie"], "***");
    assert_eq!(value["nested"][0]["password"], "***");
    assert_eq!(value["nested"][1]["api_key"], "***");
    assert_eq!(value["nested"][2]["private-key"], "***");
    assert_eq!(value["nested"][3]["access_key"], "***");
    assert_eq!(value["nested"][4]["query"], "select 1");

    let serialized = serde_json::to_string(&redacted).expect("serialize redacted payload");
    for secret in [
        "secret-token",
        "sid=abc",
        "p@ss",
        "key-123",
        "private-key-body",
        "access-key-value",
    ] {
        assert!(!serialized.contains(secret));
    }
}

#[test]
fn structured_arguments_are_redacted_or_replaced_conservatively() {
    let redacted = SensitiveOperationPayload::structured(json!({
        "valid": {
            "arguments": json!({
                "token": "nested-secret",
                "safe": "visible"
            }).to_string()
        },
        "invalid": {
            "arguments": "password=opaque-secret"
        }
    }))
    .redact();

    let value = redacted
        .structured_value()
        .expect("structured redacted value");
    let valid_arguments = value["valid"]["arguments"]
        .as_str()
        .expect("valid arguments remain a JSON string");
    let valid_arguments: Value =
        serde_json::from_str(valid_arguments).expect("parse redacted arguments");
    assert_eq!(valid_arguments["token"], "***");
    assert_eq!(valid_arguments["safe"], "visible");

    let invalid_arguments = value["invalid"]["arguments"]
        .as_str()
        .expect("invalid arguments become a marker");
    assert!(invalid_arguments.contains("non-json arguments redacted"));

    let serialized = serde_json::to_string(&redacted).expect("serialize redacted payload");
    assert!(!serialized.contains("nested-secret"));
    assert!(!serialized.contains("opaque-secret"));
}

#[test]
fn structured_arguments_only_preserve_json_objects() {
    for arguments in [
        json!("token=scalar-secret").to_string(),
        json!(["--password", "array-secret"]).to_string(),
    ] {
        let redacted =
            SensitiveOperationPayload::structured(json!({"arguments": arguments})).redact();
        let arguments = redacted
            .structured_value()
            .and_then(|value| value["arguments"].as_str())
            .expect("non-object arguments become a marker");

        assert!(arguments.contains("non-json arguments redacted"));
        let serialized = serde_json::to_string(&redacted).expect("serialize redacted payload");
        assert!(!serialized.contains("scalar-secret"));
        assert!(!serialized.contains("array-secret"));
    }
}

#[test]
fn non_ascii_structured_keys_are_redacted_conservatively() {
    let redacted =
        SensitiveOperationPayload::structured(json!({"passwоrd": "confusable-secret"})).redact();

    assert_eq!(
        redacted.structured_value().expect("structured payload")["passwоrd"],
        "***"
    );
    assert!(redacted.redaction_applied());
    assert!(
        !serde_json::to_string(&redacted)
            .expect("serialize redacted payload")
            .contains("confusable-secret")
    );
}

#[test]
fn structured_payload_without_sensitive_fields_remains_complete() {
    let original = json!({
        "path": "/tmp/report.txt",
        "operation": "refresh",
        "arguments": {"recursive": false}
    });
    let redacted = SensitiveOperationPayload::structured(original.clone()).redact();

    assert_eq!(
        redacted.completeness(),
        OperationPayloadCompleteness::Complete
    );
    assert!(!redacted.redaction_applied());
    assert_eq!(redacted.structured_value(), Some(&original));
}

#[test]
fn structured_preview_is_bounded_after_redaction() {
    let redacted =
        SensitiveOperationPayload::structured(json!({"safe": "x".repeat(2_000)})).redact();
    let preview = redacted.preview();

    assert!(preview.chars().count() <= OPERATION_PAYLOAD_PREVIEW_CHARS + "…<truncated>".len());
    assert!(preview.ends_with("…<truncated>"));
}

#[test]
fn redacted_payload_deserialization_rejects_unredacted_sensitive_values() {
    let redacted =
        SensitiveOperationPayload::structured(json!({"password": "plain-secret"})).redact();
    let mut value = serde_json::to_value(redacted).expect("serialize redacted payload");
    value["value"]["password"] = json!("restored-secret");

    let error = serde_json::from_value::<RedactedOperationPayload>(value)
        .expect_err("unredacted persisted payload must be rejected");
    assert!(
        error
            .to_string()
            .contains("structured payload contains unredacted sensitive data")
    );
}

#[test]
fn redacted_payload_roundtrips_and_rejects_forged_metadata() {
    let redacted = SensitiveOperationPayload::structured(json!({"token": "plain-secret"})).redact();
    let value = serde_json::to_value(&redacted).expect("serialize redacted payload");
    let decoded = serde_json::from_value::<RedactedOperationPayload>(value.clone())
        .expect("deserialize redacted payload");
    assert_eq!(decoded, redacted);

    let mut forged = value;
    forged["redaction_applied"] = json!(false);
    let error = serde_json::from_value::<RedactedOperationPayload>(forged)
        .expect_err("redaction metadata must match structured content");
    assert!(
        error
            .to_string()
            .contains("structured payload redaction metadata does not match content")
    );
}
