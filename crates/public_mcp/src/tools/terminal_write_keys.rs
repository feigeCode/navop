use crate::registry::PublicMcpRegistry;
use crate::terminal_write_keys::{TerminalWriteKeysRequest, TerminalWriteKeysResult};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ResourceCapability, ResourceKind, RiskLevel, ToolAdapter, ToolAnnotations, ToolContext,
    ToolDescriptor, ToolError, ToolHandler, ToolMode, ToolRegistry, ToolResult, ToolTargetSpec,
};

const MAX_WRITE_KEYS_BYTES: usize = 8 * 1024;

#[derive(Clone)]
struct TerminalWriteKeysRuntime {
    registry: PublicMcpRegistry,
}

impl TerminalWriteKeysRuntime {
    fn new(registry: PublicMcpRegistry) -> Self {
        Self { registry }
    }

    fn write_keys(&self, input: Value) -> Result<ToolResult, ToolError> {
        let request = parse_write_keys_request(input)?;
        let target = request.target.clone();
        let result = self
            .registry
            .terminal_write_keys(&target, request)
            .map_err(tool_error)?;
        terminal_write_keys_result(result)
    }
}

pub fn terminal_write_keys_tool_registry(registry: PublicMcpRegistry) -> ToolRegistry {
    ToolRegistry::new(vec![Arc::new(TerminalWriteKeysRuntime::new(registry))])
}

impl ToolHandler for TerminalWriteKeysRuntime {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "terminal.write_keys".to_string(),
            title: "Write raw keys to terminal".to_string(),
            description: "Write an explicit bounded sequence of raw bytes directly to the PTY of the current visible terminal, including when a foreground TUI such as vim is running. This bypasses shell readiness and is not a replacement for terminal.exec or terminal.control. Use only when the user explicitly asks to interact with the foreground terminal program; the bytes can change or destroy the current terminal state. For example, vim save-and-quit is [58,119,113,13] (`:wq` followed by carriage return). The result confirms that the bytes were queued for the PTY backend, not that the foreground program has completed; call terminal.read to verify the visible result.".to_string(),
            input_schema: write_keys_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations {
                title: "Write raw keys to terminal".to_string(),
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
        Box::pin(async move { runtime.write_keys(input) })
    }
}

fn write_keys_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "target": {
                "type": "string",
                "description": "Exact `id` of the active visible terminal whose foreground PTY should receive the bytes. Call `connections.list_sessions` with capability=\"terminal_input\" and copy an `id` from the result. Do not invent or reuse a stale terminal id."
            },
            "bytes": {
                "type": "array",
                "items": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 255
                },
                "minItems": 1,
                "maxItems": MAX_WRITE_KEYS_BYTES,
                "description": "Raw byte values to write to the PTY, each from 0 through 255. For example, `:wq` plus carriage return is [58,119,113,13]."
            }
        },
        "required": ["target", "bytes"]
    })
}

fn parse_write_keys_request(input: Value) -> Result<TerminalWriteKeysRequest, ToolError> {
    let target = input
        .get("target")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| missing_field("target"))?
        .to_string();
    let bytes = input
        .get("bytes")
        .and_then(Value::as_array)
        .ok_or_else(|| missing_field("bytes"))?;
    if bytes.is_empty() {
        return Err(ToolError::Failed {
            message: "field `bytes` must not be empty".to_string(),
        });
    }
    if bytes.len() > MAX_WRITE_KEYS_BYTES {
        return Err(ToolError::Failed {
            message: format!("field `bytes` exceeds the maximum length of {MAX_WRITE_KEYS_BYTES}"),
        });
    }
    let bytes = bytes
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let byte = value.as_u64().ok_or_else(|| ToolError::Failed {
                message: format!("field `bytes[{index}]` must be an integer from 0 through 255"),
            })?;
            u8::try_from(byte).map_err(|_| ToolError::Failed {
                message: format!("field `bytes[{index}]` must be an integer from 0 through 255"),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TerminalWriteKeysRequest { target, bytes })
}

fn terminal_write_keys_result(result: TerminalWriteKeysResult) -> Result<ToolResult, ToolError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raw_bytes_without_utf8_conversion() {
        let request = parse_write_keys_request(json!({
            "target": "terminal-1",
            "bytes": [0, 27, 58, 119, 113, 13, 255],
        }))
        .expect("raw byte array should parse");

        assert_eq!(vec![0, 27, 58, 119, 113, 13, 255], request.bytes);
    }

    #[test]
    fn rejects_empty_bytes() {
        let error = parse_write_keys_request(json!({
            "target": "terminal-1",
            "bytes": [],
        }))
        .expect_err("empty raw input should be rejected");

        assert!(error.to_string().contains("must not be empty"));
    }

    #[test]
    fn rejects_values_outside_byte_range() {
        let error = parse_write_keys_request(json!({
            "target": "terminal-1",
            "bytes": [256],
        }))
        .expect_err("out-of-range raw input should be rejected");

        assert!(error.to_string().contains("0 through 255"));
    }
}
