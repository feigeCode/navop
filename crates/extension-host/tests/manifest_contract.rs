use extension_host::{NativeDriverManifest, NativeDriverProcessScope};

#[test]
fn generic_manifest_supports_non_sql_api_and_process_scope() {
    let manifest: NativeDriverManifest = serde_json::from_value(serde_json::json!({
        "id": "mongo-modern",
        "name": "MongoDB",
        "api": "mongodb",
        "entry": {"command": "mongo-driver"},
        "transport": {"name": "mongo.sock"},
        "process": {"scope": "shared"}
    }))
    .expect("generic manifest should deserialize");

    assert_eq!("mongodb", manifest.api);
    assert_eq!(NativeDriverProcessScope::Shared, manifest.process.scope);
    manifest.validate().expect("manifest should validate");

    let session = manifest.process_session_config("1.2.3", "instance-1");
    assert_eq!("mongo-modern", session.label);
    assert_eq!(
        vec![("mongodb".to_string(), "1.0".to_string())],
        session.negotiation.api_offered
    );
}
