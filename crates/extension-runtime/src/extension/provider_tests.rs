use std::{fs, sync::Arc};

use super::{
    AcpAgentExtensionProvider, DatabaseDriverExtensionProvider, ExtensionKind, ExtensionProvider,
    ExtensionRegistry, LanguageExtensionProvider, RemoteDesktopProviderExtensionProvider,
    builtin_registry, load_language_extensions_from_root,
};

#[test]
fn extension_kind_maps_stable_directories() {
    assert_eq!("languages", ExtensionKind::Language.dir_name());
    assert_eq!("database_drivers", ExtensionKind::DatabaseDriver.dir_name());
    assert_eq!(
        "remote_desktop_providers",
        ExtensionKind::RemoteDesktopProvider.dir_name()
    );
    assert_eq!("mcp_helpers", ExtensionKind::McpHelper.dir_name());
    assert_eq!("acp_agents", ExtensionKind::AcpAgent.dir_name());
    assert_eq!("composite", ExtensionKind::Composite.dir_name());
}

#[test]
fn extension_kind_parses_remote_desktop_provider() {
    let kind: ExtensionKind = serde_json::from_str(r#""remote_desktop_provider""#).unwrap();

    assert_eq!(ExtensionKind::RemoteDesktopProvider, kind);
}

#[test]
fn extension_kind_parses_mcp_helper() {
    let kind: ExtensionKind = serde_json::from_str(r#""mcp_helper""#).unwrap();

    assert_eq!(ExtensionKind::McpHelper, kind);
}

#[test]
fn extension_kind_parses_acp_agent() {
    let kind: ExtensionKind = serde_json::from_str(r#""acp_agent""#).unwrap();

    assert_eq!(ExtensionKind::AcpAgent, kind);
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
fn database_driver_provider_install_from_dir_reports_invalid_driver_manifest() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("database_drivers");
    let driver_dir = root.join("broken");
    fs::create_dir_all(&driver_dir).unwrap();
    fs::write(
        driver_dir.join("driver.json"),
        r#"{
            "id": "broken",
            "name": "Broken Driver"
        }"#,
    )
    .unwrap();

    let provider = DatabaseDriverExtensionProvider;
    let err = provider.install_from_dir(&driver_dir).unwrap_err();
    let message = err.to_string();

    assert!(message.contains("解析 driver manifest 失败"));
    assert!(
        message.contains("invalid driver manifest") || message.contains("missing field `entry`")
    );
}

#[test]
fn database_driver_provider_install_from_dir_accepts_single_wrapped_driver_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("database_drivers");
    let outer_dir = root.join("gbase8s");
    let driver_dir = outer_dir.join("gbase8s");
    fs::create_dir_all(&driver_dir).unwrap();
    fs::write(
        driver_dir.join("driver.json"),
        r#"{
            "id": "gbase8s",
            "name": "GBase 8s",
            "description": "GBase 8s IPC driver",
            "version": "0.1.0",
            "entry": { "command": "./gbase8s-ipc-driver" },
            "transport": { "name": "gbase8s.sock" }
        }"#,
    )
    .unwrap();

    let provider = DatabaseDriverExtensionProvider;
    let summary = provider.install_from_dir(&outer_dir).unwrap();

    assert_eq!("gbase8s", summary.name);
    assert_eq!("0.1.0", summary.version);
    assert_eq!(driver_dir, summary.path);
}

#[test]
fn remote_desktop_provider_lists_installed_provider_summaries() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let provider_dir = root.join("remote_desktop_providers").join("rdp");
    fs::create_dir_all(&provider_dir).unwrap();
    fs::write(
        provider_dir.join("remote_desktop_provider.json"),
        remote_desktop_provider_json("rdp", "RDP", "rdp", "./onetcli-rdp-helper"),
    )
    .unwrap();

    let mut registry = ExtensionRegistry::new(root);
    registry.register_provider(Arc::new(RemoteDesktopProviderExtensionProvider));

    let list = registry
        .list_installed_of(ExtensionKind::RemoteDesktopProvider)
        .expect("remote desktop providers should list");

    assert_eq!(1, list.len());
    assert_eq!(ExtensionKind::RemoteDesktopProvider, list[0].kind);
    assert_eq!("rdp", list[0].name);
    assert_eq!("1.2.3", list[0].version);
    assert_eq!("RDP provider", list[0].description);
    assert_eq!(Some("Monitor"), list[0].icon.as_deref());
    assert_eq!(Some(3389), list[0].default_port);
}

#[test]
fn remote_desktop_provider_install_from_dir_requires_manifest() {
    let tmp = tempfile::TempDir::new().unwrap();
    let empty_dir = tmp.path().join("remote_desktop_providers").join("empty");
    fs::create_dir_all(&empty_dir).unwrap();

    let provider = RemoteDesktopProviderExtensionProvider;
    let err = provider.install_from_dir(&empty_dir).unwrap_err();

    assert!(err.to_string().contains("remote_desktop_provider"));
}

#[test]
fn acp_agent_provider_lists_installed_agent_summaries() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let agent_dir = root.join("acp_agents").join("codex");
    fs::create_dir_all(agent_dir.join("bin")).unwrap();
    fs::write(agent_dir.join("bin/codex-acp"), b"#!/bin/sh\n").unwrap();
    make_executable(&agent_dir.join("bin/codex-acp"));
    fs::write(agent_dir.join("acp_agent.json"), acp_agent_json()).unwrap();

    let mut registry = ExtensionRegistry::new(root);
    registry.register_provider(Arc::new(AcpAgentExtensionProvider));

    let list = registry
        .list_installed_of(ExtensionKind::AcpAgent)
        .expect("ACP agent extensions should list");

    assert_eq!(1, list.len());
    assert_eq!(ExtensionKind::AcpAgent, list[0].kind);
    assert_eq!("codex", list[0].name);
    assert_eq!("1.2.3", list[0].version);
    assert_eq!("Codex ACP agent", list[0].description);
}

#[test]
fn acp_agent_provider_install_from_dir_requires_manifest() {
    let tmp = tempfile::TempDir::new().unwrap();
    let empty_dir = tmp.path().join("acp_agents").join("empty");
    fs::create_dir_all(&empty_dir).unwrap();

    let provider = AcpAgentExtensionProvider;
    let err = provider.install_from_dir(&empty_dir).unwrap_err();

    assert!(err.to_string().contains("acp_agent"));
}

#[test]
fn builtin_registry_registers_all_extension_providers() {
    let tmp = tempfile::TempDir::new().unwrap();
    let registry = builtin_registry(tmp.path().join("extensions"));

    assert!(registry.provider(ExtensionKind::Language).is_some());
    assert!(registry.provider(ExtensionKind::DatabaseDriver).is_some());
    assert!(
        registry
            .provider(ExtensionKind::RemoteDesktopProvider)
            .is_some()
    );
    assert!(registry.provider(ExtensionKind::McpHelper).is_some());
    assert!(registry.provider(ExtensionKind::AcpAgent).is_some());
    assert!(registry.provider(ExtensionKind::Composite).is_some());
    assert_eq!(
        tmp.path().join("extensions/languages"),
        registry.root_for(ExtensionKind::Language)
    );
}

fn acp_agent_json() -> String {
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
    }"#
    .to_string()
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

fn remote_desktop_provider_json(id: &str, name: &str, protocol: &str, command: &str) -> String {
    format!(
        r#"{{
            "id": "{id}",
            "name": "{name}",
            "description": "{name} provider",
            "version": "1.2.3",
            "protocol": "{protocol}",
            "entry": {{ "command": "{command}" }},
            "capabilities": {{
                "resize": "remote_resize",
                "clipboard_text": true,
                "cursor_shape": true,
                "audio": false,
                "file_transfer": false
            }},
            "ui": {{
                "icon": "Monitor",
                "default_port": 3389
            }}
        }}"#
    )
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
