use super::*;
use std::fs;

#[test]
fn parses_local_socket_transport() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"}}"#,
    )
    .unwrap();

    assert_eq!(manifest.transport.name, "demo.sock");
}

#[test]
fn rejects_missing_transport() {
    let result = serde_json::from_str::<IpcDriverManifest>(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"}}"#,
    );

    assert!(result.is_err());
}

#[test]
fn rejects_local_socket_transport_without_name() {
    let mut manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":""}}"#,
    )
    .unwrap();
    manifest.manifest_dir = PathBuf::from(".");

    assert!(manifest.validate().is_err());
}

#[test]
fn rejects_unknown_protocol_method_names() {
    let mut manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"methods":["schema/colums"]}"#,
    )
    .unwrap();
    manifest.manifest_dir = PathBuf::from(".");

    let err = manifest.validate().unwrap_err();
    assert!(format!("{err}").contains("schema/colums"));
}

#[test]
fn allows_private_extension_method_namespace() {
    let mut manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"methods":["schema/columns","x/demo/profile"]}"#,
    )
    .unwrap();
    manifest.manifest_dir = PathBuf::from(".");

    manifest.validate().unwrap();
}

#[test]
fn scans_driver_manifests() {
    let temp = tempfile::tempdir().unwrap();
    let driver_dir = temp.path().join("demo");
    fs::create_dir(&driver_dir).unwrap();
    fs::write(
        driver_dir.join(DRIVER_MANIFEST_FILE),
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"}}"#,
    )
    .unwrap();

    let registry = IpcDriverRegistry::load_from_dir(temp.path()).unwrap();
    assert_eq!(registry.drivers().len(), 1);
    assert_eq!(registry.find("demo").unwrap().name, "Demo");
}

#[test]
fn scans_single_driver_directory() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join(DRIVER_MANIFEST_FILE),
        r#"{"id":"duckdb","name":"DuckDB","entry":{"command":"./duckdb_driver"},"transport":{"name":"duckdb.sock"}}"#,
    )
    .unwrap();

    let registry = IpcDriverRegistry::load_from_dir(temp.path()).unwrap();

    assert_eq!(registry.drivers().len(), 1);
    assert_eq!(registry.find("duckdb").unwrap().manifest_dir, temp.path());
}

#[test]
fn scans_driver_manifest_with_ui_form_without_capabilities() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join(DRIVER_MANIFEST_FILE),
        r#"{
            "id": "duckdb",
            "name": "DuckDB",
            "entry": { "command": "./duckdb_driver" },
            "transport": { "name": "duckdb.sock" },
            "ui": {
                "form": {
                    "schema_version": 1,
                    "forms": [],
                    "actions": { "actions": [] }
                }
            }
        }"#,
    )
    .unwrap();

    let registry = IpcDriverRegistry::load_from_dir(temp.path()).unwrap();

    assert_eq!(1, registry.drivers().len());
    assert!(registry.find("duckdb").is_some());
}

#[test]
fn load_from_dirs_prioritizes_earlier_driver_ids() {
    let user = tempfile::tempdir().unwrap();
    let bundled = tempfile::tempdir().unwrap();
    write_driver_manifest(user.path(), "duckdb", "duckdb", "User DuckDB");
    write_driver_manifest(bundled.path(), "duckdb", "duckdb", "Bundled DuckDB");
    write_driver_manifest(bundled.path(), "demo", "demo", "Demo");

    let dirs = vec![user.path().to_path_buf(), bundled.path().to_path_buf()];
    let registry = IpcDriverRegistry::load_from_dirs(&dirs).unwrap();

    assert_eq!(registry.drivers().len(), 2);
    assert_eq!(registry.find("duckdb").unwrap().name, "User DuckDB");
    assert_eq!(registry.find("demo").unwrap().name, "Demo");
}

#[test]
fn load_from_dirs_skips_unscannable_roots() {
    let temp = tempfile::tempdir().unwrap();
    let bad_root = temp.path().join("not-a-directory");
    fs::write(&bad_root, "not a directory").unwrap();
    let bundled = tempfile::tempdir().unwrap();
    write_driver_manifest(bundled.path(), "demo", "demo", "Demo");

    let dirs = vec![bad_root, bundled.path().to_path_buf()];
    let registry = IpcDriverRegistry::load_from_dirs(&dirs).unwrap();

    assert_eq!(registry.drivers().len(), 1);
    assert_eq!(registry.find("demo").unwrap().name, "Demo");
}

#[test]
fn default_driver_dirs_only_use_extension_database_drivers_dir() {
    let config_dir = one_core::storage::get_config_dir().unwrap();
    let expected = config_dir.join("extensions").join("database_drivers");

    assert_eq!(vec![expected.clone()], default_driver_dirs());
    assert_eq!(expected, default_driver_dir());
}

#[test]
fn relative_entry_command_prefers_manifest_dir_binary() {
    let manifest_dir = tempfile::tempdir().unwrap();
    let exe_dir = tempfile::tempdir().unwrap();
    fs::write(manifest_dir.path().join("driver"), "manifest binary").unwrap();
    fs::write(exe_dir.path().join("driver"), "exe sibling").unwrap();
    let mut driver = manifest("demo", "Demo");
    driver.manifest_dir = manifest_dir.path().to_path_buf();

    super::entry::resolve_relative_entry_command(&mut driver, exe_dir.path());

    assert_eq!(driver.entry.command, "./driver");
}

#[test]
fn relative_entry_command_falls_back_to_exe_sibling() {
    let manifest_dir = tempfile::tempdir().unwrap();
    let exe_dir = tempfile::tempdir().unwrap();
    let exe_driver = exe_dir.path().join("driver");
    fs::write(&exe_driver, "exe sibling").unwrap();
    let mut driver = manifest("demo", "Demo");
    driver.manifest_dir = manifest_dir.path().to_path_buf();

    super::entry::resolve_relative_entry_command(&mut driver, exe_dir.path());

    assert_eq!(driver.entry.command, exe_driver.to_string_lossy());
}

#[test]
fn parses_top_level_capabilities() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"dialect":{"supports_schema":false},"capabilities":{"supports_schema":true,"supports_functions":true}}"#,
    )
    .unwrap();

    let capabilities = manifest.effective_capabilities();
    assert!(capabilities.supports_schema);
    assert!(capabilities.supports_functions);
}

#[test]
fn falls_back_to_legacy_dialect_capabilities() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"dialect":{"supports_schema":true,"supports_sequences":true}}"#,
    )
    .unwrap();

    let capabilities = manifest.effective_capabilities();
    assert!(capabilities.supports_schema);
    assert!(capabilities.supports_sequences);
    assert!(capabilities.supports_functions);
    assert!(capabilities.supports_procedures);
}

#[test]
fn falls_back_to_legacy_ui_form_capabilities() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"ui":{"form":{"schema_version":1,"capabilities":{"supports_triggers":true},"forms":[],"actions":{"actions":[]}}}}"#,
    )
    .unwrap();

    assert!(manifest.effective_capabilities().supports_triggers);
}

fn write_driver_manifest(root: &Path, dir_name: &str, id: &str, name: &str) {
    let driver_dir = root.join(dir_name);
    fs::create_dir(&driver_dir).unwrap();
    fs::write(
        driver_dir.join(DRIVER_MANIFEST_FILE),
        format!(
            r#"{{"id":"{id}","name":"{name}","entry":{{"command":"./driver"}},"transport":{{"name":"{id}.sock"}}}}"#
        ),
    )
    .unwrap();
}

fn manifest(id: &str, name: &str) -> IpcDriverManifest {
    IpcDriverManifest {
        id: id.to_string(),
        name: name.to_string(),
        description: String::new(),
        version: String::new(),
        entry: IpcDriverEntry {
            command: "./driver".to_string(),
            args: Vec::new(),
            working_dir: None,
        },
        transport: IpcDriverTransport::local_socket(format!("{id}.sock")),
        dialect: Default::default(),
        capabilities: None,
        methods: Vec::new(),
        ui: Default::default(),
        manifest_dir: PathBuf::from("."),
    }
}
