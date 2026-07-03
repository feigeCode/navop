use crate::registry::PublicMcpRegistry;
use crate::terminal_exec::{TerminalExecRequest, TerminalExecResult};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ResourceCapability, ResourceKind, RiskLevel, ToolAdapter, ToolAnnotations, ToolContext,
    ToolDescriptor, ToolError, ToolHandler, ToolMode, ToolRegistry, ToolResult, ToolTargetSpec,
};

#[derive(Clone)]
struct TerminalExecRuntime {
    registry: PublicMcpRegistry,
}

impl TerminalExecRuntime {
    fn new(registry: PublicMcpRegistry) -> Self {
        Self { registry }
    }

    fn exec(&self, input: Value) -> Result<ToolResult, ToolError> {
        let request = parse_exec_request(input)?;
        let target = request.target.clone();
        let result = self
            .registry
            .terminal_exec(&target, request)
            .map_err(tool_error)?;
        terminal_exec_result(result)
    }
}

pub fn terminal_exec_tool_registry(registry: PublicMcpRegistry) -> ToolRegistry {
    ToolRegistry::new(vec![Arc::new(TerminalExecRuntime::new(registry))])
}

impl ToolHandler for TerminalExecRuntime {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "terminal.exec".to_string(),
            title: "Execute in terminal".to_string(),
            description:
                "Insert a command into an active visible terminal session and optionally submit it."
                    .to_string(),
            input_schema: exec_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations {
                title: "Execute in terminal".to_string(),
                read_only: false,
                destructive: false,
                idempotent: false,
                open_world: true,
                supports_parallel: false,
                risk: RiskLevel::High,
            },
        }
    }

    fn target_spec(&self) -> ToolTargetSpec {
        ToolTargetSpec::required_with_capabilities(
            vec![ResourceKind::Terminal],
            vec![ResourceCapability::TerminalExec],
        )
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let runtime = self.clone();
        Box::pin(async move { run_terminal_exec(runtime, input).await })
    }
}

async fn run_terminal_exec(
    runtime: TerminalExecRuntime,
    input: Value,
) -> Result<ToolResult, ToolError> {
    if tokio::runtime::Handle::try_current().is_err() {
        return runtime.exec(input);
    }

    tokio::task::spawn_blocking(move || runtime.exec(input))
        .await
        .map_err(|error| ToolError::Failed {
            message: format!("terminal.exec task failed: {error}"),
        })?
}

fn exec_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "target": {
                "type": "string",
                "description": "Active terminal resource id, label, or alias."
            },
            "command": {
                "type": "string",
                "description": "Command text to insert into the terminal."
            },
            "submit": {
                "type": "boolean",
                "default": true,
                "description": "When true, press Enter after inserting the command."
            },
            "wait_for_output": {
                "type": "boolean",
                "default": true,
                "description": "When true, wait for a bounded terminal output delta."
            },
            "timeout_ms": {
                "type": ["integer", "null"],
                "default": 60000
            }
        },
        "required": ["target", "command"]
    })
}

fn parse_exec_request(input: Value) -> Result<TerminalExecRequest, ToolError> {
    let target = required_str(&input, "target")?.to_string();
    let command = required_str(&input, "command")?.to_string();
    let submit = optional_bool(&input, "submit").unwrap_or(true);
    let wait_for_output = optional_bool(&input, "wait_for_output").unwrap_or(true);
    let timeout_ms = optional_u64(&input, "timeout_ms");
    Ok(TerminalExecRequest {
        target,
        command,
        submit,
        wait_for_output,
        timeout_ms,
    })
}

fn terminal_exec_result(result: TerminalExecResult) -> Result<ToolResult, ToolError> {
    let value = serde_json::to_value(result).map_err(tool_error)?;
    Ok(ToolResult::structured(value))
}

fn required_str<'a>(input: &'a Value, field: &'static str) -> Result<&'a str, ToolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ToolError::Failed {
            message: format!("missing required string field `{field}`"),
        })
}

fn optional_bool(input: &Value, field: &str) -> Option<bool> {
    input.get(field).and_then(Value::as_bool)
}

fn optional_u64(input: &Value, field: &str) -> Option<u64> {
    input.get(field).and_then(Value::as_u64)
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}
