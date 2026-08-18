use public_mcp::registry::{
    ConnectionState, PublicMcpRegistry, TerminalConnectionKind, TerminalInputSessionHandle,
    TerminalSessionSnapshot,
};
use public_mcp::terminal_write_keys::{TerminalWriteKeysRequest, TerminalWriteKeysResult};
use public_mcp::tools::{PublicMcpToolRegistry, terminal_write_keys_tool_registry};
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
fn terminal_write_keys_is_registered_with_terminal_tools() {
    let (registry, _terminal) = registry_with_terminal();
    let names = PublicMcpToolRegistry::terminal(registry)
        .expect("terminal registry should be valid")
        .tools()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();

    assert!(names.contains(&"terminal.write_keys".to_string()));
}

#[test]
fn terminal_write_keys_descriptor_requires_raw_byte_array_and_capability() {
    let (registry, _terminal) = registry_with_terminal();
    let tool = terminal_write_keys_tool_registry(registry)
        .list(ToolAdapter::Mcp)
        .into_iter()
        .find(|tool| tool.id == "terminal.write_keys")
        .expect("terminal.write_keys should be listed");

    assert_eq!(json!(["target", "bytes"]), tool.input_schema["required"]);
    assert_eq!(
        json!("array"),
        tool.input_schema["properties"]["bytes"]["type"]
    );
    assert_eq!(
        json!(0),
        tool.input_schema["properties"]["bytes"]["items"]["minimum"]
    );
    assert_eq!(
        json!(255),
        tool.input_schema["properties"]["bytes"]["items"]["maximum"]
    );
    assert_eq!(RiskLevel::High, tool.annotations.risk);
    assert!(!tool.annotations.supports_parallel);
    assert!(tool.description.contains("foreground"));
    assert!(tool.description.contains("terminal.read"));

    let target_spec = terminal_write_keys_tool_registry(PublicMcpRegistry::default())
        .list(ToolAdapter::Mcp)
        .into_iter()
        .find(|tool| tool.id == "terminal.write_keys")
        .expect("terminal.write_keys should be listed");
    assert!(
        target_spec.input_schema["properties"]["target"]["description"]
            .as_str()
            .unwrap_or_default()
            .contains("terminal_input")
    );
}

#[tokio::test]
async fn terminal_write_keys_preserves_raw_bytes_and_returns_count() {
    let (registry, terminal) = registry_with_terminal();
    let result = terminal_write_keys_tool_registry(registry)
        .call(
            "terminal.write_keys",
            json!({
                "target": "terminal-1",
                "bytes": [27, 58, 119, 113, 13],
            }),
            ToolContext::for_adapter(ToolAdapter::Mcp),
        )
        .await
        .expect("terminal.write_keys should succeed");

    assert_eq!(json!(true), result.structured_content["sent"]);
    assert_eq!(json!(5), result.structured_content["bytes_written"]);
    let requests = terminal.requests.lock().unwrap();
    assert_eq!(1, requests.len());
    assert_eq!(vec![27, 58, 119, 113, 13], requests[0].bytes);
}

#[tokio::test]
async fn terminal_write_keys_rejects_invalid_payload() {
    let (registry, _terminal) = registry_with_terminal();

    let error = terminal_write_keys_tool_registry(registry)
        .call(
            "terminal.write_keys",
            json!({"target": "terminal-1", "bytes": [256]}),
            ToolContext::for_adapter(ToolAdapter::Mcp),
        )
        .await
        .expect_err("out-of-range bytes should be rejected");

    assert!(error.to_string().contains("0 through 255"));
}
