use extension_protocol::result_ref::ResultRef;

#[test]
fn result_refs_use_explicit_stable_kinds() {
    let cases = [
        (
            ResultRef::Inline {
                value: serde_json::json!({"ok": true}),
            },
            serde_json::json!({"kind": "inline", "value": {"ok": true}}),
        ),
        (
            ResultRef::Blob {
                id: "blob-1".into(),
            },
            serde_json::json!({"kind": "blob", "id": "blob-1"}),
        ),
        (
            ResultRef::EventStream {
                id: "stream-1".into(),
            },
            serde_json::json!({"kind": "event_stream", "id": "stream-1"}),
        ),
    ];

    for (expected, value) in cases {
        assert_eq!(value, serde_json::to_value(&expected).unwrap());
        assert_eq!(
            expected,
            serde_json::from_value::<ResultRef>(value).unwrap()
        );
    }
}
