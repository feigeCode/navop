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
        Self(sanitize_tool_name(&name.into()))
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
        Self::new(value)
    }
}

impl From<String> for ToolName {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

fn sanitize_tool_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len().max(1));
    let mut last_was_underscore = false;
    for ch in name.chars() {
        let valid = ch.is_ascii_alphanumeric() || ch == '_' || ch == '-';
        let next = if valid { ch } else { '_' };
        if next == '_' && last_was_underscore {
            continue;
        }
        last_was_underscore = next == '_';
        out.push(next);
    }
    let trimmed = out.trim_matches('_').to_string();
    let mut normalized = if trimmed.is_empty() {
        "tool".to_string()
    } else {
        trimmed
    };
    if !normalized
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
    {
        normalized = format!("tool_{normalized}");
    }
    normalized
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

#[cfg(test)]
mod tests {
    use super::ToolName;

    #[test]
    fn tool_name_is_openai_function_name_safe() {
        assert_eq!(ToolName::new("sample.echo").as_str(), "sample_echo");
        assert_eq!(ToolName::new("执行 SQL").as_str(), "SQL");
        assert_eq!(ToolName::new("4.query").as_str(), "tool_4_query");
        assert_eq!(ToolName::new("___").as_str(), "tool");
    }
}
