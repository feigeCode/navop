use public_mcp::registry::{
    ConnectionState, PublicMcpRegistry, TerminalConnectionKind, TerminalReadSessionHandle,
    TerminalSessionSnapshot,
};
use public_mcp::terminal_read::{TerminalReadRequest, TerminalReadResult};
use public_mcp::tools::{PublicMcpToolRegistry, terminal_read_tool_registry};
use serde_json::json;
use tool_runtime::{ToolAdapter, ToolContext};

struct FakeTerminalRead;

impl TerminalReadSessionHandle for FakeTerminalRead {
    fn snapshot(&self) -> TerminalSessionSnapshot {
        TerminalSessionSnapshot {
            session_id: "terminal-1".to_string(),
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

    fn read_terminal(&self, request: TerminalReadRequest) -> anyhow::Result<TerminalReadResult> {
        Ok(TerminalReadResult {
            target: request.target,
            text: "line 2\nline 3".to_string(),
            requested_lines: request.lines,
            returned_lines: 2,
            available_lines: 3,
            history_size: 10,
            screen_lines: 24,
            columns: 120,
            truncated: false,
        })
    }
}

fn registry_with_terminal() -> PublicMcpRegistry {
    let registry = PublicMcpRegistry::default();
    registry.register_terminal_read(FakeTerminalRead);
    registry
}

#[test]
fn terminal_read_is_registered_as_bounded_read_only_tool() {
    let registry = registry_with_terminal();
    let runtime_registry = terminal_read_tool_registry(registry);
    let tool = runtime_registry
        .list(ToolAdapter::Mcp)
        .into_iter()
        .find(|tool| tool.id == "terminal.read")
        .expect("terminal.read should be listed");

    assert_eq!(json!(["target"]), tool.input_schema["required"]);
    assert_eq!(
        json!(80),
        tool.input_schema["properties"]["lines"]["default"]
    );
    assert_eq!(
        json!(500),
        tool.input_schema["properties"]["lines"]["maximum"]
    );
    assert!(tool.description.contains("without executing a command"));
    assert!(tool.description.contains("scrollback"));
    assert!(tool.annotations.read_only);
    assert!(tool.annotations.idempotent);
}

#[tokio::test]
async fn terminal_read_returns_recent_pty_rows() {
    let registry = registry_with_terminal();
    let runtime_registry = terminal_read_tool_registry(registry);

    let result = runtime_registry
        .call(
            "terminal.read",
            json!({"target": "terminal-1", "lines": 2}),
            ToolContext::for_adapter(ToolAdapter::Mcp),
        )
        .await
        .expect("terminal.read should succeed");

    assert_eq!(json!("line 2\nline 3"), result.structured_content["text"]);
    assert_eq!(json!(2), result.structured_content["requested_lines"]);
    assert_eq!(json!(2), result.structured_content["returned_lines"]);
}

#[test]
fn public_terminal_registry_lists_terminal_read() {
    let names = PublicMcpToolRegistry::terminal(registry_with_terminal())
        .tools()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();

    assert!(names.contains(&"terminal.read".to_string()));
}
