//! 连接级 AI 访问策略。
//!
//! 用户可以在 Navop 里把某个连接标记为「仅手动管理」，从而禁止 AI 在该连接上执行
//! 任何工具调用（issue #437）。本模块只提供与存储无关的策略语义：
//!
//! - [`AiAccessLevel`]：访问级别，以及它对工具注解、资源能力的影响；
//! - [`ToolAccessGuard`]：由上层（`navop_app`）实现的执行期校验钩子；
//! - [`narrow_capabilities`]：把访问级别换算成 AI 可见/可用的资源能力。
//!
//! 真正“某条连接是哪个级别”由存储层决定，本 crate 不感知连接存储。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ResourceCapability, RuntimeToolDescriptor, ToolAnnotations, ToolError};

/// 某条连接对 AI 的开放级别。
///
/// 存储层使用 [`AiAccessLevel::as_str`] 落库；老数据（列/键缺失）按
/// [`AiAccessLevel::Allowed`] 处理，保持升级前的行为不变。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiAccessLevel {
    /// 默认：AI 可以像以前一样使用该连接。
    #[default]
    Allowed,
    /// 只读：AI 只能使用只读工具（查询、读取、列举），写操作一律拒绝。
    ReadOnly,
    /// 禁止：AI 完全不能通过该连接执行任何操作。
    Denied,
}

impl AiAccessLevel {
    /// 全部级别，顺序按「由宽到严」，供设置界面枚举。
    pub const ALL: [AiAccessLevel; 3] = [
        AiAccessLevel::Allowed,
        AiAccessLevel::ReadOnly,
        AiAccessLevel::Denied,
    ];

    /// 落库/上报用的稳定字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            AiAccessLevel::Allowed => "allowed",
            AiAccessLevel::ReadOnly => "read_only",
            AiAccessLevel::Denied => "denied",
        }
    }

    /// 严格解析，未知取值返回 `None`。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "allowed" => Some(AiAccessLevel::Allowed),
            "read_only" => Some(AiAccessLevel::ReadOnly),
            "denied" => Some(AiAccessLevel::Denied),
            _ => None,
        }
    }

    /// 从存储值解析。
    ///
    /// - `None` / 空串：老数据，等同 [`AiAccessLevel::Allowed`]；
    /// - 未知取值：按最严格的 [`AiAccessLevel::Denied`] 处理（fail closed），
    ///   避免降级客户端把新版本设置的限制静默放开。
    pub fn from_storage(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            None | Some("") => AiAccessLevel::Allowed,
            Some(raw) => Self::parse(raw).unwrap_or(AiAccessLevel::Denied),
        }
    }

    /// 是否保持默认值，用于 `serde(skip_serializing_if)`，避免旧载荷被迫升级。
    pub fn is_allowed(self) -> bool {
        matches!(self, AiAccessLevel::Allowed)
    }

    /// 是否放行某个工具调用（依据工具自身的只读注解）。
    pub fn permits(self, annotations: &ToolAnnotations) -> bool {
        match self {
            AiAccessLevel::Allowed => true,
            AiAccessLevel::ReadOnly => annotations.read_only,
            AiAccessLevel::Denied => false,
        }
    }

    /// 按级别裁剪资源能力，供资源池/资源目录收窄 AI 可见范围。
    pub fn narrow_capabilities(
        self,
        capabilities: &[ResourceCapability],
    ) -> Vec<ResourceCapability> {
        narrow_capabilities(self, capabilities)
    }
}

/// 只读级别下仍然放行的能力：查询、列举、读文件。
///
/// 这里刻意采用白名单而不是“排除写能力”的黑名单：新增的写类能力默认被拒，
/// 不会因为忘记登记而漏放。
/// 一条受 AI 访问策略限制的连接（供拦截器按名字或 id 匹配）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestrictedConnection {
    /// 连接在本地数据库中的 id。
    pub connection_id: i64,
    /// 连接显示名，用于匹配工具入参里的连接名。
    pub name: String,
    /// 该连接被限制到的级别（只读或禁止）。
    pub level: AiAccessLevel,
}

pub const READ_ONLY_CAPABILITIES: [ResourceCapability; 4] = [
    ResourceCapability::Query,
    ResourceCapability::DatabaseQuery,
    ResourceCapability::List,
    ResourceCapability::ReadFile,
];

/// 按访问级别裁剪资源能力。
pub fn narrow_capabilities(
    level: AiAccessLevel,
    capabilities: &[ResourceCapability],
) -> Vec<ResourceCapability> {
    match level {
        AiAccessLevel::Allowed => capabilities.to_vec(),
        AiAccessLevel::ReadOnly => capabilities
            .iter()
            .filter(|capability| READ_ONLY_CAPABILITIES.contains(capability))
            .cloned()
            .collect(),
        AiAccessLevel::Denied => Vec::new(),
    }
}

/// 执行期校验钩子。
///
/// 在参数归一化（`target` 已解析回 `connection` / `session_id`）之后、工具真正执行
/// 之前调用。实现方负责把参数里的连接标识映射到策略级别。
pub trait ToolAccessGuard: Send + Sync + 'static {
    fn authorize(&self, tool: &RuntimeToolDescriptor, input: &Value) -> Result<(), ToolError>;
}

/// 策略拒绝时的统一错误。
pub fn access_denied_error(tool_id: &str, level: AiAccessLevel, target: Option<&str>) -> ToolError {
    let action = match level {
        AiAccessLevel::ReadOnly => "只读",
        _ => "禁止 AI 操作",
    };
    let target = target
        .map(|target| format!(" `{target}`"))
        .unwrap_or_default();
    ToolError::Failed {
        message: format!(
            "连接{target}已被用户设置为「{action}」，工具 `{tool_id}` 被拒绝执行。\
             请由用户在 Navop 界面手动完成该操作，或请用户调整该连接的 AI 访问设置；\
             不要尝试通过其他连接或直接执行命令绕过限制。"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn read_only_annotations() -> ToolAnnotations {
        ToolAnnotations::read_only("query")
    }

    fn mutating_annotations() -> ToolAnnotations {
        ToolAnnotations::mutating("execute")
    }

    #[test]
    fn level_strings_round_trip() {
        for level in AiAccessLevel::ALL {
            assert_eq!(AiAccessLevel::parse(level.as_str()), Some(level));
        }
        assert_eq!(AiAccessLevel::parse("forbidden"), None);
        assert_eq!(AiAccessLevel::parse(""), None);
    }

    #[test]
    fn legacy_and_unknown_storage_values() {
        assert_eq!(AiAccessLevel::from_storage(None), AiAccessLevel::Allowed);
        assert_eq!(
            AiAccessLevel::from_storage(Some("")),
            AiAccessLevel::Allowed
        );
        assert_eq!(
            AiAccessLevel::from_storage(Some("  ")),
            AiAccessLevel::Allowed
        );
        assert_eq!(
            AiAccessLevel::from_storage(Some("denied")),
            AiAccessLevel::Denied
        );
        assert_eq!(
            AiAccessLevel::from_storage(Some("read_only")),
            AiAccessLevel::ReadOnly
        );
        // 未知取值必须 fail closed，防止旧客户端把新版本的更严格设置放开。
        assert_eq!(
            AiAccessLevel::from_storage(Some("future_level")),
            AiAccessLevel::Denied
        );
    }

    #[test]
    fn serde_uses_stable_snake_case_strings() {
        assert_eq!(
            serde_json::to_string(&AiAccessLevel::ReadOnly).unwrap(),
            "\"read_only\""
        );
        assert_eq!(
            serde_json::from_str::<AiAccessLevel>("\"denied\"").unwrap(),
            AiAccessLevel::Denied
        );
        assert!(AiAccessLevel::Allowed.is_allowed());
        assert!(!AiAccessLevel::Denied.is_allowed());
    }

    #[test]
    fn permits_follows_read_only_annotation() {
        assert!(AiAccessLevel::Allowed.permits(&read_only_annotations()));
        assert!(AiAccessLevel::Allowed.permits(&mutating_annotations()));

        assert!(AiAccessLevel::ReadOnly.permits(&read_only_annotations()));
        assert!(!AiAccessLevel::ReadOnly.permits(&mutating_annotations()));

        // 禁止级别连只读工具也不放行：连不可达的连接都不该让 AI 触碰。
        assert!(!AiAccessLevel::Denied.permits(&read_only_annotations()));
        assert!(!AiAccessLevel::Denied.permits(&mutating_annotations()));
    }

    #[test]
    fn narrow_capabilities_keeps_only_read_only_whitelist() {
        let all = [
            ResourceCapability::DatabaseQuery,
            ResourceCapability::DatabaseExecute,
            ResourceCapability::Query,
            ResourceCapability::Execute,
            ResourceCapability::List,
            ResourceCapability::ReadFile,
            ResourceCapability::WriteFile,
            ResourceCapability::ExecCommand,
            ResourceCapability::RemoteExec,
            ResourceCapability::TerminalExec,
            ResourceCapability::TerminalControl,
            ResourceCapability::TerminalInput,
            ResourceCapability::ManageConnection,
            ResourceCapability::OpenSession,
        ];

        assert_eq!(
            AiAccessLevel::Allowed.narrow_capabilities(&all),
            all.to_vec()
        );

        let read_only = AiAccessLevel::ReadOnly.narrow_capabilities(&all);
        assert_eq!(
            read_only,
            vec![
                ResourceCapability::DatabaseQuery,
                ResourceCapability::Query,
                ResourceCapability::List,
                ResourceCapability::ReadFile,
            ]
        );

        assert!(
            AiAccessLevel::Denied.narrow_capabilities(&all).is_empty(),
            "禁止级别不能保留任何能力"
        );
    }

    // ---- ToolRegistry 集成：guard 必须在 handler 执行前介入 ----

    use crate::{
        ToolAdapter, ToolContext, ToolDescriptor, ToolFuture, ToolHandler, ToolMode, ToolRegistry,
        ToolResult,
    };
    use serde_json::json;

    struct FixedGuard {
        level: AiAccessLevel,
    }

    impl ToolAccessGuard for FixedGuard {
        fn authorize(&self, tool: &RuntimeToolDescriptor, _input: &Value) -> Result<(), ToolError> {
            if self.level.permits(&tool.annotations) {
                Ok(())
            } else {
                Err(access_denied_error(tool.id.as_str(), self.level, None))
            }
        }
    }

    struct EchoHandler {
        descriptor: ToolDescriptor,
    }

    impl EchoHandler {
        fn new(id: &str, read_only: bool) -> Self {
            let annotations = if read_only {
                ToolAnnotations::read_only(id)
            } else {
                ToolAnnotations::mutating(id)
            };
            Self {
                descriptor: ToolDescriptor {
                    id: id.to_string(),
                    title: id.to_string(),
                    description: String::new(),
                    input_schema: json!({"type": "object"}),
                    output_schema: Value::Null,
                    permissions: Vec::new(),
                    mode: ToolMode::Deterministic,
                    adapters: vec![ToolAdapter::FunctionCalling],
                    annotations,
                },
            }
        }
    }

    impl ToolHandler for EchoHandler {
        fn descriptor(&self) -> ToolDescriptor {
            self.descriptor.clone()
        }

        fn call(&self, input: Value, _context: ToolContext) -> ToolFuture {
            Box::pin(async move { Ok(ToolResult::structured(input)) })
        }
    }

    fn registry_with(guard: Option<AiAccessLevel>) -> ToolRegistry {
        let registry = ToolRegistry::new(vec![
            Arc::new(EchoHandler::new("database.query", true)),
            Arc::new(EchoHandler::new("database.execute", false)),
        ]);
        match guard {
            Some(level) => registry.with_access_guard(Arc::new(FixedGuard { level })),
            None => registry,
        }
    }

    fn call(registry: &ToolRegistry, id: &str) -> Result<ToolResult, ToolError> {
        futures::executor::block_on(registry.call(
            id,
            json!({}),
            ToolContext::for_adapter(ToolAdapter::FunctionCalling),
        ))
    }

    #[test]
    fn registry_without_guard_stays_permissive() {
        let registry = registry_with(None);
        assert!(!registry.has_access_guard());
        assert!(call(&registry, "database.execute").is_ok());
    }

    #[test]
    fn registry_guard_blocks_denied_tools_before_execution() {
        let registry = registry_with(Some(AiAccessLevel::Denied));
        assert!(registry.has_access_guard());
        let error = call(&registry, "database.query").unwrap_err();
        assert!(error.to_string().contains("禁止 AI 操作"));
        assert!(call(&registry, "database.execute").is_err());
    }

    #[test]
    fn registry_guard_allows_read_only_tools_on_read_only_level() {
        let registry = registry_with(Some(AiAccessLevel::ReadOnly));
        assert!(call(&registry, "database.query").is_ok());
        let error = call(&registry, "database.execute").unwrap_err();
        assert!(error.to_string().contains("只读"));
    }

    #[test]
    fn registry_guard_allows_everything_by_default() {
        let registry = registry_with(Some(AiAccessLevel::Allowed));
        assert!(call(&registry, "database.query").is_ok());
        assert!(call(&registry, "database.execute").is_ok());
    }

    #[test]
    fn deny_error_is_actionable_and_mentions_tool() {
        let error = access_denied_error("database.execute", AiAccessLevel::Denied, Some("测试库"));
        let message = error.to_string();
        assert!(message.contains("database.execute"));
        assert!(message.contains("禁止 AI 操作"));
        assert!(message.contains("测试库"));
        assert!(message.contains("手动"));
    }
}
