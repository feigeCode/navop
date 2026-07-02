use public_mcp::registry::{
    ConnectionState, PublicMcpRegistry, TerminalConnectionKind, TerminalExecSessionHandle,
    TerminalSessionSnapshot,
};
use public_mcp::terminal_exec::{TerminalExecCompletion, TerminalExecRequest, TerminalExecResult};
use public_mcp::tools::{PublicMcpToolRegistry, terminal_exec_tool_registry};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tool_runtime::{ToolAdapter, ToolContext};

#[derive(Clone)]
struct FakeTerminalExec {
    id: String,
    inserted: Arc<Mutex<Vec<String>>>,
}

impl FakeTerminalExec {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            inserted: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn inserted(&self) -> Vec<String> {
        self.inserted.lock().expect("inserted lock").clone()
    }
}

impl TerminalExecSessionHandle for FakeTerminalExec {
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

    fn exec_in_terminal(&self, request: TerminalExecRequest) -> anyhow::Result<TerminalExecResult> {
        let suffix = if request.submit { "\n" } else { "" };
        self.inserted
            .lock()
            .expect("inserted lock")
            .push(format!("{}{}", request.command, suffix));
        Ok(TerminalExecResult {
            target: request.target,
            command: request.command,
            submitted: request.submit,
            completion: TerminalExecCompletion::ObservedOutput,
            exit_code: None,
            output: "Filesystem Size Used Avail Use% Mounted on\n/dev/sda1 47G 42G 5G 90% /\n"
                .to_string(),
            duration_ms: 12,
        })
    }
}

fn registry_with_terminal() -> (PublicMcpRegistry, FakeTerminalExec) {
    let registry = PublicMcpRegistry::default();
    let terminal = FakeTerminalExec::new("terminal-1");
    registry.register_terminal_exec(terminal.clone());
    (registry, terminal)
}

#[test]
fn terminal_exec_tool_is_registered() {
    let (registry, _terminal) = registry_with_terminal();
    let tool_registry = PublicMcpToolRegistry::terminal(registry);
    let names = tool_registry
        .tools()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();

    assert!(names.contains(&"terminal.exec".to_string()));
}

#[test]
fn terminal_exec_descriptor_uses_target_and_command_schema() {
    let (registry, _terminal) = registry_with_terminal();
    let runtime_registry = terminal_exec_tool_registry(registry);
    let tool = runtime_registry
        .list(ToolAdapter::Mcp)
        .into_iter()
        .find(|tool| tool.id == "terminal.exec")
        .expect("terminal.exec should be listed");

    assert_eq!(json!(["target", "command"]), tool.input_schema["required"]);
    assert_eq!("string", tool.input_schema["properties"]["target"]["type"]);
    assert_eq!("string", tool.input_schema["properties"]["command"]["type"]);
    assert!(!tool.annotations.read_only);
    assert!(tool.annotations.open_world);
}

#[test]
fn terminal_exec_inserts_command_into_terminal_and_returns_observed_output() {
    let (registry, terminal) = registry_with_terminal();
    let runtime_registry = terminal_exec_tool_registry(registry);

    let result = futures::executor::block_on(runtime_registry.call(
        "terminal.exec",
        json!({
            "target": "terminal-1",
            "command": "df -h",
            "submit": true,
            "wait_for_output": true
        }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect("terminal.exec should run");

    assert_eq!(vec!["df -h\n".to_string()], terminal.inserted());
    assert_eq!(json!("terminal-1"), result.structured_content["target"]);
    assert_eq!(json!("df -h"), result.structured_content["command"]);
    assert_eq!(json!(true), result.structured_content["submitted"]);
    assert_eq!(
        json!("observed_output"),
        result.structured_content["completion"]
    );
    assert_eq!(json!(null), result.structured_content["exit_code"]);
    assert!(
        result.structured_content["output"]
            .as_str()
            .unwrap_or_default()
            .contains("/dev/sda1")
    );
}
