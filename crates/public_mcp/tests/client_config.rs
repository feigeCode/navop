use public_mcp::client_config::{
    ClientConfigHealth, ClientConfigInstall, helper_install_path_from_data_dir,
    inspect_claude_desktop_config, inspect_codex_config, install_claude_desktop_config,
    install_codex_config,
};

#[test]
fn codex_config_install_replaces_managed_block_and_preserves_user_content() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        r#"model = "gpt-5.4"

# BEGIN ONETCLI PUBLIC MCP
[mcp_servers.onetcli]
command = "/old/helper"
args = ["--discovery", "/old/discovery.json"]
# END ONETCLI PUBLIC MCP
"#,
    )
    .unwrap();

    install_codex_config(&config_path, &install()).unwrap();

    let text = std::fs::read_to_string(config_path).unwrap();
    assert!(text.contains(r#"model = "gpt-5.4""#));
    assert!(!text.contains("/old/helper"));
    assert!(text.contains("[mcp_servers.navop]"));
    assert!(text.contains(
        r#"command = "/opt/one-hub/extensions/mcp_helpers/onetcli-public-mcp/onetcli-public-mcp""#
    ));
    assert!(text.contains(r#"args = ["--discovery", "/tmp/onetcli/public-mcp.json"]"#));
}

#[test]
fn codex_config_install_creates_parent_directory() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("nested").join("config.toml");

    install_codex_config(&config_path, &install()).unwrap();

    assert!(
        std::fs::read_to_string(config_path)
            .unwrap()
            .contains("[mcp_servers.navop]")
    );
}

#[test]
fn claude_config_install_merges_onetcli_server_and_preserves_existing_servers() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("claude_desktop_config.json");
    std::fs::write(
        &config_path,
        r#"{
  "mcpServers": {
    "existing": {
      "command": "/usr/bin/existing",
      "args": ["serve"]
    }
  }
}"#,
    )
    .unwrap();

    install_claude_desktop_config(&config_path, &install()).unwrap();

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config_path).unwrap()).unwrap();
    assert_eq!(
        "/usr/bin/existing",
        json["mcpServers"]["existing"]["command"]
    );
    assert_eq!(
        "/opt/one-hub/extensions/mcp_helpers/onetcli-public-mcp/onetcli-public-mcp",
        json["mcpServers"]["navop"]["command"]
    );
    assert_eq!(
        serde_json::json!(["--discovery", "/tmp/onetcli/public-mcp.json"]),
        json["mcpServers"]["navop"]["args"]
    );
}

#[test]
fn claude_config_install_creates_missing_config() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("Claude").join("claude_desktop_config.json");

    install_claude_desktop_config(&config_path, &install()).unwrap();

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config_path).unwrap()).unwrap();
    assert_eq!(
        "/opt/one-hub/extensions/mcp_helpers/onetcli-public-mcp/onetcli-public-mcp",
        json["mcpServers"]["navop"]["command"]
    );
}

#[test]
fn codex_config_inspection_reports_up_to_date_and_not_installed() {
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("onetcli-public-mcp");
    write_usable_helper(&helper);
    let install = install_with_helper(&helper);
    let config_path = dir.path().join("config.toml");

    assert_eq!(
        ClientConfigHealth::NotInstalled,
        inspect_codex_config(&config_path, &install).unwrap()
    );

    install_codex_config(&config_path, &install).unwrap();

    assert_eq!(
        ClientConfigHealth::UpToDate,
        inspect_codex_config(&config_path, &install).unwrap()
    );
}

#[test]
fn codex_config_inspection_reports_needs_repair_for_stale_managed_block() {
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("onetcli-public-mcp");
    write_usable_helper(&helper);
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        r#"# BEGIN ONETCLI PUBLIC MCP
[mcp_servers.onetcli]
command = "/old/helper"
args = ["--discovery", "/old/discovery.json"]
# END ONETCLI PUBLIC MCP
"#,
    )
    .unwrap();

    assert_eq!(
        ClientConfigHealth::NeedsMigration,
        inspect_codex_config(&config_path, &install_with_helper(&helper)).unwrap()
    );
}

#[test]
fn claude_config_inspection_reports_health_states() {
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("onetcli-public-mcp");
    write_usable_helper(&helper);
    let install = install_with_helper(&helper);
    let config_path = dir.path().join("claude_desktop_config.json");

    assert_eq!(
        ClientConfigHealth::NotInstalled,
        inspect_claude_desktop_config(&config_path, &install).unwrap()
    );

    install_claude_desktop_config(&config_path, &install).unwrap();
    assert_eq!(
        ClientConfigHealth::UpToDate,
        inspect_claude_desktop_config(&config_path, &install).unwrap()
    );

    std::fs::write(
        &config_path,
        r#"{"mcpServers":{"onetcli":{"command":"/old/helper","args":["serve"]}}}"#,
    )
    .unwrap();
    assert_eq!(
        ClientConfigHealth::NeedsMigration,
        inspect_claude_desktop_config(&config_path, &install).unwrap()
    );
}

#[test]
fn client_config_inspection_reports_missing_helper_before_config_state() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");

    assert_eq!(
        ClientConfigHealth::MissingHelper,
        inspect_codex_config(&config_path, &install()).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn client_config_inspection_reports_unusable_helper_for_non_executable_file() {
    let dir = tempfile::tempdir().unwrap();
    let helper = dir.path().join("onetcli-public-mcp");
    std::fs::write(&helper, "").unwrap();
    let config_path = dir.path().join("config.toml");

    assert_eq!(
        ClientConfigHealth::UnusableHelper,
        inspect_codex_config(&config_path, &install_with_helper(&helper)).unwrap()
    );
    assert_eq!(
        ClientConfigHealth::UnusableHelper,
        inspect_claude_desktop_config(&config_path, &install_with_helper(&helper)).unwrap()
    );
}

#[test]
fn helper_install_path_resolves_npx_instead_of_a_bundled_helper() {
    let path = helper_install_path_from_data_dir("/Users/me/.config/one-hub");
    assert_eq!(Some("npx"), path.file_stem().and_then(|stem| stem.to_str()));
}

#[test]
fn npx_install_uses_latest_package_tag_and_navop_server_name() {
    let dir = tempfile::tempdir().unwrap();
    let npx = dir.path().join("npx");
    write_usable_helper(&npx);
    let install = ClientConfigInstall::from_npx_path(&npx, "/tmp/navop/public-mcp.json", "latest");
    assert_eq!("npx", install.launch_spec.command);
    assert_eq!(npx, install.launcher_path);

    let config_path = dir.path().join("config.toml");
    install_codex_config(&config_path, &install).unwrap();
    let text = std::fs::read_to_string(config_path).unwrap();
    assert!(text.contains("[mcp_servers.navop]"));
    assert!(text.contains(r#"command = "npx""#));
    assert!(text.contains("@navop/mcp@latest"));
}

#[test]
fn install_from_helper_path_is_independent_from_app_bundle() {
    let install = ClientConfigInstall::from_helper_path(
        "/Users/me/.config/one-hub/extensions/mcp_helpers/onetcli-public-mcp/onetcli-public-mcp",
        "/tmp/onetcli/public-mcp.json",
    );

    assert_eq!(
        std::path::PathBuf::from(
            "/Users/me/.config/one-hub/extensions/mcp_helpers/onetcli-public-mcp/onetcli-public-mcp"
        ),
        install.launcher_path
    );
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
