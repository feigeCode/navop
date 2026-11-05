//! 扩展工具的 Public MCP 适配（plugin-ai-tools.md §11，里程碑 M3）。
//!
//! 把 `contributes.tools` 条目包成 [`tool_runtime::ToolHandler`]，经
//! `mcp_host::runtime_provider` 进入 Public MCP 的 tools/list 与 tools/call。
//! 执行链与 Agent 侧（[`crate::extension_tools::tool::ExtensionTool`]）完全
//! 同源：参数校验 → provider 租约 → resource/open → invoke → 解码截断。
//!
//! 与 Agent 侧的唯一差异是连接的解析来源：Agent 侧从对话资源引用
//! （`ResourceRef`）拿当前连接；MCP 侧没有对话上下文，连接型工具从
//! `input.connection`（保存连接的 id 或名字）解析。

use serde_json::{Value, json};

use tool_runtime::{
    ToolAnnotations, ToolContext, ToolDescriptor, ToolHandler, ToolOrigin, ToolResult,
    descriptor::{RiskLevel as RuntimeRiskLevel, ToolAdapter, ToolMode},
    registry::ToolRegistry,
};

use agent_runtime::ToolError;
use extension_host::CancellationToken as HostCancellationToken;
use extension_runtime::{RegisteredShellAppContribution, RegisteredToolContribution};

use crate::extension_tools::connections::ConnectionStore;
use crate::extension_tools::error::{ExtensionToolErrorCode, execution_error};
use crate::extension_tools::result_decode::{self, TruncatedValue};
use crate::universal_plugins::UniversalPluginService;

/// 一个扩展工具的 MCP handler。
pub struct ExtensionToolHandler {
    service: UniversalPluginService,
    connections: Option<ConnectionStore>,
    /// 扩展 shellApp（标题等元数据）。
    app: RegisteredShellAppContribution,
    contribution: RegisteredToolContribution,
    /// MCP 对外 id：与 Agent 侧公开名一致（`ext_<ns>_<tool_id>`）。
    tool_id: String,
}

impl ExtensionToolHandler {
    pub(crate) fn new(
        service: UniversalPluginService,
        connections: Option<ConnectionStore>,
        app: RegisteredShellAppContribution,
        contribution: RegisteredToolContribution,
        tool_id: String,
    ) -> Self {
        Self {
            service,
            connections,
            app,
            contribution,
            tool_id,
        }
    }
}

/// 按 id（数字字符串）或名称查找保存的连接。
fn find_stored_connection(
    store: &ConnectionStore,
    reference: &str,
) -> Result<one_core::storage::StoredConnection, ToolError> {
    // 先按 id 试，再按名字。
    if let Ok(id) = reference.parse::<i64>() {
        if let Some(found) = store.load(id) {
            return Ok(found);
        }
    }
    store.find_by_name(reference).ok_or_else(|| {
        execution_error(
            ExtensionToolErrorCode::MissingConnection,
            format!("找不到连接：{reference}"),
        )
    })
}

impl ToolHandler for ExtensionToolHandler {
    fn descriptor(&self) -> ToolDescriptor {
        let annotations = &self.contribution.annotations;
        let description = format!(
            "{}（扩展「{}」·{}）。{}",
            self.contribution.title,
            self.app.title,
            if annotations.read_only { "只读" } else { "可能修改数据" },
            self.contribution.description,
        );
        // 连接型工具补 connection 参数（受限 schema 子集里没有它）。
        let input_schema = if self.contribution.connection {
            add_connection_parameter(&self.contribution.input_schema)
        } else {
            self.contribution.input_schema.clone()
        };
        ToolDescriptor {
            id: self.tool_id.clone(),
            title: self.contribution.title.clone(),
            description,
            input_schema,
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![ToolAdapter::Mcp, ToolAdapter::FunctionCalling],
            annotations: ToolAnnotations {
                title: self.contribution.title.clone(),
                read_only: annotations.read_only,
                destructive: annotations.destructive,
                idempotent: annotations.idempotent,
                open_world: annotations.open_world,
                supports_parallel: annotations.supports_parallel,
                risk: risk_to_runtime(self.contribution.risk),
            },
        }
    }

    fn origin(&self) -> ToolOrigin {
        ToolOrigin::PublicMcp
    }

    fn call(&self, input: Value, context: ToolContext) -> tool_runtime::ToolFuture {
        let handler = ExtensionToolCall {
            service: self.service.clone(),
            connections: self.connections.clone(),
            contribution: self.contribution.clone(),
        };
        let cancellation = context.cancellation;
        Box::pin(async move {
            let value = handler
                .execute_with(input, cancellation)
                .await
                .map_err(tool_error)?;
            let TruncatedValue { value, truncated } = result_decode::truncate_result(
                &value,
                handler.contribution.max_result_bytes as usize,
            );
            let mut structured = json!({ "result": value });
            if truncated > 0 {
                structured["truncated_bytes"] = json!(truncated);
            }
            Ok(ToolResult::structured(structured))
        })
    }
}

/// `call` 的拥有型载体（handler 本体不能 clone：service 可以，贡献不行）。
struct ExtensionToolCall {
    service: UniversalPluginService,
    connections: Option<ConnectionStore>,
    contribution: RegisteredToolContribution,
}

impl ExtensionToolCall {
    async fn execute_with(
        &self,
        input: Value,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<Value, ToolError> {
        // 与 ExtensionToolHandler::execute 同链；这里直接复用静态执行器。
        let contribution = &self.contribution;
        let arguments = crate::extension_tools::env::validate_arguments(
            &contribution.input_schema,
            &input,
        )?;
        let connection = if contribution.connection {
            let reference = arguments
                .get("connection")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    execution_error(
                        ExtensionToolErrorCode::MissingConnection,
                        "连接型工具需要 connection 参数（保存连接的 id 或名称）",
                    )
                })?;
            Some(self.resolve_connection(&reference)?)
        } else {
            None
        };
        let timeout = std::time::Duration::from_millis(contribution.timeout_ms.min(300_000));
        let cancel = bridge_cancel(&cancellation);
        let result = tokio::time::timeout(
            timeout,
            crate::extension_tools::tool::execute_on_provider(
                &self.service,
                self.connections.as_ref(),
                contribution,
                connection,
                arguments,
                &cancel,
            ),
        )
        .await;
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(error),
            Err(_) => {
                cancellation.cancel();
                cancel.cancel();
                Err(execution_error(
                    ExtensionToolErrorCode::Timeout,
                    format!(
                        "工具 `{}` 执行超过 {} ms",
                        contribution.id, contribution.timeout_ms
                    ),
                ))
            }
        }
    }

    fn resolve_connection(
        &self,
        reference: &str,
    ) -> Result<crate::extension_tools::tool::ResolvedConnection, ToolError> {
        let Some(store) = self.connections.as_ref() else {
            return Err(execution_error(
                ExtensionToolErrorCode::MissingConnection,
                "宿主没有提供连接仓库，无法解析连接",
            ));
        };
        let stored = find_stored_connection(store, reference)?;
        let params = stored.to_extension_params().map_err(|error| {
            execution_error(
                ExtensionToolErrorCode::WrongConnection,
                format!("连接不是扩展连接：{error}"),
            )
        })?;
        if params.extension_id != self.contribution.extension_id {
            return Err(execution_error(
                ExtensionToolErrorCode::WrongConnection,
                format!(
                    "工具属于扩展 `{}`，连接属于扩展 `{}`",
                    self.contribution.extension_id, params.extension_id
                ),
            ));
        }
        Ok(crate::extension_tools::tool::ResolvedConnection {
            connection_id: stored
                .id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            contribution_id: params.contribution_id,
        })
    }
}

/// tokio_util 取消位 → extension_host 取消位桥（MCP 侧取消语义）。
fn bridge_cancel(cancellation: &tokio_util::sync::CancellationToken) -> HostCancellationToken {
    let host_token = HostCancellationToken::new();
    let watcher = host_token.clone();
    let source = cancellation.clone();
    tokio::spawn(async move {
        source.cancelled().await;
        watcher.cancel();
    });
    host_token
}

/// agent 侧 ToolError → tool_runtime 侧 ToolError。
fn tool_error(error: ToolError) -> tool_runtime::ToolError {
    // tool_runtime 的错误面只有 UnknownTool/UnsupportedAdapter/Failed：
    // 全部折成 Failed，保留 "[CODE] 消息" 结构。
    tool_runtime::ToolError::Failed {
        message: error.to_string(),
    }
}

fn risk_to_runtime(risk: extension_runtime::ToolRisk) -> RuntimeRiskLevel {
    match risk {
        extension_runtime::ToolRisk::Read => RuntimeRiskLevel::Read,
        extension_runtime::ToolRisk::Low => RuntimeRiskLevel::Low,
        extension_runtime::ToolRisk::Medium => RuntimeRiskLevel::Medium,
        extension_runtime::ToolRisk::High => RuntimeRiskLevel::High,
        extension_runtime::ToolRisk::Critical => RuntimeRiskLevel::Critical,
    }
}

/// 给受限 schema 补 `connection` 字符串参数（connected 工具在 MCP 面
/// 没有资源引用，必须显式传连接）。
fn add_connection_parameter(schema: &Value) -> Value {
    let mut cloned = schema.clone();
    let Some(object) = cloned.as_object_mut() else {
        return cloned;
    };
    let properties = object
        .entry("properties")
        .or_insert_with(|| json!({}));
    if let Some(props) = properties.as_object_mut() {
        props.insert(
            "connection".to_string(),
            json!({
                "type": "string",
                "description": "保存的扩展连接 id 或名称",
            }),
        );
    }
    if let Some(required) = object.get_mut("required") {
        if let Some(list) = required.as_array_mut() {
            if !list.iter().any(|item| item == "connection") {
                list.push(json!("connection"));
            }
        }
    }
    cloned
}

/// 组装扩展工具的 MCP registry（toolset 工厂用）。
pub fn extension_mcp_tool_registry(
    service: &UniversalPluginService,
    connections: Option<ConnectionStore>,
    disabled_extensions: &std::collections::BTreeSet<String>,
) -> ToolRegistry {
    let catalog = service.catalog_snapshot();
    let mut handlers: Vec<std::sync::Arc<dyn ToolHandler>> = Vec::new();
    for contribution in catalog.tools() {
        if disabled_extensions.contains(&contribution.extension_id) {
            continue;
        }
        let Some(app) = catalog.shell_app(&contribution.extension_id) else {
            tracing::warn!(
                extension = %contribution.extension_id,
                tool = %contribution.id,
                "扩展工具没有 shellApp，跳过 MCP 暴露"
            );
            continue;
        };
        let tool_id =
            crate::extension_tools::public_tool_name(&contribution.extension_id, &contribution.id)
                .as_str()
                .to_string();
        handlers.push(std::sync::Arc::new(ExtensionToolHandler::new(
            service.clone(),
            connections.clone(),
            app.clone(),
            contribution.clone(),
            tool_id,
        )));
    }
    ToolRegistry::new(handlers)
}
