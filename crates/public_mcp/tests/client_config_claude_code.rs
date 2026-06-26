use public_mcp::client_config::{
    ClientConfigHealth, ClientConfigInstall, agent_mcp_config_json, claude_code_config_path,
    inspect_claude_code_config, install_claude_code_config,
};

#[test]
fn claude_code_config_install_writes_user_config_shape() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join(".claude.json");

    install_claude_code_config(&config_path, &install()).unwrap();

    let json = read_json(&config_path);
    assert_eq!(
        "/opt/one-hub/extensions/mcp_helpers/onetcli-public-mcp/onetcli-public-mcp",
        json["mcpServers"]["onetcli"]["command"]
    );
    assert_eq!(
        serde_json::json!(["--discovery", "/tmp/onetcli/public-mcp.json"]),
        json["mcpServers"]["onetcli"]["args"]
    );
}

#[test]
fn claude_code_config_inspection_reports_health_states() {
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("onetcli-public-mcp");
    write_usable_helper(&helper);
    let install = install_with_helper(&helper);
    let config_path = dir.path().join(".claude.json");

    assert_eq!(
        ClientConfigHealth::NotInstalled,
        inspect_claude_code_config(&config_path, &install).unwrap()
    );

    install_claude_code_config(&config_path, &install).unwrap();
    assert_eq!(
        ClientConfigHealth::UpToDate,
        inspect_claude_code_config(&config_path, &install).unwrap()
    );

    std::fs::write(
        &config_path,
        r#"{"mcpServers":{"onetcli":{"command":"/old/helper","args":["serve"]}}}"#,
    )
    .unwrap();
    assert_eq!(
        ClientConfigHealth::NeedsRepair,
        inspect_claude_code_config(&config_path, &install).unwrap()
    );
}

#[test]
fn agent_mcp_config_json_returns_portable_mcp_servers_block() {
    let json =
        serde_json::from_str::<serde_json::Value>(&agent_mcp_config_json(&install()).unwrap())
            .expect("portable MCP config should be valid JSON");

    assert_eq!(
        serde_json::json!({
            "command": "/opt/one-hub/extensions/mcp_helpers/onetcli-public-mcp/onetcli-public-mcp",
            "args": ["--discovery", "/tmp/onetcli/public-mcp.json"]
        }),
        json["mcpServers"]["onetcli"]
    );
}

#[cfg(unix)]
#[test]
fn claude_code_inspection_reports_unusable_helper_for_non_executable_file() {
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("onetcli-public-mcp");
    std::fs::write(&helper, "").unwrap();
    let config_path = dir.path().join(".claude.json");

    assert_eq!(
        ClientConfigHealth::UnusableHelper,
        inspect_claude_code_config(&config_path, &install_with_helper(&helper)).unwrap()
    );
}

#[test]
fn claude_code_config_path_uses_claude_code_user_file() {
    let path = claude_code_config_path().expect("home directory should be available");

    assert_eq!(
        Some(".claude.json"),
        path.file_name().and_then(|name| name.to_str())
    );
}

fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn install() -> ClientConfigInstall {
    ClientConfigInstall::from_helper_path(
        "/opt/one-hub/extensions/mcp_helpers/onetcli-public-mcp/onetcli-public-mcp",
        "/tmp/onetcli/public-mcp.json",
    )
}

fn install_with_helper(helper: &std::path::Path) -> ClientConfigInstall {
    ClientConfigInstall::from_helper_path(helper, "/tmp/onetcli/public-mcp.json")
}

fn write_usable_helper(path: &std::path::Path) {
    std::fs::write(path, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }
}
