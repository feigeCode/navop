use public_mcp::registry::{
    ConnectionState, PublicMcpRegistry, TerminalConnectionKind, TerminalInputSessionHandle,
    TerminalSessionSnapshot,
};
use public_mcp::terminal_write_keys::{TerminalWriteKeysRequest, TerminalWriteKeysResult};
use public_mcp::tools::{PublicMcpToolRegistry, terminal_write_command_tool_registry};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tool_runtime::{RiskLevel, ToolAdapter, ToolContext};

#[derive(Clone)]
struct FakeTerminalInput {
    id: String,
    requests: Arc<Mutex<Vec<TerminalWriteKeysRequest>>>,
}

impl FakeTerminalInput {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl TerminalInputSessionHandle for FakeTerminalInput {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        TerminalSessionSnapshot {
            session_id: self.id.clone(),
            connection_id: Some(42),
            title: "terminal".to_string(),
            host_label: "prod-a".to_string(),
            cwd: Some("/root".to_string()),
            rows: 24,
            cols: 120,
            connection_kind: TerminalConnectionKind::Ssh,
            connection_state: ConnectionState::Connected,
        }
    }

    fn write_keys(
        &self,
        request: TerminalWriteKeysRequest,
    ) -> anyhow::Result<TerminalWriteKeysResult> {
        let bytes_written = request.bytes.len();
        self.requests.lock().unwrap().push(request.clone());
        Ok(TerminalWriteKeysResult {
            target: request.target,
            sent: true,
            bytes_written,
        })
    }
}

fn registry_with_terminal() -> (PublicMcpRegistry, FakeTerminalInput) {
    let registry = PublicMcpRegistry::default();
    let terminal = FakeTerminalInput::new("terminal-1");
    registry.register_terminal_input(terminal.clone());
    (registry, terminal)
}

#[test]
fn terminal_write_command_is_registered_with_terminal_tools() {
    let (registry, _terminal) = registry_with_terminal();
    let names = PublicMcpToolRegistry::terminal(registry)
        .expect("terminal registry should be valid")
        .tools()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();

    assert!(names.contains(&"terminal.write_command".to_string()));
}

#[test]
fn terminal_write_command_descriptor_bypasses_shell_integration_and_points_to_read() {
    let (registry, _terminal) = registry_with_terminal();
    let tool = terminal_write_command_tool_registry(registry)
        .list(ToolAdapter::Mcp)
        .into_iter()
        .find(|tool| tool.id == "terminal.write_command")
        .expect("terminal.write_command should be listed");

    assert_eq!(json!(["target", "command"]), tool.input_schema["required"]);
    assert_eq!(
        json!("string"),
        tool.input_schema["properties"]["command"]["type"]
    );
    assert_eq!(
        json!(true),
        tool.input_schema["properties"]["submit"]["default"]
    );
    assert_eq!(RiskLevel::High, tool.annotations.risk);
    assert!(
        tool.description
            .contains("does not require shell integration")
    );
    assert!(tool.description.contains("terminal.read"));
    assert!(
        tool.input_schema["properties"]["target"]["description"]
            .as_str()
            .unwrap_or_default()
            .contains("terminal_input")
    );
}

#[tokio::test]
async fn terminal_write_command_submits_text_with_carriage_return() {
    let (registry, terminal) = registry_with_terminal();
    let result = terminal_write_command_tool_registry(registry)
        .call(
            "terminal.write_command",
            json!({
                "target": "terminal-1",
                "command": "printf 'hello'",
            }),
            ToolContext::for_adapter(ToolAdapter::Mcp),
        )
        .await
        .expect("terminal.write_command should succeed");

    let expected = b"printf 'hello'\r";
    assert_eq!(json!(true), result.structured_content["sent"]);
    assert_eq!(
        json!(expected.len()),
        result.structured_content["bytes_written"]
    );
    let requests = terminal.requests.lock().unwrap();
    assert_eq!(1, requests.len());
    assert_eq!(expected, requests[0].bytes.as_slice());
}

#[tokio::test]
async fn terminal_write_command_can_stage_without_submitting() {
    let (registry, terminal) = registry_with_terminal();
    terminal_write_command_tool_registry(registry)
        .call(
            "terminal.write_command",
            json!({
                "target": "terminal-1",
                "command": "echo staged",
                "submit": false,
            }),
            ToolContext::for_adapter(ToolAdapter::Mcp),
        )
        .await
        .expect("terminal.write_command should succeed");

    let requests = terminal.requests.lock().unwrap();
    assert_eq!(b"echo staged", requests[0].bytes.as_slice());
}

#[tokio::test]
async fn terminal_write_command_rejects_empty_multiline_and_control_input() {
    for command in [
        "",
        "   ",
        "echo one\necho two",
        "echo one\recho two",
        "echo\u{3}",
    ] {
        let (registry, _terminal) = registry_with_terminal();
        let error = terminal_write_command_tool_registry(registry)
            .call(
                "terminal.write_command",
                json!({
                    "target": "terminal-1",
                    "command": command,
                }),
                ToolContext::for_adapter(ToolAdapter::Mcp),
            )
            .await
            .expect_err("invalid command text should be rejected");

        assert!(
            error.to_string().contains("command"),
            "unexpected error for {command:?}: {error}"
        );
    }
}
