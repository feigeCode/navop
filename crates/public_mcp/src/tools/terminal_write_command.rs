use crate::registry::PublicMcpRegistry;
use crate::terminal_write_keys::{TerminalWriteKeysRequest, TerminalWriteKeysResult};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ResourceCapability, ResourceKind, RiskLevel, ToolAdapter, ToolAnnotations, ToolContext,
    ToolDescriptor, ToolError, ToolHandler, ToolMode, ToolRegistry, ToolResult, ToolTargetSpec,
};

const MAX_WRITE_COMMAND_BYTES: usize = 8 * 1024 - 1;

#[derive(Clone)]
struct TerminalWriteCommandRuntime {
    registry: PublicMcpRegistry,
}

impl TerminalWriteCommandRuntime {
    fn new(registry: PublicMcpRegistry) -> Self {
        Self { registry }
    }

    fn write_command(&self, input: Value) -> Result<ToolResult, ToolError> {
        let request = parse_write_command_request(input)?;
        let target = request.target.clone();
        let result = self
            .registry
            .terminal_write_keys(&target, request)
            .map_err(tool_error)?;
        terminal_write_command_result(result)
    }
}

pub fn terminal_write_command_tool_registry(registry: PublicMcpRegistry) -> ToolRegistry {
    ToolRegistry::new(vec![Arc::new(TerminalWriteCommandRuntime::new(registry))])
}

impl ToolHandler for TerminalWriteCommandRuntime {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "terminal.write_command".to_string(),
            title: "Write command to terminal".to_string(),
            description: "Write one shell command directly to the PTY of the current visible terminal and optionally submit it with Enter. This does not require shell integration, OSC prompt markers, or terminal.exec readiness, so it remains usable after entering a nested shell such as `sudo -i` or `su`. It only queues input; it does not capture output or provide an exit code. After writing, call terminal.read to inspect the visible result. Because this bypasses terminal.exec's safe-prompt checks, call it only when the visible shell prompt is idle; if a foreground command or TUI owns the terminal, that program will receive the text. Do not embed newlines or control bytes; use terminal.write_keys for raw keys, control sequences, or TUI interaction.".to_string(),
            input_schema: write_command_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations {
                title: "Write command to terminal".to_string(),
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
            vec![ResourceCapability::TerminalInput],
        )
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let runtime = self.clone();
        Box::pin(async move { runtime.write_command(input) })
    }
}

fn write_command_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "target": {
                "type": "string",
                "description": "Exact `id` of the active visible terminal whose PTY should receive the command. Call `connections.list_sessions` with capability=\"terminal_input\" and copy an `id` from the result. Do not invent or reuse a stale terminal id."
            },
            "command": {
                "type": "string",
                "minLength": 1,
                "description": "One shell command to write as UTF-8 text. Must not be empty or contain carriage returns, line feeds, NUL, or other control characters. Use terminal.write_keys for multi-line text or raw control bytes."
            },
            "submit": {
                "type": "boolean",
                "default": true,
                "description": "When true, append a carriage return to submit the command, equivalent to pressing Enter. When false, only stage the text in the terminal input line."
            }
        },
        "required": ["target", "command"]
    })
}

fn parse_write_command_request(input: Value) -> Result<TerminalWriteKeysRequest, ToolError> {
    let target = input
        .get("target")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| missing_field("target"))?
        .to_string();
    let command = input
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| missing_field("command"))?;
    if command.trim().is_empty() {
        return Err(ToolError::Failed {
            message: "field `command` must not be empty".to_string(),
        });
    }
    if command.len() > MAX_WRITE_COMMAND_BYTES {
        return Err(ToolError::Failed {
            message: format!(
                "field `command` exceeds the maximum length of {MAX_WRITE_COMMAND_BYTES} bytes"
            ),
        });
    }
    if command
        .chars()
        .any(|character| character.is_control() && character != '\t')
    {
        return Err(ToolError::Failed {
            message: "field `command` must contain exactly one line without control characters; use terminal.write_keys for raw or multi-line input".to_string(),
        });
    }
    let submit = match input.get("submit") {
        None => true,
        Some(Value::Bool(submit)) => *submit,
        Some(_) => {
            return Err(ToolError::Failed {
                message: "field `submit` must be a boolean".to_string(),
            });
        }
    };
    let mut bytes = command.as_bytes().to_vec();
    if submit {
        bytes.push(b'\r');
    }
    Ok(TerminalWriteKeysRequest { target, bytes })
}

fn terminal_write_command_result(result: TerminalWriteKeysResult) -> Result<ToolResult, ToolError> {
    serde_json::to_value(result)
        .map(ToolResult::structured)
        .map_err(tool_error)
}

fn missing_field(field: &str) -> ToolError {
    ToolError::Failed {
        message: format!("missing required field `{field}`"),
    }
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}
