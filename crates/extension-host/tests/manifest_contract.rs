use extension_host::{NativeDriverManifest, NativeDriverProcessScope};

#[test]
fn shipped_native_manifests_are_generic_and_have_executable_entries() {
    let manifests = [
        (
            "redis",
            include_str!("../../../drivers/redis-driver/driver.json"),
            "onetcli-redis-driver",
        ),
        (
            "mongodb-modern",
            include_str!("../../../drivers/mongodb-driver/packages/mongodb-modern/driver.json"),
            "onetcli-mongodb-modern-driver",
        ),
        (
            "mongodb-legacy",
            include_str!("../../../drivers/mongodb-driver/packages/mongodb-legacy/driver.json"),
            "onetcli-mongodb-legacy-driver",
        ),
    ];

    for (id, json, executable) in manifests {
        let manifest: NativeDriverManifest = serde_json::from_str(json).unwrap();
        assert_eq!(id, manifest.id);
        assert!(!manifest.api.is_empty());
        assert!(manifest.entry.command.ends_with(executable));
        assert!(!manifest.methods.is_empty());
    }
}

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
