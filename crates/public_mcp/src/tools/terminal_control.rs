use crate::registry::PublicMcpRegistry;
use crate::terminal_control::{
    TerminalControlAction, TerminalControlRequest, TerminalControlResult,
};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ResourceCapability, ResourceKind, RiskLevel, ToolAdapter, ToolAnnotations, ToolContext,
    ToolDescriptor, ToolError, ToolHandler, ToolMode, ToolRegistry, ToolResult, ToolTargetSpec,
};

#[derive(Clone)]
struct TerminalControlRuntime {
    registry: PublicMcpRegistry,
}

impl TerminalControlRuntime {
    fn new(registry: PublicMcpRegistry) -> Self {
        Self { registry }
    }

    async fn control(&self, input: Value, context: ToolContext) -> Result<ToolResult, ToolError> {
        let request = parse_control_request(input)?;
        let target = request.target.clone();
        let result = self
            .registry
            .terminal_control(&target, request, context.cancellation)
            .await
            .map_err(tool_error)?;
        terminal_control_result(result)
    }
}

pub fn terminal_control_tool_registry(registry: PublicMcpRegistry) -> ToolRegistry {
    ToolRegistry::new(vec![Arc::new(TerminalControlRuntime::new(registry))])
}

impl ToolHandler for TerminalControlRuntime {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            id: "terminal.control".to_string(),
            title: "Control terminal".to_string(),
            description: "Send an explicit control action to an active visible terminal session."
                .to_string(),
            input_schema: control_schema(),
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations {
                title: "Control terminal".to_string(),
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
            vec![ResourceCapability::TerminalControl],
        )
    }

    fn call(&self, input: Value, context: ToolContext) -> tool_runtime::ToolFuture {
        let runtime = self.clone();
        Box::pin(async move { runtime.control(input, context).await })
    }
}

fn control_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "target": {
                "type": "string",
                "description": "Active terminal resource id, label, or alias."
            },
            "action": {
                "type": "string",
                "enum": ["interrupt"],
                "description": "Explicit terminal control action."
            }
        },
        "required": ["target", "action"]
    })
}

fn parse_control_request(input: Value) -> Result<TerminalControlRequest, ToolError> {
    let target = required_str(&input, "target")?.to_string();
    let action = match required_str(&input, "action")? {
        "interrupt" => TerminalControlAction::Interrupt,
        action => {
            return Err(ToolError::Failed {
                message: format!("unsupported terminal control action `{action}`"),
            });
        }
    };
    Ok(TerminalControlRequest { target, action })
}

fn terminal_control_result(result: TerminalControlResult) -> Result<ToolResult, ToolError> {
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

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}
