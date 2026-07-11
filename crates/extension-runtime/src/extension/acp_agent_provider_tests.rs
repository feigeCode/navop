use std::fs;
use std::path::Path;

use super::AcpAgentExtensionProvider;

#[test]
fn legacy_manifest_uses_safe_auth_and_timeout_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let agent = load_single_agent(temp.path(), legacy_manifest()).unwrap();

    assert!(agent.auth.allow_unauthenticated_fallback);
    assert!(agent.auth.methods.is_empty());
    assert_eq!(30, agent.timeouts.connect_seconds);
    assert_eq!(120, agent.timeouts.authenticate_seconds);
    assert_eq!(600, agent.timeouts.prompt_seconds);
}

#[test]
fn manifest_parses_explicit_auth_requirements() {
    let temp = tempfile::tempdir().unwrap();
    let agent = load_single_agent(temp.path(), manifest_with_auth()).unwrap();

    assert_eq!(Some("api-key"), agent.auth.preferred_method.as_deref());
    assert!(!agent.auth.allow_unauthenticated_fallback);
    assert_eq!(
        vec!["OPENAI_API_KEY", "CODEX_API_KEY"],
        agent.auth.methods[0].env_any
    );
    assert!(agent.auth.methods[1].interactive);
    assert_eq!(45, agent.timeouts.connect_seconds);
}

#[test]
fn manifest_rejects_timeout_outside_supported_range() {
    let temp = tempfile::tempdir().unwrap();
    let error = load_single_agent(temp.path(), manifest_with_prompt_timeout(0)).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("prompt_seconds must be between 1 and 3600")
    );
}

fn load_single_agent(
    root: &Path,
    manifest: String,
) -> anyhow::Result<super::AcpAgentExtensionAgent> {
    let package = root.join("test-agent");
    fs::create_dir_all(package.join("bin"))?;
    let command = package.join("bin/test-agent");
    fs::write(&command, "#!/bin/sh\n")?;
    make_executable(&command);
    fs::write(package.join("acp_agent.json"), manifest)?;
    let mut agents = AcpAgentExtensionProvider::load_agents_from_root(root)?;
    Ok(agents.remove(0))
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

fn legacy_manifest() -> String {
    manifest_with_agent_fields("")
}

fn manifest_with_auth() -> String {
    manifest_with_agent_fields(
        r#",
        "auth": {
          "preferred_method": "api-key",
          "allow_unauthenticated_fallback": false,
          "methods": [
            {
              "id": "api-key",
              "env_any": ["OPENAI_API_KEY", "CODEX_API_KEY"],
              "env_all": [],
              "interactive": false
            },
            {
              "id": "chat-gpt",
              "interactive": true
            }
          ]
        },
        "timeouts": {
          "connect_seconds": 45,
          "authenticate_seconds": 90,
          "prompt_seconds": 300
        }"#,
    )
}

fn manifest_with_prompt_timeout(seconds: u64) -> String {
    manifest_with_agent_fields(&format!(
        r#",
        "timeouts": {{
          "prompt_seconds": {seconds}
        }}"#
    ))
}

fn manifest_with_agent_fields(extra: &str) -> String {
    format!(
        r#"{{
  "id": "test-extension",
  "name": "Test Extension",
  "agents": [
    {{
      "id": "test-agent",
      "name": "Test Agent",
      "transport": {{
        "type": "stdio",
        "command": "bin/test-agent"
      }}{extra}
    }}
  ]
}}"#
    )
}
