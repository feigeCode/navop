//! 内置工具:回显工具,用于联通性测试与端到端验证。

use crate::error::ToolError;
use crate::resource::ResourceContext;
use crate::tools::invocation::ToolInvocation;
use crate::tools::observation::{ObservationData, ToolObservation};
use crate::tools::registry::Tool;
use crate::tools::spec::{ToolName, ToolSpec};
use async_trait::async_trait;
use serde_json::json;

/// 回显工具:把入参 `message` 原样作为观测返回。
///
/// 没有任何副作用,不依赖资源,便于在没有真实连接时打通整条 Runtime 闭环。
pub struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> ToolName {
        ToolName::new("echo")
    }

    fn spec(&self, _resources: &ResourceContext) -> ToolSpec {
        ToolSpec::new(
            "echo",
            "回显给定的 message 文本,用于联通性测试。不产生任何副作用。",
            json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "要回显的文本"
                    }
                },
                "required": ["message"]
            }),
        )
    }

    async fn execute(&self, invocation: ToolInvocation) -> Result<ToolObservation, ToolError> {
        let message = invocation
            .arg_str("message")
            .ok_or_else(|| ToolError::InvalidArguments("缺少字符串字段 message".to_string()))?
            .to_string();

        Ok(ToolObservation::success(
            invocation.call_id,
            invocation.tool_name,
            format!("echo: {message}"),
            ObservationData::Text(message),
        ))
    }
}
