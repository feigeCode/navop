use std::fs;

use extension_host::NativeDriverRegistry;

#[test]
fn native_registry_loads_and_filters_multiple_apis() {
    let root = tempfile::TempDir::new().unwrap();
    write_driver(root.path(), "redis", "redis");
    write_driver(root.path(), "mongodb-modern", "mongodb");

    let registry = NativeDriverRegistry::load_from_dir(root.path()).unwrap();

    assert_eq!(2, registry.drivers().len());
    assert!(registry.find("redis", "redis").is_some());
    assert!(registry.find("redis", "mongodb-modern").is_none());
    assert!(registry.find("mongodb", "mongodb-modern").is_some());
}

#[test]
fn native_registry_skips_incompatible_sibling_manifests() {
    let root = tempfile::TempDir::new().unwrap();
    write_driver(root.path(), "redis", "redis");
    write_manifest(
        &root.path().join("legacy-sql"),
        serde_json::json!({
            "id": "legacy-sql",
            "name": "Legacy SQL",
            "entry": { "command": "driver" },
            "transport": { "name": "legacy-sql.sock" },
            "capabilities": { "supports_schema": true }
        }),
    );

    let registry = NativeDriverRegistry::load_from_dir(root.path()).unwrap();

    assert_eq!(1, registry.drivers().len());
    assert!(registry.find("redis", "redis").is_some());
}

#[test]
fn native_registry_treats_legacy_sql_manifest_as_unrelated() {
    let root = tempfile::TempDir::new().unwrap();
    let sql_dir = root.path().join("duckdb");
    write_manifest(
        &sql_dir,
        serde_json::json!({
            "id": "duckdb",
            "name": "DuckDB",
            "entry": { "command": "duckdb-driver" },
            "transport": { "name": "duckdb.sock" },
            "dialect": { "identifier_quote_left": "\"" },
            "capabilities": { "supports_schema": true }
        }),
    );

    let driver = NativeDriverRegistry::load_driver_from_dir(&sql_dir).unwrap();

    assert!(driver.is_none());
}

#[test]
fn native_registry_skips_backup_dirs_with_multiple_manifests() {
    let root = tempfile::TempDir::new().unwrap();
    write_driver(root.path(), "mongodb-modern", "mongodb");
    write_driver(&root.path().join(".backups"), "redis-old", "redis");
    write_driver(
        &root.path().join(".backups"),
        "mongodb-modern-old",
        "mongodb",
    );

    let registry = NativeDriverRegistry::load_from_dir(root.path()).unwrap();

    assert_eq!(1, registry.drivers().len());
    assert!(registry.find("mongodb", "mongodb-modern").is_some());
}

fn write_driver(root: &std::path::Path, id: &str, api: &str) {
    write_manifest(
        &root.join(id),
        serde_json::json!({
            "id": id,
            "name": id,
            "api": api,
            "entry": { "command": "driver" },
            "transport": { "name": format!("{id}.sock") }
        }),
    );
}

fn write_manifest(dir: &std::path::Path, manifest: serde_json::Value) {
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("driver.json"), manifest.to_string()).unwrap();
}
