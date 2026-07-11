use public_mcp::registry::{
    ConnectionState, PublicMcpRegistry, TerminalConnectionKind, TerminalControlFuture,
    TerminalControlSessionHandle, TerminalSessionSnapshot,
};
use public_mcp::terminal_control::{
    TerminalControlAction, TerminalControlReadiness, TerminalControlRequest, TerminalControlResult,
};
use public_mcp::tools::{PublicMcpToolRegistry, terminal_control_tool_registry};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tool_runtime::{RiskLevel, ToolAdapter, ToolContext};

#[derive(Clone)]
struct FakeTerminalControl {
    id: String,
    requests: Arc<Mutex<Vec<TerminalControlRequest>>>,
}

impl FakeTerminalControl {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl TerminalControlSessionHandle for FakeTerminalControl {
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

    fn control_terminal(
        &self,
        request: TerminalControlRequest,
        _cancellation: tokio_util::sync::CancellationToken,
    ) -> TerminalControlFuture {
        self.requests.lock().unwrap().push(request.clone());
        Box::pin(async move {
            Ok(TerminalControlResult {
                target: request.target,
                action: request.action,
                sent: true,
                readiness_before: TerminalControlReadiness::CommandRunning,
            })
        })
    }
}

fn registry_with_terminal() -> (PublicMcpRegistry, FakeTerminalControl) {
    let registry = PublicMcpRegistry::default();
    let terminal = FakeTerminalControl::new("terminal-1");
    registry.register_terminal_control(terminal.clone());
    (registry, terminal)
}

#[test]
fn terminal_control_tool_is_registered_with_terminal_tools() {
    let (registry, _terminal) = registry_with_terminal();
    let names = PublicMcpToolRegistry::terminal(registry)
        .tools()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();

    assert!(names.contains(&"terminal.control".to_string()));
}

#[test]
fn terminal_control_descriptor_requires_interrupt_action() {
    let (registry, _terminal) = registry_with_terminal();
    let descriptor = terminal_control_tool_registry(registry)
        .list(ToolAdapter::Mcp)
        .into_iter()
        .find(|tool| tool.id == "terminal.control")
        .expect("terminal.control should be listed");

    assert_eq!(
        json!(["target", "action"]),
        descriptor.input_schema["required"]
    );
    assert_eq!(
        json!(["interrupt"]),
        descriptor.input_schema["properties"]["action"]["enum"]
    );
    assert_eq!(RiskLevel::High, descriptor.annotations.risk);
    assert!(descriptor.annotations.open_world);
    assert!(!descriptor.annotations.supports_parallel);
}

#[test]
fn terminal_control_interrupt_returns_structured_result() {
    let (registry, terminal) = registry_with_terminal();
    let result = futures::executor::block_on(terminal_control_tool_registry(registry).call(
        "terminal.control",
        json!({
            "target": "terminal-1",
            "action": "interrupt"
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("terminal.control should run");

    assert_eq!(json!(true), result.structured_content["sent"]);
    assert_eq!(
        json!("command_running"),
        result.structured_content["readiness_before"]
    );
    let requests = terminal.requests.lock().unwrap();
    assert_eq!(1, requests.len());
    assert_eq!(TerminalControlAction::Interrupt, requests[0].action);
}
