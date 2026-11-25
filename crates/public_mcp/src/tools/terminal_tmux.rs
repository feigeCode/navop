use crate::registry::{PublicMcpRegistry, TmuxSendKeysRequest, TmuxSendKeysResult};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ResourceCapability, ResourceKind, RiskLevel, ToolAdapter, ToolAnnotations, ToolContext,
    ToolDescriptor, ToolError, ToolHandler, ToolMode, ToolRegistry, ToolResult, ToolTargetSpec,
};

#[derive(Clone)]
struct TerminalTmuxListRuntime {
    registry: PublicMcpRegistry,
}

impl TerminalTmuxListRuntime {
    fn new(registry: PublicMcpRegistry) -> Self {
        Self { registry }
    }

    fn list(&self, input: Value) -> Result<ToolResult, ToolError> {
        let target = required_string(&input, "target")?;
        let result = self
            .registry
            .terminal_tmux_list_panes(&target)
            .map_err(tool_error)?;
        let result = json!({
            "target": target,
            "session_id": result.session_id,
            "session_name": result.session_name,
            "active_pane": result.active_pane,
            "panes": result.panes,
        });
        Ok(ToolResult::structured(result))
    }
}

pub fn terminal_tmux_list_tool_registry(registry: PublicMcpRegistry) -> ToolRegistry {
    ToolRegistry::new(vec![Arc::new(TerminalTmuxListRuntime::new(registry))])
}

impl ToolHandler for TerminalTmuxListRuntime {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "terminal.tmux_list_panes".to_string(),
            title: "List tmux panes".to_string(),
            description: "List the panes of the tmux session attached to a visible terminal. Use this before terminal.tmux_send_keys when you need to address a pane other than the active one. The result is read from the latest control-channel snapshot and does not execute a shell command in a pane.".to_string(),
            input_schema: target_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations {
                title: "List tmux panes".to_string(),
                read_only: true,
                destructive: false,
                idempotent: true,
                open_world: false,
                supports_parallel: true,
                risk: RiskLevel::Low,
            },
        }
    }

    fn target_spec(&self) -> ToolTargetSpec {
        ToolTargetSpec::required_with_capabilities(
            vec![ResourceKind::Terminal],
            vec![ResourceCapability::Tmux],
        )
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let runtime = self.clone();
        Box::pin(async move { runtime.list(input) })
    }
}

#[derive(Clone)]
struct TerminalTmuxSendKeysRuntime {
    registry: PublicMcpRegistry,
}

impl TerminalTmuxSendKeysRuntime {
    fn new(registry: PublicMcpRegistry) -> Self {
        Self { registry }
    }

    async fn send(&self, input: Value, context: ToolContext) -> Result<ToolResult, ToolError> {
        let target = required_string(&input, "target")?;
        let pane_id = required_string(&input, "pane_id")?;
        validate_pane_id(&pane_id)?;
        let keys = required_string(&input, "keys")?;
        let submit = input.get("submit").and_then(Value::as_bool).unwrap_or(true);
        let result = self
            .registry
            .terminal_tmux_send_keys(
                &target,
                TmuxSendKeysRequest {
                    pane_id,
                    keys,
                    submit,
                },
                context.cancellation,
            )
            .await
            .map_err(tool_error)?;
        serialize_result(result)
    }
}

pub fn terminal_tmux_send_keys_tool_registry(registry: PublicMcpRegistry) -> ToolRegistry {
    ToolRegistry::new(vec![Arc::new(TerminalTmuxSendKeysRuntime::new(registry))])
}

impl ToolHandler for TerminalTmuxSendKeysRuntime {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "terminal.tmux_send_keys".to_string(),
            title: "Send keys to a tmux pane".to_string(),
            description: "Send one line of literal text to a specific tmux pane and optionally submit it with Enter. `target` and `pane_id` are required; use terminal.tmux_list_panes first. This bypasses terminal.exec and does not capture output. Keys must be UTF-8 text without newlines or control bytes; use terminal.write_keys for raw control input.".to_string(),
            input_schema: send_keys_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations {
                title: "Send keys to a tmux pane".to_string(),
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
            vec![ResourceCapability::Tmux, ResourceCapability::TerminalInput],
        )
    }

    fn call(&self, input: Value, context: ToolContext) -> tool_runtime::ToolFuture {
        let runtime = self.clone();
        Box::pin(async move { runtime.send(input, context).await })
    }
}

fn target_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "target": {
                "type": "string",
                "description": "Exact id of the visible terminal whose tmux session should be inspected."
            }
        },
        "required": ["target"]
    })
}

fn send_keys_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "target": {
                "type": "string",
                "description": "Exact id of the visible terminal whose tmux session should receive keys."
            },
            "pane_id": {
                "type": "string",
                "pattern": "^%[0-9]+$",
                "description": "tmux pane id from terminal.tmux_list_panes, for example `%3`."
            },
            "keys": {
                "type": "string",
                "minLength": 1,
                "description": "One line of UTF-8 text. Newlines and control bytes are rejected."
            },
            "submit": {
                "type": "boolean",
                "default": true,
                "description": "When true, send Enter after the literal text."
            }
        },
        "required": ["target", "pane_id", "keys"]
    })
}

fn required_string(input: &Value, field: &str) -> Result<String, ToolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ToolError::Failed {
            message: format!("field `{field}` is required"),
        })
}

fn validate_pane_id(pane_id: &str) -> Result<(), ToolError> {
    let valid = pane_id
        .strip_prefix('%')
        .is_some_and(|number| !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit()));
    if valid {
        Ok(())
    } else {
        Err(ToolError::Failed {
            message: "field `pane_id` must have the form `%<number>`".to_string(),
        })
    }
}

fn serialize_result(result: TmuxSendKeysResult) -> Result<ToolResult, ToolError> {
    serde_json::to_value(result)
        .map(ToolResult::structured)
        .map_err(tool_error)
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_pane_id;

    #[test]
    fn validates_tmux_pane_ids() {
        assert!(validate_pane_id("%0").is_ok());
        assert!(validate_pane_id("%42").is_ok());
        assert!(validate_pane_id("0").is_err());
        assert!(validate_pane_id("%x").is_err());
    }
}
