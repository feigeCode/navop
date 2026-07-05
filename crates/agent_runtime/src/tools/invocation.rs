//! 工具调用上下文。
//!
//! 工具执行时拿到的全部信息。第一版只携带标识、参数、资源上下文与取消令牌;
//! 真实工具(SQL/SSH 等)后续可在此扩展连接句柄等能力(沿用 onetcli 现有的
//! 能力注入模式)。

use crate::error::ToolError;
use crate::ids::{SessionId, ToolCallId, TurnId};
use crate::resource::{ResourceContext, ResourceId, ResourceRef};
use crate::skill::SkillContext;
use crate::tools::spec::ToolName;
use serde::de::DeserializeOwned;
use tokio_util::sync::CancellationToken;

/// 一次工具调用的执行上下文。
pub struct ToolInvocation {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub call_id: ToolCallId,
    pub tool_name: ToolName,
    /// 已解析为 JSON 的调用参数。
    pub arguments: serde_json::Value,
    /// 本次调用显式指定的目标资源(可能为空,表示用当前聚焦资源)。
    pub resource_id: Option<ResourceId>,
    /// 会话级资源上下文。
    pub resources: ResourceContext,
    /// 会话级 Skill 目录与选择状态。
    pub skills: SkillContext,
    /// 取消令牌:整轮被中断时触发,工具应尽快返回。
    pub cancellation: CancellationToken,
}

impl ToolInvocation {
    /// 把参数反序列化为目标类型。
    pub fn parse_arguments<T: DeserializeOwned>(&self) -> Result<T, ToolError> {
        serde_json::from_value(self.arguments.clone())
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))
    }

    /// 取某个字符串字段。
    pub fn arg_str(&self, key: &str) -> Option<&str> {
        self.arguments.get(key).and_then(|v| v.as_str())
    }

    /// 解析本次调用的目标资源:优先用显式 `resource_id`,否则回退到当前聚焦资源。
    pub fn target_resource(&self) -> Option<&ResourceRef> {
        match &self.resource_id {
            Some(id) => self.resources.get(id),
            None => self.resources.current(),
        }
    }

    /// 解析目标资源,缺失时返回 [`ToolError::MissingResource`]。
    pub fn require_resource(&self) -> Result<&ResourceRef, ToolError> {
        self.target_resource().ok_or_else(|| {
            ToolError::MissingResource(format!("工具 {} 找不到可用的目标资源", self.tool_name))
        })
    }
}
