use crate::registry::PublicMcpRegistry;
use crate::terminal_read::{TerminalReadRequest, TerminalReadResult};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ResourceKind, RiskLevel, ToolAdapter, ToolAnnotations, ToolContext, ToolDescriptor, ToolError,
    ToolHandler, ToolMode, ToolRegistry, ToolResult, ToolTargetSpec,
};

const DEFAULT_LINES: usize = 80;
const MAX_LINES: usize = 500;
const MAX_OUTPUT_CHARS: usize = 128 * 1024;

#[derive(Clone)]
struct TerminalReadRuntime {
    registry: PublicMcpRegistry,
}

impl TerminalReadRuntime {
    fn new(registry: PublicMcpRegistry) -> Self {
        Self { registry }
    }

    fn read(&self, input: Value) -> Result<ToolResult, ToolError> {
        let request = parse_read_request(input)?;
        let target = request.target.clone();
        let mut result = self
            .registry
            .terminal_read(&target, request)
            .map_err(tool_error)?;
        let (text, truncated) = truncate_tail_chars(&result.text, MAX_OUTPUT_CHARS);
        result.text = text;
        result.truncated |= truncated;
        if truncated {
            result.returned_lines = result.text.lines().count();
        }
        terminal_read_result(result)
    }
}

pub fn terminal_read_tool_registry(registry: PublicMcpRegistry) -> ToolRegistry {
    ToolRegistry::new(vec![Arc::new(TerminalReadRuntime::new(registry))])
}

impl ToolHandler for TerminalReadRuntime {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "terminal.read".to_string(),
            title: "Read visible terminal output".to_string(),
            description: "Read a bounded tail of the current visible terminal PTY, including scrollback, without executing a command or changing the user's viewport. Use it to inspect output from commands the user ran manually, diagnose the current terminal state, or recover context before deciding whether another command is needed. Prefer ssh.command.output for a known command_id and do not rerun a command merely to see its output. The returned text may contain sensitive terminal content, so request only the number of lines needed.".to_string(),
            input_schema: read_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations {
                title: "Read visible terminal output".to_string(),
                read_only: true,
                destructive: false,
                idempotent: true,
                open_world: false,
                supports_parallel: false,
                risk: RiskLevel::Read,
            },
        }
    }

    fn target_spec(&self) -> ToolTargetSpec {
        ToolTargetSpec::required(vec![ResourceKind::Terminal])
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let runtime = self.clone();
        Box::pin(async move { runtime.read(input) })
    }
}

fn read_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "target": {
                "type": "string",
                "description": "Active visible terminal resource id, label, or alias."
            },
            "lines": {
                "type": "integer",
                "default": DEFAULT_LINES,
                "minimum": 1,
                "maximum": MAX_LINES,
                "description": "Number of most recent physical PTY rows to read from the live buffer and scrollback."
            }
        },
        "required": ["target"]
    })
}

fn parse_read_request(input: Value) -> Result<TerminalReadRequest, ToolError> {
    let target = input
        .get("target")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ToolError::Failed {
            message: "missing required string field `target`".to_string(),
        })?
        .to_string();
    let lines = input
        .get("lines")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(DEFAULT_LINES)
        .clamp(1, MAX_LINES);
    Ok(TerminalReadRequest { target, lines })
}

fn terminal_read_result(result: TerminalReadResult) -> Result<ToolResult, ToolError> {
    serde_json::to_value(result)
        .map(ToolResult::structured)
        .map_err(tool_error)
}

fn truncate_tail_chars(value: &str, max_chars: usize) -> (String, bool) {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return (value.to_string(), false);
    }
    let tail = value
        .chars()
        .skip(char_count.saturating_sub(max_chars))
        .collect();
    (tail, true)
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_request_clamps_requested_lines() {
        let request = parse_read_request(json!({
            "target": "terminal-1",
            "lines": 10_000,
        }))
        .expect("request should parse");

        assert_eq!(MAX_LINES, request.lines);
    }

    #[test]
    fn output_truncation_keeps_the_most_recent_characters() {
        let value = format!("prefix{}tail", "x".repeat(MAX_OUTPUT_CHARS));
        let (truncated, did_truncate) = truncate_tail_chars(&value, MAX_OUTPUT_CHARS);

        assert!(did_truncate);
        assert_eq!(MAX_OUTPUT_CHARS, truncated.chars().count());
        assert!(truncated.ends_with("tail"));
        assert!(!truncated.starts_with("prefix"));
    }
}
