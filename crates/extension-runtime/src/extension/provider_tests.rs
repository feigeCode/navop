use std::{fs, sync::Arc};

use super::{
    DatabaseDriverExtensionProvider, ExtensionKind, ExtensionProvider, ExtensionRegistry,
    LanguageExtensionProvider, builtin_registry, load_language_extensions_from_root,
};

#[test]
fn extension_kind_maps_stable_directories() {
    assert_eq!("languages", ExtensionKind::Language.dir_name());
    assert_eq!("database_drivers", ExtensionKind::DatabaseDriver.dir_name());
    assert_eq!("composite", ExtensionKind::Composite.dir_name());
}

#[test]
fn language_provider_lists_installed_language_summaries() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let language_dir = root.join("languages").join("rust");
    fs::create_dir_all(&language_dir).unwrap();
    fs::write(
        language_dir.join("manifest.json"),
        r#"{
            "name": "rust",
            "version": "0.24.0",
            "file_extensions": ["rs", "rsx"]
        }"#,
    )
    .unwrap();
    fs::write(language_dir.join("parser.wasm"), [0u8; 4]).unwrap();

    let mut registry = ExtensionRegistry::new(root);
    registry.register_provider(Arc::new(LanguageExtensionProvider));

    let list = registry
        .list_installed_of(ExtensionKind::Language)
        .expect("language extensions should list");

    assert_eq!(1, list.len());
    assert_eq!(ExtensionKind::Language, list[0].kind);
    assert_eq!("rust", list[0].name);
    assert_eq!("0.24.0", list[0].version);
    assert_eq!(language_dir, list[0].path);
    assert_eq!(
        vec!["rs".to_string(), "rsx".to_string()],
        list[0].file_extensions
    );
    assert!(list[0].description.contains(".rs"));
}

#[test]
fn database_driver_provider_lists_installed_driver_summaries() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let driver_dir = root.join("database_drivers").join("fake_pg");
    fs::create_dir_all(&driver_dir).unwrap();
    fs::write(
        driver_dir.join("driver.json"),
        r#"{
            "id": "fake_pg",
            "name": "Fake PostgreSQL",
            "description": "Test database driver",
            "version": "1.2.3",
            "entry": { "command": "./fake_driver" },
            "transport": { "name": "fake_pg.sock" },
            "ui": {
                "icon": "Database",
                "default_port": 15432
            }
        }"#,
    )
    .unwrap();

    let mut registry = ExtensionRegistry::new(root);
    registry.register_provider(Arc::new(DatabaseDriverExtensionProvider));

    let list = registry
        .list_installed_of(ExtensionKind::DatabaseDriver)
        .expect("database drivers should list");

    assert_eq!(1, list.len());
    assert_eq!(ExtensionKind::DatabaseDriver, list[0].kind);
    assert_eq!("fake_pg", list[0].name);
    assert_eq!("1.2.3", list[0].version);
    assert_eq!("Test database driver", list[0].description);
    assert_eq!(Some("fake_pg"), list[0].driver_id.as_deref());
    assert_eq!(Some("Database"), list[0].icon.as_deref());
    assert_eq!(Some(15432), list[0].default_port);
}

#[test]
fn database_driver_provider_install_from_dir_requires_driver_manifest() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("database_drivers");
    let empty_dir = root.join("empty");
    fs::create_dir_all(&empty_dir).unwrap();

    let provider = DatabaseDriverExtensionProvider;
    let err = provider.install_from_dir(&empty_dir).unwrap_err();

    assert!(err.to_string().contains("driver"));
}

#[test]
fn builtin_registry_registers_all_extension_providers() {
    let tmp = tempfile::TempDir::new().unwrap();
    let registry = builtin_registry(tmp.path().join("extensions"));

    assert!(registry.provider(ExtensionKind::Language).is_some());
    assert!(registry.provider(ExtensionKind::DatabaseDriver).is_some());
    assert!(registry.provider(ExtensionKind::Composite).is_some());
    assert_eq!(
        tmp.path().join("extensions/languages"),
        registry.root_for(ExtensionKind::Language)
    );
}

#[test]
fn load_language_extensions_from_root_scans_languages_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let language_dir = root.join("languages").join("broken");
    fs::create_dir_all(&language_dir).unwrap();
    fs::write(
        language_dir.join("manifest.json"),
        r#"{"name":"broken","version":"0.1.0"}"#,
    )
    .unwrap();
    fs::write(language_dir.join("parser.wasm"), [0u8; 4]).unwrap();

    let report = load_language_extensions_from_root(&root).unwrap();

    assert!(report.loaded.is_empty());
    assert_eq!(1, report.failed.len());
    assert_eq!("broken", report.failed[0].0);
}
