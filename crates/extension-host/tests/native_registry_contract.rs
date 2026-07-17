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

fn write_driver(root: &std::path::Path, id: &str, api: &str) {
    let dir = root.join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("driver.json"),
        serde_json::json!({
            "id": id,
            "name": id,
            "api": api,
            "entry": { "command": "driver" },
            "transport": { "name": format!("{id}.sock") }
        })
        .to_string(),
    )
    .unwrap();
}
