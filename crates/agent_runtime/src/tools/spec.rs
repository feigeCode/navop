//! 工具规格(对模型暴露的元信息)。

use crate::risk::RiskLevel;
use llm_connector::types::Tool as LlmTool;
use serde::{Deserialize, Serialize};
use std::fmt;

/// 工具名称。作为注册表键,也对应模型 function-calling 中的函数名。
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolName(String);

impl ToolName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ToolName({})", self.0)
    }
}

impl From<&str> for ToolName {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for ToolName {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// 工具规格:名称、描述、参数 JSON Schema 以及风险等级。
///
/// 通过 [`ToolSpec::to_llm_tool`] 转换为 `llm-connector` 的工具定义,直接交给
/// 模型用于 function-calling。
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: ToolName,
    pub description: String,
    /// 参数的 JSON Schema(object 类型)。
    pub parameters: serde_json::Value,
    pub risk: RiskLevel,
}

impl ToolSpec {
    pub fn new(
        name: impl Into<ToolName>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            risk: RiskLevel::Read,
        }
    }

    pub fn with_risk(mut self, risk: RiskLevel) -> Self {
        self.risk = risk;
        self
    }

    /// 转换为 `llm-connector` 工具定义。
    pub fn to_llm_tool(&self) -> LlmTool {
        LlmTool::function(
            self.name.as_str(),
            Some(self.description.clone()),
            self.parameters.clone(),
        )
    }
}
