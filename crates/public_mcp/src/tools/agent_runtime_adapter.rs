use crate::approval::{
    PublicMcpApprovalFuture, PublicMcpApprovalManager, PublicMcpApprovalOutcome,
    PublicMcpApprovalRequest, PublicMcpApprover,
};
use crate::permissions::PermissionMode;
use crate::tools::{PublicMcpToolContext, PublicMcpToolRegistry};
use agent_runtime::tools::{
    ObservationData, Tool as AgentTool, ToolInvocation, ToolName, ToolNameAllocator,
    ToolObservation, ToolRegistry, ToolSpec,
};
use agent_runtime::{ResourceContext, ResourceKind, RiskLevel, ToolError};
use async_trait::async_trait;
use rmcp::model::{CallToolResult, JsonObject, Tool};
use serde_json::Value;
use std::sync::Arc;

/// 允许模型省略 `target` 的工具，及其需要的资源类型。
///
/// 省略时由本适配器补上会话的默认资源；显式传 `target` 的行为不变。
/// 只收可省略且不增加风险的读/执行入口：`terminal.control`、
/// `terminal.write_command` 与 `terminal.write_keys` 要求显式 target，
/// 因为它们是可控到单个前台程序的危险动作。
const DEFAULT_TARGET_TOOLS: [(&str, ResourceKind); 2] = [
    ("terminal.read", ResourceKind::Terminal),
    ("terminal.exec", ResourceKind::Terminal),
];
const TARGET_FIELD: &str = "target";
const PROPERTIES_FIELD: &str = "properties";
const REQUIRED_FIELD: &str = "required";
const DESCRIPTION_FIELD: &str = "description";
const DEFAULT_TARGET_DESCRIPTION: &str = "Exact `id` of the terminal to use. Omit it to use the default terminal of this session \
     (the resource marked [current], or the only terminal available). Pass an explicit id from \
     `connections.list_sessions` with kind=\"terminal\" to target another terminal.";
const DEFAULT_TARGET_TOOL_HINT: &str = "The `target` field may be omitted to use the default \
     terminal of this session.";

pub fn agent_runtime_tool_registry(
    registry: PublicMcpToolRegistry,
    _permission_mode: PermissionMode,
    _approver: PublicMcpApprovalManager,
) -> ToolRegistry {
    let mut agent_registry = ToolRegistry::new();
    let context = agent_approved_public_mcp_context();
    let mut tools = registry.tools();
    tools.sort_by(|left, right| left.name.cmp(&right.name));
    let mut names = ToolNameAllocator::default();
    for tool in tools {
        let original_name = tool.name.to_string();
        let adapter = PublicMcpAgentTool {
            name: names.allocate(&original_name),
            description: agent_description(&original_name, &tool),
            parameters: agent_parameters(&original_name, &tool),
            original_name,
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
        let arguments = with_default_target(
            &self.original_name,
            arguments,
            &invocation.resources,
            declares_target_field(&self.parameters),
        )?;
        let result = self
            .registry
            .call_tool_with_cancellation(
                &self.original_name,
                Some(arguments),
                self.context.clone(),
                invocation.cancellation.clone(),
            )
            .await
            .map_err(|error| ToolError::Execution(error.to_string()))?;
        Ok(observation_from_result(invocation, result))
    }
}

fn agent_approved_public_mcp_context() -> PublicMcpToolContext {
    // Agent Runtime already applies its own risk/tool-mode approval policy. Avoid
    // running the Public MCP approval layer a second time for an approved call.
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

/// 该工具是否允许省略 `target`；是则返回它需要的资源类型。
fn default_target_kind(tool_name: &str) -> Option<&'static ResourceKind> {
    DEFAULT_TARGET_TOOLS
        .iter()
        .find(|(name, _)| *name == tool_name)
        .map(|(_, kind)| kind)
}

/// 生成交给模型的参数 schema：允许省略 `target` 的工具去掉必填约束。
fn agent_parameters(tool_name: &str, tool: &Tool) -> Value {
    let mut schema = Value::Object(tool.input_schema.as_ref().clone());
    if default_target_kind(tool_name).is_none() {
        return schema;
    }
    let Some(object) = schema.as_object_mut() else {
        return schema;
    };
    if let Some(Value::Array(required)) = object.get_mut(REQUIRED_FIELD) {
        required.retain(|field| field.as_str() != Some(TARGET_FIELD));
        if required.is_empty() {
            object.remove(REQUIRED_FIELD);
        }
    }
    if let Some(Value::Object(properties)) = object.get_mut(PROPERTIES_FIELD) {
        if let Some(Value::Object(target)) = properties.get_mut(TARGET_FIELD) {
            target.insert(
                DESCRIPTION_FIELD.to_string(),
                Value::String(DEFAULT_TARGET_DESCRIPTION.to_string()),
            );
        }
    }
    schema
}

/// 生成交给模型的工具描述：允许省略 `target` 的工具在末尾补一句提示。
fn agent_description(tool_name: &str, tool: &Tool) -> String {
    let description = tool
        .description
        .as_ref()
        .map(|desc| desc.to_string())
        .unwrap_or_default();
    if default_target_kind(tool_name).is_none() {
        return description;
    }
    if description.is_empty() {
        return DEFAULT_TARGET_TOOL_HINT.to_string();
    }
    format!("{description} {DEFAULT_TARGET_TOOL_HINT}")
}

/// 转接层的工具是否真的声明了 `target` 字段。
///
/// `DEFAULT_TARGET_TOOLS` 按工具名匹配，但同名的外部 provider 未必提供 `target`
/// （例如只接 `command` 的简单工具）：此时不能凭空要求一个 schema 里不存在的字段。
fn declares_target_field(parameters: &Value) -> bool {
    parameters
        .get(PROPERTIES_FIELD)
        .and_then(Value::as_object)
        .is_some_and(|properties| properties.contains_key(TARGET_FIELD))
}

/// 模型省略 `target` 时补上会话的默认资源。
///
/// 只有泛型上允许省略、且 schema 确实声明了 `target` 的工具才会被替换；
/// 显式 target（含空串）保持原样，由工具自己报参数错误。找不到可用默认资源时
/// 给出可执行的下一步。
fn with_default_target(
    tool_name: &str,
    mut arguments: JsonObject,
    resources: &ResourceContext,
    declares_target: bool,
) -> Result<JsonObject, ToolError> {
    let Some(kind) = default_target_kind(tool_name) else {
        return Ok(arguments);
    };
    if !declares_target {
        return Ok(arguments);
    }
    let explicit = arguments
        .get(TARGET_FIELD)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty());
    if explicit {
        return Ok(arguments);
    }
    let Some(resource) = resources.default_target_of_kind(kind) else {
        return Err(ToolError::InvalidArguments(format!(
            "{tool_name} needs an explicit `target`: this session has no unique {} resource. \
             Call connections.list_sessions, then pass an `id` from the result",
            kind.as_str()
        )));
    };
    arguments.insert(
        TARGET_FIELD.to_string(),
        Value::String(resource.id.as_str().to_string()),
    );
    Ok(arguments)
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
    use super::{DEFAULT_TARGET_TOOL_HINT, REQUIRED_FIELD, agent_runtime_tool_registry};
    use crate::approval::PublicMcpApprovalManager;
    use crate::permissions::PermissionMode;
    use crate::tools::{PublicMcpToolContext, PublicMcpToolFuture, PublicMcpToolProvider};
    use agent_runtime::{
        ResourceContext, ResourceKind, SessionId, SkillContext, ToolName, TurnId, ids::ToolCallId,
        tools::ToolInvocation,
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

    #[tokio::test]
    async fn adapter_allocates_stable_unique_names_for_sanitized_collisions() {
        let reverse_provider = Arc::new(CollisionProvider::new(true));
        let forward_provider = Arc::new(CollisionProvider::new(false));
        let reverse_registry = agent_runtime_tool_registry(
            crate::tools::PublicMcpToolRegistry::new(vec![reverse_provider.clone()]),
            PermissionMode::Deny,
            PublicMcpApprovalManager::default(),
        );
        let forward_registry = agent_runtime_tool_registry(
            crate::tools::PublicMcpToolRegistry::new(vec![forward_provider]),
            PermissionMode::Deny,
            PublicMcpApprovalManager::default(),
        );

        let names = |registry: &agent_runtime::ToolRegistry| {
            registry
                .names()
                .into_iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            vec!["sample_echo".to_string(), "sample_echo_2".to_string()],
            names(&reverse_registry)
        );
        assert_eq!(names(&reverse_registry), names(&forward_registry));

        reverse_registry
            .get(&ToolName::new("sample_echo"))
            .expect("dotted MCP tool should use the base public name")
            .execute(invocation_for(
                "sample_echo",
                json!({ "message": "dotted" }),
            ))
            .await
            .expect("dotted MCP tool should execute");
        reverse_registry
            .get(&ToolName::new("sample_echo_2"))
            .expect("underscore MCP tool should use the suffixed public name")
            .execute(invocation_for(
                "sample_echo_2",
                json!({ "message": "underscore" }),
            ))
            .await
            .expect("underscore MCP tool should execute");

        assert_eq!(
            vec!["sample.echo".to_string(), "sample_echo".to_string()],
            reverse_provider.called_names()
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

    struct CollisionProvider {
        reverse: bool,
        called_names: Mutex<Vec<String>>,
    }

    impl CollisionProvider {
        fn new(reverse: bool) -> Self {
            Self {
                reverse,
                called_names: Mutex::new(Vec::new()),
            }
        }

        fn called_names(&self) -> Vec<String> {
            self.called_names
                .lock()
                .expect("called names lock should not be poisoned")
                .clone()
        }
    }

    impl PublicMcpToolProvider for CollisionProvider {
        fn tools(&self) -> Vec<Tool> {
            let mut tools = vec![
                Tool::new("sample.echo", "Dotted echo", schema()),
                Tool::new("sample_echo", "Underscore echo", schema()),
            ];
            if self.reverse {
                tools.reverse();
            }
            tools
        }

        fn call_tool(
            &self,
            name: &str,
            arguments: Option<JsonObject>,
            _context: PublicMcpToolContext,
        ) -> Option<PublicMcpToolFuture> {
            if !matches!(name, "sample.echo" | "sample_echo") {
                return None;
            }
            self.called_names
                .lock()
                .expect("called names lock should not be poisoned")
                .push(name.to_string());
            let value = serde_json::Value::Object(arguments.unwrap_or_default());
            Some(Box::pin(async move {
                Ok(CallToolResult::success(vec![Content::text(
                    value["message"].as_str().unwrap_or_default().to_string(),
                )]))
            }))
        }
    }

    #[tokio::test]
    async fn terminal_read_spec_allows_omitting_target() {
        let provider = Arc::new(TerminalReadProvider::default());
        let registry = agent_runtime_tool_registry(
            crate::tools::PublicMcpToolRegistry::new(vec![provider]),
            PermissionMode::Deny,
            PublicMcpApprovalManager::default(),
        );

        let spec = registry
            .specs(&ResourceContext::new())
            .into_iter()
            .find(|spec| spec.name.as_str() == "terminal_read")
            .expect("terminal.read spec should be registered");

        assert!(
            spec.parameters.get(REQUIRED_FIELD).is_none(),
            "target 可省略时不应再有必填字段: {}",
            spec.parameters
        );
        assert!(
            spec.parameters["properties"]["target"]["description"]
                .as_str()
                .is_some_and(|description| description.contains("Omit it")),
            "target 描述应说明可省略: {}",
            spec.parameters["properties"]["target"]
        );
        assert!(
            spec.description.contains(DEFAULT_TARGET_TOOL_HINT),
            "工具描述应提示可省略 target: {}",
            spec.description
        );
    }

    #[tokio::test]
    async fn terminal_read_injects_default_terminal_when_target_omitted() {
        let provider = Arc::new(TerminalReadProvider::default());
        let registry = agent_runtime_tool_registry(
            crate::tools::PublicMcpToolRegistry::new(vec![provider.clone()]),
            PermissionMode::Deny,
            PublicMcpApprovalManager::default(),
        );
        let resources = ResourceContext::new().with_resource(terminal_resource("term-1"));

        registry
            .get(&ToolName::new("terminal_read"))
            .expect("tool should be registered")
            .execute(invocation_with_resources(
                "terminal_read",
                json!({ "lines": 20 }),
                resources,
            ))
            .await
            .expect("call should succeed");

        assert_eq!(
            Some(json!({ "lines": 20, "target": "term-1" })),
            provider.last_arguments()
        );
    }

    #[tokio::test]
    async fn terminal_read_keeps_explicit_target() {
        let provider = Arc::new(TerminalReadProvider::default());
        let registry = agent_runtime_tool_registry(
            crate::tools::PublicMcpToolRegistry::new(vec![provider.clone()]),
            PermissionMode::Deny,
            PublicMcpApprovalManager::default(),
        );

        registry
            .get(&ToolName::new("terminal_read"))
            .expect("tool should be registered")
            .execute(invocation_with_resources(
                "terminal_read",
                json!({ "target": "term-2" }),
                ResourceContext::new(),
            ))
            .await
            .expect("call should succeed");

        assert_eq!(
            Some(json!({ "target": "term-2" })),
            provider.last_arguments()
        );
    }

    #[tokio::test]
    async fn terminal_read_prefers_terminal_over_current_connection() {
        let provider = Arc::new(TerminalReadProvider::default());
        let registry = agent_runtime_tool_registry(
            crate::tools::PublicMcpToolRegistry::new(vec![provider.clone()]),
            PermissionMode::Deny,
            PublicMcpApprovalManager::default(),
        );
        let connection = agent_runtime::ResourceRef::new("74", ResourceKind::Ssh, "86服务器");
        let resources = ResourceContext {
            current: Some(agent_runtime::ResourceId::new("74")),
            resources: vec![connection, terminal_resource("term-1")],
        };

        registry
            .get(&ToolName::new("terminal_read"))
            .expect("tool should be registered")
            .execute(invocation_with_resources(
                "terminal_read",
                json!({}),
                resources,
            ))
            .await
            .expect("call should succeed");

        assert_eq!(
            Some(json!({ "target": "term-1" })),
            provider.last_arguments()
        );
    }

    #[tokio::test]
    async fn terminal_read_without_terminals_asks_for_explicit_target() {
        let provider = Arc::new(TerminalReadProvider::default());
        let registry = agent_runtime_tool_registry(
            crate::tools::PublicMcpToolRegistry::new(vec![provider.clone()]),
            PermissionMode::Deny,
            PublicMcpApprovalManager::default(),
        );

        let error = registry
            .get(&ToolName::new("terminal_read"))
            .expect("tool should be registered")
            .execute(invocation_with_resources(
                "terminal_read",
                json!({}),
                ResourceContext::new(),
            ))
            .await
            .expect_err("缺少默认终端时应报错");

        assert!(
            error.to_string().contains("connections.list_sessions"),
            "错误信息应给出下一步: {error}"
        );
    }

    fn terminal_resource(id: &str) -> agent_runtime::ResourceRef {
        agent_runtime::ResourceRef::new(id, ResourceKind::Terminal, "本地终端")
    }

    #[tokio::test]
    async fn terminal_exec_spec_allows_omitting_target_and_injects_it() {
        let provider = Arc::new(TerminalExecProvider::default());
        let registry = agent_runtime_tool_registry(
            crate::tools::PublicMcpToolRegistry::new(vec![provider.clone()]),
            PermissionMode::Deny,
            PublicMcpApprovalManager::default(),
        );

        let spec = registry
            .specs(&ResourceContext::new())
            .into_iter()
            .find(|spec| spec.name.as_str() == "terminal_exec")
            .expect("terminal.exec spec should be registered");
        assert!(
            spec.parameters[REQUIRED_FIELD]
                .as_array()
                .is_none_or(|required| {
                    !required
                        .iter()
                        .any(|field| field.as_str() == Some("target"))
                }),
            "terminal.exec 的 target 应可省略: {}",
            spec.parameters
        );

        registry
            .get(&ToolName::new("terminal_exec"))
            .expect("tool should be registered")
            .execute(invocation_with_resources(
                "terminal_exec",
                json!({ "command": "pwd" }),
                ResourceContext::new().with_resource(terminal_resource("term-1")),
            ))
            .await
            .expect("call should succeed");

        assert_eq!(
            Some(json!({ "command": "pwd", "target": "term-1" })),
            provider.last_arguments()
        );
    }

    #[test]
    fn default_target_injection_needs_the_tool_to_declare_target() {
        let arguments = json!({ "command": "pwd" })
            .as_object()
            .cloned()
            .expect("object arguments");

        let injected =
            super::with_default_target("terminal.exec", arguments, &ResourceContext::new(), false)
                .expect("schema 里没有 target 时不应报错");

        assert_eq!(
            json!({ "command": "pwd" }),
            serde_json::Value::Object(injected)
        );
    }

    #[tokio::test]
    async fn terminal_exec_with_several_terminals_asks_for_explicit_target() {
        let provider = Arc::new(TerminalExecProvider::default());
        let registry = agent_runtime_tool_registry(
            crate::tools::PublicMcpToolRegistry::new(vec![provider]),
            PermissionMode::Deny,
            PublicMcpApprovalManager::default(),
        );
        // SSH 侧栏的当前资源是连接，终端只是候选：多于一个时必须报错而不是猜一个。
        let resources = ResourceContext {
            current: Some(agent_runtime::ResourceId::new("74")),
            resources: vec![
                agent_runtime::ResourceRef::new("74", ResourceKind::Ssh, "86服务器"),
                terminal_resource("term-1"),
                terminal_resource("term-2"),
            ],
        };

        let error = registry
            .get(&ToolName::new("terminal_exec"))
            .expect("tool should be registered")
            .execute(invocation_with_resources(
                "terminal_exec",
                json!({ "command": "pwd" }),
                resources,
            ))
            .await
            .expect_err("多个终端时不应猜目标");

        assert!(
            error.to_string().contains("connections.list_sessions"),
            "错误信息应给出下一步: {error}"
        );
    }

    fn invocation_with_resources(
        tool_name: &str,
        arguments: serde_json::Value,
        resources: ResourceContext,
    ) -> ToolInvocation {
        let mut invocation = invocation_for(tool_name, arguments);
        invocation.resources = resources;
        invocation
    }

    #[derive(Default)]
    struct TerminalExecProvider {
        last_arguments: Mutex<Option<serde_json::Value>>,
    }

    impl TerminalExecProvider {
        fn last_arguments(&self) -> Option<serde_json::Value> {
            self.last_arguments
                .lock()
                .expect("arguments lock should not be poisoned")
                .clone()
        }
    }

    impl PublicMcpToolProvider for TerminalExecProvider {
        fn tools(&self) -> Vec<Tool> {
            let schema = json!({
                "type": "object",
                "properties": {
                    "target": { "type": "string", "description": "Exact terminal id" },
                    "command": { "type": "string" }
                },
                "required": ["target", "command"]
            });
            let schema = match schema {
                serde_json::Value::Object(object) => object,
                _ => unreachable!("schema should be an object"),
            };
            vec![Tool::new(
                "terminal.exec",
                "Run a command in the visible terminal",
                schema,
            )]
        }

        fn call_tool(
            &self,
            name: &str,
            arguments: Option<JsonObject>,
            _context: PublicMcpToolContext,
        ) -> Option<PublicMcpToolFuture> {
            if name != "terminal.exec" {
                return None;
            }
            let value = serde_json::Value::Object(arguments.unwrap_or_default());
            *self
                .last_arguments
                .lock()
                .expect("arguments lock should not be poisoned") = Some(value);
            Some(Box::pin(async {
                Ok(CallToolResult::success(vec![Content::text("ok")]))
            }))
        }
    }

    #[derive(Default)]
    struct TerminalReadProvider {
        last_arguments: Mutex<Option<serde_json::Value>>,
    }

    impl TerminalReadProvider {
        fn last_arguments(&self) -> Option<serde_json::Value> {
            self.last_arguments
                .lock()
                .expect("arguments lock should not be poisoned")
                .clone()
        }
    }

    impl PublicMcpToolProvider for TerminalReadProvider {
        fn tools(&self) -> Vec<Tool> {
            let schema = json!({
                "type": "object",
                "properties": {
                    "target": { "type": "string", "description": "Exact terminal id" },
                    "lines": { "type": "integer" }
                },
                "required": ["target"]
            });
            let schema = match schema {
                serde_json::Value::Object(object) => object,
                _ => unreachable!("schema should be an object"),
            };
            vec![Tool::new(
                "terminal.read",
                "Read visible terminal output",
                schema,
            )]
        }

        fn call_tool(
            &self,
            name: &str,
            arguments: Option<JsonObject>,
            _context: PublicMcpToolContext,
        ) -> Option<PublicMcpToolFuture> {
            if name != "terminal.read" {
                return None;
            }
            let value = serde_json::Value::Object(arguments.unwrap_or_default());
            *self
                .last_arguments
                .lock()
                .expect("arguments lock should not be poisoned") = Some(value);
            Some(Box::pin(async {
                Ok(CallToolResult::success(vec![Content::text("ok")]))
            }))
        }
    }

    fn invocation(arguments: serde_json::Value) -> ToolInvocation {
        invocation_for("sample.echo", arguments)
    }

    fn invocation_for(tool_name: &str, arguments: serde_json::Value) -> ToolInvocation {
        ToolInvocation {
            session_id: SessionId::from_string("session_1"),
            turn_id: TurnId::from_string("turn_1"),
            call_id: ToolCallId::from_string("call_1"),
            tool_name: ToolName::new(tool_name),
            arguments,
            resource_id: None,
            resources: ResourceContext::new(),
            skills: SkillContext::new(),
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
