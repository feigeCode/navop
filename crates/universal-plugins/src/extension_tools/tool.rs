//! 扩展工具的执行器。
//!
//! 一个 `ExtensionTool` 对应清单里的一个 `contributes.tools[]` 条目，
//! 实现 [`agent_runtime::Tool`]。执行链（设计文档 §8）：
//! 参数校验 → 连接型工具解析资源引用 → 激活 provider 租约 →
//! `resource/open` → `resource/invoke` → 结果解码与截断 → 释放。

use std::time::Duration;

use agent_runtime::ToolError;
use agent_runtime::tools::{ObservationData, ToolInvocation, ToolName, ToolObservation};
use agent_runtime::{RiskLevel, Tool as AgentTool, ToolSpec};
use extension_host::{CancellationToken, RequestOptions};
use extension_protocol::resource::{ResourceInvokeParams, ResourceOpenParams};
use extension_protocol::result_ref::ResultRef;
use extension_runtime::{RegisteredToolContribution, ToolRisk};

use crate::extension_tools::connections::{ConnectionStore, load_stored_connection};
use crate::extension_tools::error::{
    ExtensionToolErrorCode, execution_error, provider_error,
};
use crate::extension_tools::result_decode::{self, TruncatedValue};
use crate::universal_plugins::UniversalPluginService;

/// 一个扩展工具的执行单元。
pub struct ExtensionTool {
    service: UniversalPluginService,
    /// 连接仓库（连接型工具装载 StoredConnection 用）。
    connections: Option<ConnectionStore>,
    contribution: RegisteredToolContribution,
    public_name: ToolName,
}

impl ExtensionTool {
    pub(crate) fn new(
        service: UniversalPluginService,
        connections: Option<ConnectionStore>,
        contribution: RegisteredToolContribution,
        public_name: ToolName,
    ) -> Self {
        Self {
            service,
            connections,
            contribution,
            public_name,
        }
    }

    /// 工具的公开名（`ext_<ns>__<id>`）。
    pub fn public_name(&self) -> &ToolName {
        &self.public_name
    }

    /// 注册来源（扩展 id + 工具 id）。
    pub fn origin(&self) -> (&str, &str) {
        (&self.contribution.extension_id, &self.contribution.id)
    }
}

fn risk_level(risk: ToolRisk) -> RiskLevel {
    match risk {
        ToolRisk::Read => RiskLevel::Read,
        ToolRisk::Low => RiskLevel::Low,
        ToolRisk::Medium => RiskLevel::Medium,
        ToolRisk::High => RiskLevel::High,
        ToolRisk::Critical => RiskLevel::Critical,
    }
}

/// spec 描述：`{title}（扩展「{ext_title}」·只读）。{description}`。
///
/// 只读与否看 annotations.readOnly（与审批用的 risk 分开）。
fn spec_description(title: &str, ext_title: &str, readonly: bool, description: &str) -> String {
    let modifier = if readonly { "只读" } else { "可能修改数据" };
    format!("{title}（扩展「{ext_title}」·{modifier}）。{description}")
}

#[async_trait::async_trait]
impl AgentTool for ExtensionTool {
    fn name(&self) -> ToolName {
        self.public_name.clone()
    }

    fn supports_parallel(&self) -> bool {
        self.contribution.annotations.supports_parallel
    }

    fn spec(&self, _resources: &agent_runtime::ResourceContext) -> ToolSpec {
        ToolSpec {
            name: self.public_name.clone(),
            description: spec_description(
                &self.contribution.title,
                &self.contribution.extension_id,
                self.contribution.annotations.read_only,
                &self.contribution.description,
            ),
            parameters: self.contribution.input_schema.clone(),
            risk: risk_level(self.contribution.risk),
        }
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        let contribution = &self.contribution;
        let arguments = crate::extension_tools::env::validate_arguments(
            &contribution.input_schema,
            &invocation.arguments,
        )?;

        // 连接型工具：拿当前资源引用校验归属。
        let connection = if contribution.connection {
            Some(resolve_connection_resource(&invocation, &contribution.extension_id)?)
        } else {
            None
        };

        let timeout = Duration::from_millis(contribution.timeout_ms.min(300_000));
        let cancel = host_cancel(&invocation);
        let result = tokio::time::timeout(
            timeout,
            execute_on_provider(
                &self.service,
                self.connections.as_ref(),
                contribution,
                connection,
                arguments,
                &cancel,
            ),
        )
        .await;

        let value = match result {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                cancel.cancel();
                return Err(execution_error(
                    ExtensionToolErrorCode::Timeout,
                    format!(
                        "工具 `{}` 执行超过 {} ms",
                        contribution.id, contribution.timeout_ms
                    ),
                ));
            }
        };

        let TruncatedValue { value, truncated } =
            result_decode::truncate_result(&value, contribution.max_result_bytes as usize);
        let mut summary = result_decode::summarize(&value);
        if truncated > 0 {
            summary.push_str(&format!(" …[truncated {truncated} bytes]"));
        }
        let mut observation = ToolObservation::success(
            invocation.call_id.clone(),
            self.public_name.clone(),
            summary,
            ObservationData::Json(value),
        );
        observation.arguments = Some(invocation.arguments.clone());
        if let Some(resource) = invocation.target_resource() {
            observation.resource_id = Some(resource.id.clone());
        }
        Ok(observation)
    }
}

/// agent_runtime 的 tokio_util token → extension_host token 桥。
///
/// 后者是宿主传输层的取消语义；前者由对话层控制。桥接方式：host token
/// 由监听任务触发，agent 侧取消一到就把它取消。
fn host_cancel(invocation: &ToolInvocation) -> CancellationToken {
    let agent_token = invocation.cancellation.clone();
    let host_token = CancellationToken::new();
    let watcher = host_token.clone();
    tokio::spawn(async move {
        agent_token.cancelled().await;
        watcher.cancel();
    });
    host_token
}

/// 连接型工具的资源校验：ResourceRef 必须指向本扩展声明的连接贡献。
fn resolve_connection_resource(
    invocation: &ToolInvocation,
    extension_id: &str,
) -> Result<ResolvedConnection, ToolError> {
    let resource = invocation.require_resource()?;
    // ai_chat_view 的 resource_builder 为扩展连接生成的 kind 是
    // `extension:{extension_id}:{contribution_id}`。
    let kind = resource.kind.as_str();
    let Some(rest) = kind.strip_prefix("extension:") else {
        return Err(execution_error(
            ExtensionToolErrorCode::WrongConnection,
            format!("当前资源不是扩展连接（kind = {kind}）"),
        ));
    };
    let Some((ref_extension, contribution_id)) = rest.split_once(':') else {
        return Err(execution_error(
            ExtensionToolErrorCode::WrongConnection,
            format!("扩展连接的 kind 格式不正确：{kind}"),
        ));
    };
    if ref_extension != extension_id {
        return Err(execution_error(
            ExtensionToolErrorCode::WrongConnection,
            format!(
                "工具属于扩展 `{extension_id}`，当前资源属于扩展 `{ref_extension}`"
            ),
        ));
    }
    Ok(ResolvedConnection {
        connection_id: resource.id.as_str().to_string(),
        contribution_id: contribution_id.to_string(),
    })
}

pub(crate) struct ResolvedConnection {
    pub(crate) connection_id: String,
    pub(crate) contribution_id: String,
}

pub(crate) async fn execute_on_provider(
    service: &UniversalPluginService,
    connections: Option<&ConnectionStore>,
    contribution: &RegisteredToolContribution,
    connection: Option<ResolvedConnection>,
    arguments: serde_json::Value,
    cancel: &CancellationToken,
) -> Result<serde_json::Value, ToolError> {
    // 1. provider 租约。
    let activation = service
        .activate_runtime(&contribution.runtime_key)
        .await
        .map_err(|error| {
            execution_error(
                ExtensionToolErrorCode::ExtensionUnavailable,
                format!("扩展 provider 无法启动：{error}"),
            )
        })?;

    let result =
        invoke_with_lease(service, connections, contribution, &connection, arguments, cancel).await;

    // 无论成败都释放租约。
    let _ = service.deactivate_activation(&activation).await;
    result
}

async fn invoke_with_lease(
    service: &UniversalPluginService,
    connections: Option<&ConnectionStore>,
    contribution: &RegisteredToolContribution,
    connection: &Option<ResolvedConnection>,
    arguments: serde_json::Value,
    cancel: &CancellationToken,
) -> Result<serde_json::Value, ToolError> {
    // 2. resource/open。
    let client = service
        .universal_plugin_client(&contribution.runtime_key)
        .map_err(|error| {
            execution_error(
                ExtensionToolErrorCode::ExtensionUnavailable,
                format!("扩展 provider 连接失败：{error}"),
            )
        })?;

    let open_params = match connection {
        Some(resolved) => {
            let Some(store) = connections else {
                return Err(execution_error(
                    ExtensionToolErrorCode::MissingConnection,
                    "宿主没有提供连接仓库，无法打开扩展连接",
                ));
            };
            // 从宿主侧连接仓库取 StoredConnection 组装启动参数。
            let stored = load_stored_connection(store, &resolved.connection_id).map_err(
                |message| {
                    execution_error(ExtensionToolErrorCode::MissingConnection, message)
                },
            )?;
            let contribution_ref = service
                .resource_connection(&contribution.extension_id, &resolved.contribution_id)
                .ok_or_else(|| {
                    execution_error(
                        ExtensionToolErrorCode::WrongConnection,
                        format!(
                            "扩展 `{}` 没有连接贡献 `{}`",
                            contribution.extension_id, resolved.contribution_id
                        ),
                    )
                })?;
            crate::extension_resource::launch_open_params(&stored, &contribution_ref.clone())
        }
        None => ResourceOpenParams {
            resource_type: tool_resource_type(&contribution.extension_id, &contribution.id),
            config: serde_json::json!({}),
            metadata: None,
        },
    };

    let host_client = client.client();
    let opened = host_client
        .open_resource_with_options(&open_params, RequestOptions::default().with_cancel(cancel.clone()))
        .await
        .map_err(|error| provider_error(format!("resource/open 失败：{error}")))?;
    let resource_handle = opened.resource_id;

    // 3. resource/invoke。
    let invoke_result = host_client
        .invoke_resource_with_options(
            &ResourceInvokeParams {
                resource_id: resource_handle.clone(),
                method: contribution.method.clone(),
                params: arguments,
            },
            RequestOptions::default().with_cancel(cancel.clone()),
        )
        .await;

    // 4. 无论结果如何都先关资源。
    let _ = host_client
        .close_resource(&extension_protocol::resource::ResourceCloseParams {
            resource_id: resource_handle,
        })
        .await;

    let invoked = invoke_result.map_err(|error| {
        if cancel.is_cancelled() {
            execution_error(ExtensionToolErrorCode::Cancelled, "工具执行已被取消")
        } else {
            provider_error(format!("resource/invoke 失败：{error}"))
        }
    })?;

    // 5. ResultRef 解码。
    match invoked.result {
        ResultRef::Inline { value } => Ok(value),
        ResultRef::Blob { id } => result_decode::decode_blob(host_client, &id, cancel).await,
        ResultRef::EventStream { .. } => Err(execution_error(
            ExtensionToolErrorCode::UnsupportedResult,
            "工具方法返回了事件流，v1 不支持",
        )),
    }
}

/// 无连接工具的 resource_type：`{extension}::{tool}`——provider 侧按
/// 约定识别「宿主以工具身份 open」。
fn tool_resource_type(extension_id: &str, tool_id: &str) -> String {
    format!("{extension_id}::{tool_id}")
}

impl std::fmt::Debug for ExtensionTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionTool")
            .field("extension", &self.contribution.extension_id)
            .field("tool", &self.contribution.id)
            .field("public_name", &self.public_name.as_str().to_string())
            .finish()
    }
}

// —— 供 registry 与测试复用的内部工具 ——

/// 公开名组成：`ext_<ns>__<规范化 id>`。
///
/// `ToolName::new` 会把 `.` 规范化成 `_`，并把连续 `_` 合并——`ext_`、
/// `ns` 与 id 之间的双下划线分隔符会被压成一个。为了保留视觉层次，
/// ns 与 id 之间统一用双下划线拼接后再交给规范化。
pub fn public_tool_name(extension_id: &str, tool_id: &str) -> ToolName {
    let ns = extension_id.rsplit('.').next().unwrap_or(extension_id);
    // ToolName::new 合并连续下划线，因此双下划线分隔不可保留；
    // 用 ns_id 单下划线形式（`ext_docker_containers_list`）。
    ToolName::new(format!("ext_{ns}_{tool_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_name_uses_last_extension_segment() {
        let name = public_tool_name("com.navop.docker", "containers.list");
        assert_eq!(name.as_str(), "ext_docker_containers_list");
    }

    #[test]
    fn public_name_without_dots_uses_whole_id() {
        let name = public_tool_name("docker", "stop");
        assert_eq!(name.as_str(), "ext_docker_stop");
    }

    #[test]
    fn spec_description_marks_readonly() {
        let text = spec_description("列出容器", "Docker", true, "返回容器列表。");
        assert!(text.contains("只读"), "{text}");
        assert!(text.contains("扩展「Docker」"), "{text}");
    }

    #[test]
    fn spec_description_marks_mutation() {
        let text = spec_description("停止容器", "Docker", false, "停止一个容器。");
        assert!(text.contains("可能修改数据"), "{text}");
    }

    #[test]
    fn risk_mapping_is_identity_like() {
        assert_eq!(risk_level(ToolRisk::Read), RiskLevel::Read);
        assert_eq!(risk_level(ToolRisk::Critical), RiskLevel::Critical);
    }

    #[test]
    fn tool_resource_type_combines_extension_and_tool() {
        assert_eq!(
            tool_resource_type("com.navop.docker", "containers.list"),
            "com.navop.docker::containers.list"
        );
    }
}
