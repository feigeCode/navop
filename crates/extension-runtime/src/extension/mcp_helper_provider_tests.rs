use std::{fs, sync::Arc};

use super::{ExtensionKind, ExtensionProvider, ExtensionRegistry, McpHelperExtensionProvider};

#[test]
fn mcp_helper_provider_lists_installed_helper_summaries() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let helper_dir = root.join("mcp_helpers").join("onetcli-public-mcp");
    fs::create_dir_all(&helper_dir).unwrap();
    fs::write(
        helper_dir.join("mcp_helper.json"),
        mcp_helper_json(
            "onetcli-public-mcp",
            "OnetCli MCP Helper",
            "./onetcli-public-mcp",
        ),
    )
    .unwrap();
    fs::write(helper_dir.join("onetcli-public-mcp"), b"helper").unwrap();
    make_executable(&helper_dir.join("onetcli-public-mcp"));

    let mut registry = ExtensionRegistry::new(root);
    registry.register_provider(Arc::new(McpHelperExtensionProvider));

    let list = registry
        .list_installed_of(ExtensionKind::McpHelper)
        .expect("mcp helpers should list");

    assert_eq!(1, list.len());
    assert_eq!(ExtensionKind::McpHelper, list[0].kind);
    assert_eq!("onetcli-public-mcp", list[0].name);
    assert_eq!("1.2.3", list[0].version);
    assert_eq!("OnetCli MCP Helper helper", list[0].description);
}

#[test]
fn mcp_helper_provider_install_from_dir_requires_manifest() {
    let tmp = tempfile::TempDir::new().unwrap();
    let empty_dir = tmp.path().join("mcp_helpers").join("empty");
    fs::create_dir_all(&empty_dir).unwrap();

    let provider = McpHelperExtensionProvider;
    let err = provider.install_from_dir(&empty_dir).unwrap_err();

    assert!(err.to_string().contains("mcp_helper"));
}

#[test]
fn mcp_helper_provider_install_from_dir_requires_existing_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let helper_dir = tmp.path().join("mcp_helpers").join("missing-command");
    fs::create_dir_all(&helper_dir).unwrap();
    fs::write(
        helper_dir.join("mcp_helper.json"),
        mcp_helper_json("missing-command", "Missing Command", "./missing-helper"),
    )
    .unwrap();

    let provider = McpHelperExtensionProvider;
    let err = provider.install_from_dir(&helper_dir).unwrap_err();

    assert!(err.to_string().contains("entry.command"));
    assert!(err.to_string().contains("missing-helper"));
}

#[test]
fn mcp_helper_provider_install_from_dir_rejects_escaping_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let helper_dir = tmp.path().join("mcp_helpers").join("escaping-command");
    fs::create_dir_all(&helper_dir).unwrap();
    fs::write(
        helper_dir.join("mcp_helper.json"),
        mcp_helper_json("escaping-command", "Escaping Command", "../outside-helper"),
    )
    .unwrap();

    let provider = McpHelperExtensionProvider;
    let err = provider.install_from_dir(&helper_dir).unwrap_err();

    assert!(err.to_string().contains("entry.command"));
    assert!(err.to_string().contains("inside"));
}

#[cfg(unix)]
#[test]
fn mcp_helper_provider_install_from_dir_requires_executable_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let helper_dir = tmp.path().join("mcp_helpers").join("plain-file-command");
    fs::create_dir_all(&helper_dir).unwrap();
    fs::write(
        helper_dir.join("mcp_helper.json"),
        mcp_helper_json("plain-file-command", "Plain File Command", "./plain-helper"),
    )
    .unwrap();
    fs::write(helper_dir.join("plain-helper"), b"helper").unwrap();

    let provider = McpHelperExtensionProvider;
    let err = provider.install_from_dir(&helper_dir).unwrap_err();

    assert!(err.to_string().contains("entry.command"));
    assert!(err.to_string().contains("executable"));
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

fn mcp_helper_json(id: &str, name: &str, command: &str) -> String {
    format!(
        r#"{{
            "id": "{id}",
            "name": "{name}",
            "description": "{name} helper",
            "version": "1.2.3",
            "entry": {{ "command": "{command}" }}
        }}"#
    )
}
