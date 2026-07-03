use crate::approval::{
    PublicMcpApprovalFuture, PublicMcpApprovalManager, PublicMcpApprovalOutcome,
    PublicMcpApprovalRequest, PublicMcpApprover,
};
use crate::permissions::PermissionMode;
use crate::tools::{PublicMcpToolContext, PublicMcpToolRegistry};
use agent_runtime::tools::{
    ObservationData, Tool as AgentTool, ToolInvocation, ToolName, ToolObservation, ToolRegistry,
    ToolSpec,
};
use agent_runtime::{RiskLevel, ToolError};
use async_trait::async_trait;
use rmcp::model::{CallToolResult, Tool};
use serde_json::Value;
use std::sync::Arc;

pub fn agent_runtime_tool_registry(
    registry: PublicMcpToolRegistry,
    _permission_mode: PermissionMode,
    _approver: PublicMcpApprovalManager,
) -> ToolRegistry {
    let mut agent_registry = ToolRegistry::new();
    let context = agent_approved_public_mcp_context();
    for tool in registry.tools() {
        let adapter = PublicMcpAgentTool {
            name: ToolName::new(tool.name.to_string()),
            original_name: tool.name.to_string(),
            description: tool
                .description
                .as_ref()
                .map(|desc| desc.to_string())
                .unwrap_or_default(),
            parameters: Value::Object(tool.input_schema.as_ref().clone()),
            risk: risk_from_mcp_tool(&tool),
            registry: registry.clone(),
            context: context.clone(),
        };
        agent_registry.register(Arc::new(adapter));
    }
    agent_registry
}

struct PublicMcpAgentTool {
    name: ToolName,
    original_name: String,
    description: String,
    parameters: Value,
    risk: RiskLevel,
    registry: PublicMcpToolRegistry,
    context: PublicMcpToolContext,
}

#[async_trait]
impl AgentTool for PublicMcpAgentTool {
    fn name(&self) -> ToolName {
        self.name.clone()
    }

    fn spec(&self, _resources: &agent_runtime::ResourceContext) -> ToolSpec {
        ToolSpec::new(
            self.name.clone(),
            self.description.clone(),
            self.parameters.clone(),
        )
        .with_risk(self.risk)
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        let arguments = invocation_arguments(invocation.arguments.clone())?;
        let result = self
            .registry
            .call_tool(&self.original_name, Some(arguments), self.context.clone())
            .await
            .map_err(|error| ToolError::Execution(error.to_string()))?;
        Ok(observation_from_result(invocation, result))
    }
}

fn agent_approved_public_mcp_context() -> PublicMcpToolContext {
    PublicMcpToolContext {
        permission_mode: PermissionMode::Allow,
        approver: PublicMcpApprovalManager::new(Arc::new(AgentApprovedApprover)),
    }
}

fn risk_from_mcp_tool(tool: &Tool) -> RiskLevel {
    let Some(annotations) = &tool.annotations else {
        return RiskLevel::Medium;
    };
    if annotations.read_only_hint.unwrap_or(false) {
        return RiskLevel::Read;
    }
    if annotations.destructive_hint.unwrap_or(false) || annotations.open_world_hint.unwrap_or(false)
    {
        return RiskLevel::High;
    }
    RiskLevel::Medium
}

struct AgentApprovedApprover;

impl PublicMcpApprover for AgentApprovedApprover {
    fn request_approval(&self, _request: PublicMcpApprovalRequest) -> PublicMcpApprovalFuture {
        Box::pin(async { PublicMcpApprovalOutcome::Approved })
    }
}

fn invocation_arguments(arguments: Value) -> Result<rmcp::model::JsonObject, ToolError> {
    match arguments {
        Value::Object(object) => Ok(object),
        other => Err(ToolError::InvalidArguments(format!(
            "expected object arguments, got {other}"
        ))),
    }
}

fn observation_from_result(invocation: ToolInvocation, result: CallToolResult) -> ToolObservation {
    let success = !result.is_error.unwrap_or(false);
    let data = result_data(&result);
    let summary = data.to_text();
    let summary = if summary.trim().is_empty() {
        if success {
            "Tool succeeded".to_string()
        } else {
            "Tool failed".to_string()
        }
    } else {
        summary
    };

    let observation = if success {
        ToolObservation::success(invocation.call_id, invocation.tool_name, summary, data)
    } else {
        ToolObservation::failure(invocation.call_id, invocation.tool_name, summary)
    };
    observation.with_resource(invocation.resource_id)
}

fn result_data(result: &CallToolResult) -> ObservationData {
    if let Some(value) = result.structured_content.clone() {
        return ObservationData::Json(value);
    }
    let text = result
        .content
        .iter()
        .map(|content| {
            content
                .as_text()
                .map(|text| text.text.clone())
                .unwrap_or_else(|| {
                    serde_json::to_string(content).unwrap_or_else(|_| format!("{content:?}"))
                })
        })
        .collect::<Vec<_>>()
        .join("\n");
    ObservationData::Text(text)
}

#[cfg(test)]
mod tests {
    use super::agent_runtime_tool_registry;
    use crate::approval::PublicMcpApprovalManager;
    use crate::permissions::PermissionMode;
    use crate::tools::{PublicMcpToolContext, PublicMcpToolFuture, PublicMcpToolProvider};
    use agent_runtime::{
        ResourceContext, SessionId, ToolName, TurnId, ids::ToolCallId, tools::ToolInvocation,
    };
    use rmcp::model::{CallToolResult, Content, JsonObject, Tool};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn adapter_exposes_public_mcp_tool_specs_and_executes_calls() {
        let provider = Arc::new(RecordingProvider::default());
        let registry = crate::tools::PublicMcpToolRegistry::new(vec![provider.clone()]);
        let agent_registry = agent_runtime_tool_registry(
            registry,
            PermissionMode::Deny,
            PublicMcpApprovalManager::default(),
        );
        let resources = ResourceContext::new();
        let specs = agent_registry.specs(&resources);

        assert_eq!(1, specs.len());
        assert_eq!("sample_echo", specs[0].name.as_str());
        assert_eq!("Echo input", specs[0].description);
        assert_eq!("object", specs[0].parameters["type"]);

        let tool = agent_registry
            .get(&ToolName::new("sample.echo"))
            .expect("tool should be registered");
        let observation = tool
            .execute(invocation(json!({ "message": "hello" })))
            .await
            .unwrap();

        assert!(observation.success);
        assert_eq!("sample_echo", observation.tool_name.as_str());
        assert_eq!("hello", observation.summary);
        assert_eq!(Some("sample.echo".to_string()), provider.last_name());
        assert_eq!(
            Some(json!({ "message": "hello" })),
            provider.last_arguments()
        );
    }

    #[derive(Default)]
    struct RecordingProvider {
        last_name: Mutex<Option<String>>,
        last_arguments: Mutex<Option<serde_json::Value>>,
    }

    impl RecordingProvider {
        fn last_name(&self) -> Option<String> {
            self.last_name
                .lock()
                .expect("name lock should not be poisoned")
                .clone()
        }

        fn last_arguments(&self) -> Option<serde_json::Value> {
            self.last_arguments
                .lock()
                .expect("arguments lock should not be poisoned")
                .clone()
        }
    }

    impl PublicMcpToolProvider for RecordingProvider {
        fn tools(&self) -> Vec<Tool> {
            vec![Tool::new("sample.echo", "Echo input", schema())]
        }

        fn call_tool(
            &self,
            name: &str,
            arguments: Option<JsonObject>,
            _context: PublicMcpToolContext,
        ) -> Option<PublicMcpToolFuture> {
            if name != "sample.echo" {
                return None;
            }
            let value = serde_json::Value::Object(arguments.unwrap_or_default());
            *self
                .last_name
                .lock()
                .expect("name lock should not be poisoned") = Some(name.to_string());
            *self
                .last_arguments
                .lock()
                .expect("arguments lock should not be poisoned") = Some(value.clone());
            Some(Box::pin(async move {
                Ok(CallToolResult::success(vec![Content::text(
                    value["message"].as_str().unwrap_or_default().to_string(),
                )]))
            }))
        }
    }

    fn invocation(arguments: serde_json::Value) -> ToolInvocation {
        ToolInvocation {
            session_id: SessionId::from_string("session_1"),
            turn_id: TurnId::from_string("turn_1"),
            call_id: ToolCallId::from_string("call_1"),
            tool_name: ToolName::new("sample.echo"),
            arguments,
            resource_id: None,
            resources: ResourceContext::new(),
            cancellation: CancellationToken::new(),
        }
    }

    fn schema() -> JsonObject {
        let value = json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            }
        });
        match value {
            serde_json::Value::Object(object) => object,
            _ => unreachable!("schema should be an object"),
        }
    }
}
