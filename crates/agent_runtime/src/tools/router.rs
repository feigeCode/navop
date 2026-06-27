//! 工具路由器:把一次工具调用分发到注册表中的具体工具。

use crate::error::ToolError;
use crate::ids::{SessionId, ToolCallId, TurnId};
use crate::resource::{ResourceContext, ResourceId};
use crate::tools::invocation::ToolInvocation;
use crate::tools::observation::ToolObservation;
use crate::tools::registry::ToolRegistry;
use crate::tools::spec::{ToolName, ToolSpec};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

/// 一次待执行的工具调用(由 Planner / 模型产出)。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: ToolCallId,
    pub tool_name: ToolName,
    /// 已解析为 JSON 的参数。
    pub arguments: serde_json::Value,
    /// 显式指定的目标资源(可选)。
    pub resource_id: Option<ResourceId>,
}

impl ToolCall {
    pub fn new(tool_name: impl Into<ToolName>, arguments: serde_json::Value) -> Self {
        Self {
            call_id: ToolCallId::new(),
            tool_name: tool_name.into(),
            arguments,
            resource_id: None,
        }
    }

    pub fn with_call_id(mut self, call_id: ToolCallId) -> Self {
        self.call_id = call_id;
        self
    }

    pub fn with_resource(mut self, resource_id: Option<ResourceId>) -> Self {
        self.resource_id = resource_id;
        self
    }

    /// 从模型返回的 `llm-connector` 工具调用构造。
    ///
    /// `function.arguments` 是 JSON 字符串,空串视为空对象。
    pub fn from_llm(tc: &llm_connector::types::ToolCall) -> Result<Self, ToolError> {
        let args_str = tc.function.arguments.trim();
        let arguments = if args_str.is_empty() {
            serde_json::Value::Object(Default::default())
        } else {
            serde_json::from_str(args_str)
                .map_err(|e| ToolError::InvalidArguments(format!("工具参数不是合法 JSON: {e}")))?
        };
        let call_id = if tc.id.is_empty() {
            ToolCallId::new()
        } else {
            ToolCallId::from_string(tc.id.clone())
        };
        Ok(Self {
            call_id,
            tool_name: ToolName::new(tc.function.name.clone()),
            arguments,
            resource_id: None,
        })
    }
}

/// 分发工具调用所需的会话级上下文。
#[derive(Clone)]
pub struct ToolDispatchContext {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub resources: ResourceContext,
}

/// 工具路由器。
pub struct ToolRouter {
    registry: ToolRegistry,
}

impl ToolRouter {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }

    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// 当前资源上下文下所有工具的规格。
    pub fn specs(&self, resources: &ResourceContext) -> Vec<ToolSpec> {
        self.registry.specs(resources)
    }

    pub fn supports_parallel(&self, call: &ToolCall) -> bool {
        self.registry.supports_parallel(&call.tool_name)
    }

    /// 分发并执行一次工具调用。
    ///
    /// **总是返回 [`ToolObservation`]**:工具不存在、参数错误、执行失败或被取消,
    /// 都会转换成 `success = false` 的观测,从而保证失败也能写回历史并反馈给模型。
    pub async fn dispatch(
        &self,
        ctx: &ToolDispatchContext,
        call: ToolCall,
        cancellation: CancellationToken,
    ) -> ToolObservation {
        let started_at = Utc::now();
        let call_id = call.call_id.clone();
        let tool_name = call.tool_name.clone();
        let resource_id = call.resource_id.clone();

        let Some(tool) = self.registry.get(&tool_name) else {
            let mut obs = ToolObservation::from_error(
                call_id,
                tool_name.clone(),
                &ToolError::NotFound(tool_name.to_string()),
            )
            .with_resource(resource_id);
            obs.started_at = started_at;
            obs.finished_at = Utc::now();
            return obs;
        };

        let invocation = ToolInvocation {
            session_id: ctx.session_id.clone(),
            turn_id: ctx.turn_id.clone(),
            call_id: call_id.clone(),
            tool_name: tool_name.clone(),
            arguments: call.arguments,
            resource_id: resource_id.clone(),
            resources: ctx.resources.clone(),
            cancellation: cancellation.clone(),
        };

        // 在工具执行与取消之间竞争,提供分发边界上的取消能力。
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(ToolError::Cancelled),
            r = tool.execute(invocation) => r,
        };

        let mut obs = match result {
            Ok(obs) => obs,
            Err(err) => ToolObservation::from_error(call_id, tool_name, &err),
        };
        // 路由器统一覆盖资源与时间戳,工具实现无需关心。
        if obs.resource_id.is_none() {
            obs.resource_id = resource_id;
        }
        obs.started_at = started_at;
        obs.finished_at = Utc::now();
        obs
    }
}
