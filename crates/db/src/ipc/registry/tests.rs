use super::*;
use crate::ipc::{DriverAssetSource, DriverResourceLoader};
use gpui::AssetSource;
use one_core::storage::{DatabaseType, DbConnectionConfig};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn parses_local_socket_transport() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"}}"#,
    )
    .unwrap();

    assert_eq!(manifest.transport.name, "demo.sock");
}

#[test]
fn parses_driver_category() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"dm","name":"Dameng DM","category":"domestic_database","entry":{"command":"driver"},"transport":{"name":"dm.sock"}}"#,
    )
    .unwrap();

    assert_eq!(Some("domestic_database"), manifest.category.as_deref());
}

#[test]
fn parses_connection_lifecycle_policy() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{
            "id":"singlefile",
            "name":"SingleFile",
            "entry":{"command":"driver"},
            "transport":{"name":"singlefile.sock"},
            "connection":{
                "single_file":true,
                "single_connection":true,
                "close_on_release":true,
                "path_fields":["host","extra_params.path"]
            }
        }"#,
    )
    .unwrap();

    assert!(manifest.connection.single_file);
    assert!(manifest.connection.single_connection);
    assert!(manifest.connection.close_on_release);
    assert_eq!(
        vec!["host".to_string(), "extra_params.path".to_string()],
        manifest.connection.path_fields
    );
}

#[test]
fn manifest_ignores_legacy_query_extension_contract() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{
          "id":"legacy-query-driver",
          "name":"Legacy Query Driver",
          "entry":{"command":"./legacy_query_driver"},
          "transport":{"name":"legacy-query-driver.sock"},
          "query":{
            "default_language":"legacy_dsl",
            "languages":["legacy_dsl","sql"],
            "table_data_method":"x/legacy/table_data"
          }
        }"#,
    )
    .expect("manifest parses");

    assert_eq!(manifest.id, "legacy-query-driver");
    assert_eq!(manifest.methods, Vec::<String>::new());
}

#[test]
fn manifest_without_query_extension_parses() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{
          "id":"demo",
          "name":"Demo",
          "entry":{"command":"./demo"},
          "transport":{"name":"demo.sock"}
        }"#,
    )
    .expect("manifest parses");

    assert_eq!(manifest.id, "demo");
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
fn scans_valid_driver_manifests_when_sibling_manifest_is_invalid() {
    let temp = tempfile::tempdir().unwrap();
    write_driver_manifest(temp.path(), "valid", "valid", "Valid");
    let broken_dir = temp.path().join("broken");
    fs::create_dir(&broken_dir).unwrap();
    fs::write(
        broken_dir.join(DRIVER_MANIFEST_FILE),
        r#"{"id":"broken","name":"Broken","entry":{"command":"./driver"},"transport":{"name":""}}"#,
    )
    .unwrap();

    let report = IpcDriverRegistry::load_from_dir_with_report(temp.path()).unwrap();
    let registry = report.registry;

    assert_eq!(registry.drivers().len(), 1);
    assert_eq!(registry.find("valid").unwrap().name, "Valid");
    assert_eq!(report.loaded.len(), 1);
    assert_eq!(report.loaded[0].id, "valid");
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].dir, broken_dir);
    assert!(report.skipped[0].error.contains("local socket name"));
}

#[test]
fn scans_single_wrapped_driver_directory() {
    let temp = tempfile::tempdir().unwrap();
    let outer_dir = temp.path().join("gbase8s");
    let driver_dir = outer_dir.join("gbase8s");
    fs::create_dir_all(&driver_dir).unwrap();
    fs::write(
        driver_dir.join(DRIVER_MANIFEST_FILE),
        r#"{"id":"gbase8s","name":"GBase 8s","entry":{"command":"./gbase8s-ipc-driver"},"transport":{"name":"gbase8s.sock"}}"#,
    )
    .unwrap();

    let registry = IpcDriverRegistry::load_from_dir(temp.path()).unwrap();

    assert_eq!(registry.drivers().len(), 1);
    assert_eq!(registry.find("gbase8s").unwrap().manifest_dir, driver_dir);
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
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"dialect":{"supports_schema":false},"capabilities":{"supports_schema":true,"supports_views":false,"supports_functions":true}}"#,
    )
    .unwrap();

    let capabilities = manifest.effective_capabilities();
    assert!(capabilities.supports_schema);
    assert!(!capabilities.supports_views);
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
    assert!(capabilities.supports_views);
    assert!(capabilities.supports_indexes);
    assert!(capabilities.supports_functions);
    assert!(capabilities.supports_procedures);
}

#[test]
fn declared_methods_disable_views_when_schema_views_is_absent() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"methods":["schema/databases","schema/objects"]}"#,
    )
    .unwrap();

    assert!(!manifest.effective_capabilities().supports_views);
}

#[test]
fn declared_methods_disable_indexes_when_schema_indexes_is_absent() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"methods":["schema/databases","schema/objects"]}"#,
    )
    .unwrap();

    assert!(!manifest.effective_capabilities().supports_indexes);
}

#[test]
fn declared_schema_views_method_enables_views() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"methods":["schema/databases","schema/views"]}"#,
    )
    .unwrap();

    assert!(manifest.effective_capabilities().supports_views);
}

#[test]
fn declared_schema_indexes_method_enables_indexes() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"methods":["schema/databases","schema/indexes"]}"#,
    )
    .unwrap();

    assert!(manifest.effective_capabilities().supports_indexes);
}

#[test]
fn explicit_capability_can_disable_views_even_when_method_is_declared() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"methods":["schema/databases","schema/views"],"capabilities":{"supports_views":false}}"#,
    )
    .unwrap();

    assert!(!manifest.effective_capabilities().supports_views);
}

#[test]
fn explicit_capability_can_disable_indexes_even_when_method_is_declared() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"methods":["schema/databases","schema/indexes"],"capabilities":{"supports_indexes":false}}"#,
    )
    .unwrap();

    assert!(!manifest.effective_capabilities().supports_indexes);
}

#[test]
fn falls_back_to_legacy_ui_form_capabilities() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{"id":"demo","name":"Demo","entry":{"command":"python3"},"transport":{"name":"demo.sock"},"ui":{"form":{"schema_version":1,"capabilities":{"supports_triggers":true},"forms":[],"actions":{"actions":[]}}}}"#,
    )
    .unwrap();

    assert!(manifest.effective_capabilities().supports_triggers);
}

#[test]
fn parses_dialect_sql_generation_contract() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{
            "id":"demo",
            "name":"Demo",
            "entry":{"command":"python3"},
            "transport":{"name":"demo.sock"},
            "dialect":{
                "identifier_quote_left":"[",
                "identifier_quote_right":"]",
                "limit_style":"offset_fetch",
                "bool_true":"1",
                "bool_false":"0",
                "explain_template":"EXPLAIN QUERY PLAN {sql}",
                "table_reference_schema_mode":"prefer_schema",
                "row_id_column":"ROWID",
                "row_id_alias":"__rowid__",
                "default_order_by":"ROWID"
            }
        }"#,
    )
    .unwrap();

    assert_eq!(("[", "]"), manifest.dialect.identifier_quote_pair());
    assert_eq!(LimitStyle::OffsetFetch, manifest.dialect.limit_style);
    assert_eq!("1", manifest.dialect.bool_true);
    assert_eq!("0", manifest.dialect.bool_false);
    assert_eq!(
        Some("EXPLAIN QUERY PLAN {sql}"),
        manifest.dialect.explain_template.as_deref()
    );
    assert_eq!(
        TableReferenceSchemaMode::PreferSchema,
        manifest.dialect.table_reference_schema_mode
    );
    assert_eq!(Some("ROWID"), manifest.dialect.row_id_column.as_deref());
    assert_eq!(Some("__rowid__"), manifest.dialect.row_id_alias.as_deref());
    assert_eq!(Some("ROWID"), manifest.dialect.default_order_by.as_deref());
}

#[test]
fn parses_left_identifier_quote_with_bracket_default_right_quote() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{
            "id":"demo",
            "name":"Demo",
            "entry":{"command":"python3"},
            "transport":{"name":"demo.sock"},
            "dialect":{"identifier_quote_left":"["}
        }"#,
    )
    .unwrap();

    assert_eq!(("[", "]"), manifest.dialect.identifier_quote_pair());
}

#[test]
fn parses_compatible_database_type() {
    let manifest: IpcDriverManifest = serde_json::from_str(
        r#"{
            "id":"postgres-compatible",
            "name":"Postgres Compatible",
            "entry":{"command":"./driver"},
            "transport":{"name":"postgres-compatible.sock"},
            "dialect":{"compatible_database_type":"PostgreSQL"}
        }"#,
    )
    .unwrap();

    assert_eq!(
        Some(DatabaseType::PostgreSQL),
        manifest.dialect.compatible_database_type
    );
}

#[test]
fn registry_resolves_external_driver_display_metadata() {
    let mut manifest: IpcDriverManifest = serde_json::from_str(
        r#"{
            "id": "demo",
            "name": "DemoDB",
            "entry": { "command": "driver" },
            "transport": { "name": "demo.sock" },
            "ui": {
                "icon": "icons/demo.svg",
                "icon_color": "icons/demo-color.svg"
            }
        }"#,
    )
    .unwrap();
    manifest.manifest_dir = PathBuf::from("/drivers/demo");

    assert_eq!(
        Some("driver://demo/icon_color.svg".to_string()),
        manifest.preferred_icon_asset_path()
    );
    assert_eq!(
        Some(PathBuf::from("/drivers/demo/icons/demo-color.svg")),
        manifest.preferred_icon_file_path()
    );

    manifest.ui.icon_color = None;
    assert_eq!(
        Some("driver://demo/icon.svg".to_string()),
        manifest.preferred_icon_asset_path()
    );
    assert_eq!(
        Some(PathBuf::from("/drivers/demo/icons/demo.svg")),
        manifest.preferred_icon_file_path()
    );

    manifest.ui.icon = "DuckDB".to_string();
    assert_eq!(
        Some("icons/duckdb.svg".to_string()),
        manifest.preferred_icon_asset_path()
    );
    assert_eq!(None, manifest.preferred_icon_file_path());

    let registry = IpcDriverRegistry::from_drivers(vec![manifest]);
    let config = DbConnectionConfig {
        id: "1".to_string(),
        database_type: DatabaseType::external("demo"),
        name: "saved".to_string(),
        host: "localhost".to_string(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        extra_params: HashMap::new(),
    };
    let display = registry.display_for_config(&config).unwrap();

    assert_eq!("demo", display.driver_id);
    assert_eq!("DemoDB", display.name);
    assert_eq!(
        Some("icons/duckdb.svg".to_string()),
        display.icon_asset_path
    );
    assert_eq!(None, display.icon_file_path);
}

#[test]
fn custom_color_icon_path_is_preferred_over_icon_path() {
    let mut manifest: IpcDriverManifest = serde_json::from_str(
        r#"{
            "id": "demo",
            "name": "DemoDB",
            "entry": { "command": "driver" },
            "transport": { "name": "demo.sock" },
            "ui": { "icon": "icons/demo.svg" }
        }"#,
    )
    .unwrap();
    manifest.manifest_dir = PathBuf::from("/drivers/demo");

    assert_eq!(
        Some("driver://demo/icon.svg".to_string()),
        manifest.preferred_icon_asset_path()
    );
    assert_eq!(
        Some(PathBuf::from("/drivers/demo/icons/demo.svg")),
        manifest.preferred_icon_file_path()
    );

    manifest.ui.icon_color = Some("icons/demo-color.svg".to_string());

    assert_eq!(
        Some("driver://demo/icon_color.svg".to_string()),
        manifest.preferred_icon_asset_path()
    );
    assert_eq!(
        Some(PathBuf::from("/drivers/demo/icons/demo-color.svg")),
        manifest.preferred_icon_file_path()
    );
}

#[test]
fn driver_asset_source_loads_declared_icon_resources() {
    let temp = tempfile::tempdir().unwrap();
    let icons_dir = temp.path().join("icons");
    fs::create_dir(&icons_dir).unwrap();
    fs::write(icons_dir.join("demo.svg"), b"mono").unwrap();
    fs::write(icons_dir.join("demo-color.svg"), b"color").unwrap();

    let mut manifest: IpcDriverManifest = serde_json::from_str(
        r#"{
            "id": "demo",
            "name": "DemoDB",
            "entry": { "command": "driver" },
            "transport": { "name": "demo.sock" },
            "ui": {
                "icon": "icons/demo.svg",
                "icon_color": "icons/demo-color.svg"
            }
        }"#,
    )
    .unwrap();
    manifest.manifest_dir = temp.path().to_path_buf();

    let source = DriverAssetSource::new(
        Arc::new(DriverResourceLoader::new()),
        Arc::new(IpcDriverRegistry::from_drivers(vec![manifest])),
    );

    let mono = source.load("driver://demo/icon").unwrap().unwrap();
    let color = source.load("driver://demo/icon_color").unwrap().unwrap();
    let mono_with_ext = source.load("driver://demo/icon.svg").unwrap().unwrap();
    let color_with_ext = source
        .load("driver://demo/icon_color.svg")
        .unwrap()
        .unwrap();

    assert_eq!(&*mono, b"mono");
    assert_eq!(&*color, b"color");
    assert_eq!(&*mono_with_ext, b"mono");
    assert_eq!(&*color_with_ext, b"color");
}

#[test]
fn driver_asset_source_reloads_registry_for_new_driver_resources() {
    let temp = tempfile::tempdir().unwrap();
    let icons_dir = temp.path().join("icons");
    fs::create_dir(&icons_dir).unwrap();
    fs::write(icons_dir.join("demo.svg"), b"mono").unwrap();

    let mut manifest: IpcDriverManifest = serde_json::from_str(
        r#"{
            "id": "demo",
            "name": "DemoDB",
            "entry": { "command": "driver" },
            "transport": { "name": "demo.sock" },
            "ui": { "icon": "icons/demo.svg" }
        }"#,
    )
    .unwrap();
    manifest.manifest_dir = temp.path().to_path_buf();

    let source = DriverAssetSource::with_registry_reloader(
        Arc::new(DriverResourceLoader::new()),
        Arc::new(IpcDriverRegistry::empty()),
        Arc::new(move || IpcDriverRegistry::from_drivers(vec![manifest.clone()])),
    );

    let mono = source.load("driver://demo/icon").unwrap().unwrap();

    assert_eq!(&*mono, b"mono");
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
        category: None,
        description: String::new(),
        version: String::new(),
        entry: IpcDriverEntry {
            command: "./driver".to_string(),
            commands: Default::default(),
            args: Vec::new(),
            working_dir: None,
            env_from_config: Default::default(),
        },
        transport: IpcDriverTransport::local_socket(format!("{id}.sock")),
        dialect: Default::default(),
        capabilities: None,
        connection: Default::default(),
        methods: Vec::new(),
        ui: Default::default(),
        manifest_dir: PathBuf::from("."),
    }
}
