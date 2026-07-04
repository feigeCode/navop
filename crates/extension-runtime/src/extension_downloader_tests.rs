use std::{fs, sync::Arc};

use crate::extension::{
    AcpAgentExtensionProvider, CompositeExtensionProvider, DatabaseDriverExtensionProvider,
    ExtensionKind, ExtensionRegistry, ExtensionSummary, LanguageExtensionProvider,
};
use crate::extension_downloader::{
    MarketplaceManifest, detect_package_kind, install_from_staging_generic,
    install_from_staging_with_high_risk_permissions,
};

#[test]
fn marketplace_manifest_accepts_v2_universal_language_artifact() {
    let manifest: MarketplaceManifest = serde_json::from_str(
        r#"{
            "schema_version": 2,
            "release_version": "2026.05",
            "extensions": [{
                "id": "rust",
                "kind": "language",
                "name": "rust",
                "version": "0.24.0",
                "release_tag": "rust-v0.24.0",
                "description": "Rust syntax",
                "file_extensions": ["rs"],
                "artifacts": {
                    "universal": {
                        "file": "rust-universal.tar.gz",
                        "sha256": "abc"
                    }
                }
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
    assert_eq!(
        "rust-universal.tar.gz",
        entries[0].artifacts["universal"].file
    );
}

#[test]
fn marketplace_manifest_accepts_language_bundle_artifact() {
    let manifest: MarketplaceManifest = serde_json::from_str(
        r#"{
            "schema_version": 2,
            "release_version": "2026.07",
            "extensions": [{
                "id": "tree-sitter-languages",
                "kind": "language_bundle",
                "name": "Tree-sitter Languages",
                "version": "0.1.0",
                "release_tag": "tree-sitter-languages-v0.1.0",
                "description": "Tree-sitter syntax bundle",
                "file_extensions": ["js", "rs"],
                "artifacts": {
                    "universal": {
                        "file": "tree-sitter-languages-language-bundle-universal.tar.gz",
                        "sha256": "abc"
                    }
                }
            }]
        }"#,
    )
    .unwrap();

    let entries = manifest.into_entries();

    assert_eq!(1, entries.len());
    assert_eq!("tree-sitter-languages", entries[0].id);
    assert_eq!(ExtensionKind::LanguageBundle, entries[0].kind);
    assert_eq!(
        vec!["js".to_string(), "rs".to_string()],
        entries[0].file_extensions
    );
}

#[test]
fn marketplace_manifest_accepts_string_schema_version() {
    let manifest: MarketplaceManifest = serde_json::from_str(
        r#"{
            "schema_version": "2",
            "release_version": "2026.05",
            "extensions": []
        }"#,
    )
    .unwrap();

    assert_eq!(2, manifest.schema_version);
    assert_eq!("2026.05", manifest.release_version);
    assert!(manifest.into_entries().is_empty());
}

#[test]
fn marketplace_manifest_skips_entries_with_future_extension_kind() {
    let manifest: MarketplaceManifest = serde_json::from_str(
        r#"{
            "schema_version": 3,
            "release_version": "2026.07",
            "extensions": [
                {
                    "id": "future_tool",
                    "kind": "future_tool",
                    "name": "Future Tool",
                    "version": "1.0.0",
                    "manifest": "future_tool/manifest.json"
                },
                {
                    "id": "rust",
                    "kind": "language",
                    "name": "rust",
                    "version": "0.24.0",
                    "release_tag": "rust-v0.24.0",
                    "file_extensions": ["rs"],
                    "artifacts": {
                        "universal": {
                            "file": "rust-universal.tar.gz"
                        }
                    }
                }
            ]
        }"#,
    )
    .unwrap();

    let entries = manifest.into_entries();

    assert_eq!(1, entries.len());
    assert_eq!("rust", entries[0].id);
    assert_eq!(ExtensionKind::Language, entries[0].kind);
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

    let acp_agent_dir = tmp.path().join("acp_agent");
    fs::create_dir_all(&acp_agent_dir).unwrap();
    fs::write(acp_agent_dir.join("acp_agent.json"), "{}").unwrap();

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
    assert_eq!(
        ExtensionKind::AcpAgent,
        detect_package_kind(&acp_agent_dir).unwrap()
    );
}

#[test]
fn detect_package_kind_identifies_language_bundle() {
    let tmp = tempfile::TempDir::new().unwrap();
    let bundle_dir = tmp.path().join("tree-sitter-languages");
    fs::create_dir_all(bundle_dir.join("rust")).unwrap();
    fs::create_dir_all(bundle_dir.join("javascript")).unwrap();
    fs::write(
        bundle_dir.join("manifest.json"),
        r#"{
            "id": "tree-sitter-languages",
            "name": "Tree-sitter Languages",
            "version": "0.1.0",
            "languages": ["javascript", "rust"]
        }"#,
    )
    .unwrap();
    fs::write(
        bundle_dir.join("rust/manifest.json"),
        r#"{"name":"rust","version":"0.24.0","file_extensions":["rs"]}"#,
    )
    .unwrap();
    fs::write(bundle_dir.join("rust/parser.wasm"), [0u8; 4]).unwrap();
    fs::write(
        bundle_dir.join("javascript/manifest.json"),
        r#"{"name":"javascript","version":"0.23.1","file_extensions":["js"]}"#,
    )
    .unwrap();
    fs::write(bundle_dir.join("javascript/parser.wasm"), [0u8; 4]).unwrap();

    assert_eq!(
        ExtensionKind::LanguageBundle,
        detect_package_kind(&bundle_dir).unwrap()
    );
}

#[test]
fn detect_package_kind_keeps_single_wrapped_language_as_language() {
    let tmp = tempfile::TempDir::new().unwrap();
    let wrapped = tmp.path().join("wrapped");
    let language = wrapped.join("rust");
    fs::create_dir_all(&language).unwrap();
    fs::write(language.join("manifest.json"), r#"{"name":"rust"}"#).unwrap();
    fs::write(language.join("parser.wasm"), [0u8; 4]).unwrap();

    assert_eq!(
        ExtensionKind::Language,
        detect_package_kind(&wrapped).unwrap()
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
fn install_from_staging_generic_installs_acp_agent() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let staging = tmp.path().join("staging");
    fs::create_dir_all(staging.join("bin")).unwrap();
    fs::write(staging.join("bin/codex-acp"), b"#!/bin/sh\n").unwrap();
    make_executable(&staging.join("bin/codex-acp"));
    write_acp_agent_manifest(&staging);

    let mut registry = ExtensionRegistry::new(root.clone());
    registry.register_provider(Arc::new(AcpAgentExtensionProvider));

    let summary = install_from_staging_generic(&staging, &registry, None).unwrap();

    assert_eq!(
        ExtensionSummary::new(
            ExtensionKind::AcpAgent,
            "codex",
            "1.2.3",
            root.join("acp_agents").join("codex")
        )
        .with_description("Codex ACP agent"),
        summary
    );
    assert!(root.join("acp_agents/codex/acp_agent.json").exists());
    assert!(root.join("acp_agents/codex/bin/codex-acp").exists());
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

fn write_acp_agent_manifest(dir: &std::path::Path) {
    fs::write(
        dir.join("acp_agent.json"),
        r#"{
            "id": "codex",
            "name": "Codex",
            "description": "Codex ACP agent",
            "version": "1.2.3",
            "agents": [{
                "id": "codex",
                "name": "Codex",
                "transport": {
                    "type": "stdio",
                    "command": "bin/codex-acp",
                    "args": ["--stdio"],
                    "env": { "CODEX_HOME": "test-home" }
                }
            }]
        }"#,
    )
    .unwrap();
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &std::path::Path) {}

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
