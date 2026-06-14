use std::{fs, sync::Arc};

use crate::extension::{
    CompositeExtensionProvider, DatabaseDriverExtensionProvider, ExtensionKind, ExtensionRegistry,
    ExtensionSummary, LanguageExtensionProvider,
};
use crate::extension_downloader::{
    MarketplaceManifest, detect_package_kind, install_from_staging_generic,
    install_from_staging_with_high_risk_permissions,
};

#[test]
fn marketplace_manifest_accepts_legacy_languages() {
    let manifest: MarketplaceManifest = serde_json::from_str(
        r#"{
            "release_version": "2026.05",
            "languages": [{
                "name": "rust",
                "version": "0.24.0",
                "description": "Rust syntax",
                "file_extensions": ["rs"],
                "asset_url": "https://example.test/rust.tar.gz"
            }]
        }"#,
    )
    .unwrap();

    let entries = manifest.into_entries();

    assert_eq!(1, entries.len());
    assert_eq!("rust", entries[0].id);
    assert_eq!(ExtensionKind::Language, entries[0].kind);
    assert_eq!("rust", entries[0].name);
    assert_eq!("0.24.0", entries[0].version);
    assert_eq!(vec!["rs".to_string()], entries[0].file_extensions);
}

#[test]
fn detect_package_kind_identifies_language_database_composite() {
    let tmp = tempfile::TempDir::new().unwrap();
    let language_dir = tmp.path().join("language");
    fs::create_dir_all(&language_dir).unwrap();
    fs::write(language_dir.join("manifest.json"), "{}").unwrap();
    fs::write(language_dir.join("parser.wasm"), [0u8; 4]).unwrap();

    let driver_dir = tmp.path().join("driver");
    fs::create_dir_all(&driver_dir).unwrap();
    fs::write(driver_dir.join("driver.json"), "{}").unwrap();

    let composite_dir = tmp.path().join("composite");
    fs::create_dir_all(&composite_dir).unwrap();
    fs::write(composite_dir.join("extension.json"), "{}").unwrap();

    assert_eq!(
        ExtensionKind::Language,
        detect_package_kind(&language_dir).unwrap()
    );
    assert_eq!(
        ExtensionKind::DatabaseDriver,
        detect_package_kind(&driver_dir).unwrap()
    );
    assert_eq!(
        ExtensionKind::Composite,
        detect_package_kind(&composite_dir).unwrap()
    );
}

#[test]
fn install_from_staging_generic_installs_database_driver() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let staging = tmp.path().join("staging");
    fs::create_dir_all(&staging).unwrap();
    write_driver_manifest(&staging, "fake_pg", "Fake PostgreSQL");
    fs::write(staging.join("driver-bin"), b"driver").unwrap();

    let mut registry = ExtensionRegistry::new(root.clone());
    registry.register_provider(Arc::new(DatabaseDriverExtensionProvider));

    let summary = install_from_staging_generic(&staging, &registry, None).unwrap();

    assert_eq!(
        ExtensionSummary::new(
            ExtensionKind::DatabaseDriver,
            "fake_pg",
            "1.2.3",
            root.join("database_drivers").join("fake_pg")
        )
        .with_description("Test database driver")
        .with_driver_id("fake_pg")
        .with_icon("Database")
        .with_default_port(15432),
        summary
    );
    assert!(root.join("database_drivers/fake_pg/driver.json").exists());
    assert!(root.join("database_drivers/fake_pg/driver-bin").exists());
}

#[test]
fn install_from_staging_generic_rejects_composite_high_risk_permissions() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let staging = tmp.path().join("staging");
    fs::create_dir_all(&staging).unwrap();
    fs::write(
        staging.join("extension.json"),
        r#"{
            "schema_version": 1,
            "id": "acme.shell",
            "name": "Shell Runner",
            "version": "1.0.0",
            "engines": { "onetcli": ">=0.0.0" },
            "permissions": ["shell:exec"]
        }"#,
    )
    .unwrap();
    let registry = ExtensionRegistry::new(root);

    let err = install_from_staging_generic(&staging, &registry, Some(ExtensionKind::Composite))
        .unwrap_err();

    assert!(err.to_string().contains("高危权限"));
    assert!(!tmp.path().join("extensions/composite/acme.shell").exists());
}

#[test]
fn install_from_staging_with_permission_approval_allows_high_risk_composite() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let staging = tmp.path().join("staging");
    fs::create_dir_all(&staging).unwrap();
    write_composite_manifest(&staging, "acme.shell", r#""shell:exec""#);
    let mut registry = ExtensionRegistry::new(root.clone());
    registry.register_provider(Arc::new(CompositeExtensionProvider));

    let summary = install_from_staging_with_high_risk_permissions(
        &staging,
        &registry,
        Some(ExtensionKind::Composite),
    )
    .unwrap();
    assert_eq!(ExtensionKind::Composite, summary.kind);
    assert_eq!("acme.shell", summary.name);
    assert!(root.join("composite/acme.shell/extension.json").exists());
}

#[test]
fn install_from_staging_generic_rejects_path_install_name_before_copy() {
    let tmp = tempfile::TempDir::new().unwrap();
    let staging = tmp.path().join("staging");
    fs::create_dir_all(&staging).unwrap();
    write_driver_manifest(&staging, "../bad", "Bad Driver");
    let registry = ExtensionRegistry::new(tmp.path().join("extensions"));

    let err =
        install_from_staging_generic(&staging, &registry, Some(ExtensionKind::DatabaseDriver))
            .unwrap_err();
    assert!(err.to_string().contains("安装名"));
    assert!(!tmp.path().join("extensions").exists());
}

#[test]
fn install_from_staging_generic_removes_target_when_provider_rejects_manifest() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let staging = tmp.path().join("staging");
    fs::create_dir_all(&staging).unwrap();
    fs::write(
        staging.join("driver.json"),
        r#"{
            "id": "broken",
            "name": "Broken Driver"
        }"#,
    )
    .unwrap();

    let mut registry = ExtensionRegistry::new(root.clone());
    registry.register_provider(Arc::new(DatabaseDriverExtensionProvider));

    let err =
        install_from_staging_generic(&staging, &registry, Some(ExtensionKind::DatabaseDriver))
            .unwrap_err();
    assert!(err.to_string().contains("install DatabaseDriver"));
    assert!(!root.join("database_drivers/broken").exists());
}

#[test]
fn install_from_staging_generic_preserves_existing_extension_when_reinstall_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let installed = root.join("database_drivers/fake_pg");
    let staging = tmp.path().join("staging");
    fs::create_dir_all(&installed).unwrap();
    fs::create_dir_all(&staging).unwrap();
    write_driver_manifest(&installed, "fake_pg", "Existing PostgreSQL");
    fs::write(installed.join("driver-bin"), b"old driver").unwrap();
    fs::write(installed.join("old-marker"), b"keep me").unwrap();
    fs::write(
        staging.join("driver.json"),
        r#"{
            "id": "fake_pg",
            "name": "Broken Update"
        }"#,
    )
    .unwrap();

    let mut registry = ExtensionRegistry::new(root.clone());
    registry.register_provider(Arc::new(DatabaseDriverExtensionProvider));

    let err =
        install_from_staging_generic(&staging, &registry, Some(ExtensionKind::DatabaseDriver))
            .unwrap_err();

    assert!(err.to_string().contains("install DatabaseDriver"));
    assert_eq!(
        b"keep me",
        fs::read(installed.join("old-marker")).unwrap().as_slice()
    );
    assert_eq!(
        b"old driver",
        fs::read(installed.join("driver-bin")).unwrap().as_slice()
    );
}

#[test]
fn install_from_staging_generic_removes_language_target_when_wasm_register_fails() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let staging = tmp.path().join("staging");
    fs::create_dir_all(&staging).unwrap();
    fs::write(
        staging.join("manifest.json"),
        r#"{
            "name": "__test_broken_language_install__",
            "version": "0.1.0",
            "file_extensions": ["broken"]
        }"#,
    )
    .unwrap();
    fs::write(staging.join("parser.wasm"), [0u8; 4]).unwrap();

    let mut registry = ExtensionRegistry::new(root.clone());
    registry.register_provider(Arc::new(LanguageExtensionProvider));

    let err = install_from_staging_generic(&staging, &registry, Some(ExtensionKind::Language))
        .unwrap_err();

    assert!(err.to_string().contains("install Language"));
    assert!(
        !root
            .join("languages/__test_broken_language_install__")
            .exists()
    );
}

fn write_driver_manifest(dir: &std::path::Path, id: &str, name: &str) {
    fs::write(
        dir.join("driver.json"),
        format!(
            r#"{{
                "id": "{id}",
                "name": "{name}",
                "description": "Test database driver",
                "version": "1.2.3",
                "entry": {{ "command": "./driver-bin" }},
                "transport": {{ "name": "{id}.sock" }},
                "ui": {{
                    "icon": "Database",
                    "default_port": 15432
                }}
            }}"#
        ),
    )
    .unwrap();
}

fn write_composite_manifest(dir: &std::path::Path, id: &str, permission: &str) {
    fs::write(
        dir.join("extension.json"),
        format!(
            r#"{{
                "schema_version": 1,
                "id": "{id}",
                "name": "Shell Runner",
                "version": "1.0.0",
                "engines": {{ "onetcli": ">=0.0.0" }},
                "permissions": [{permission}]
            }}"#
        ),
    )
    .unwrap();
}
