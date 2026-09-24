//! 工具规格(对模型暴露的元信息)。

use crate::risk::RiskLevel;
use llm_connector::types::Tool as LlmTool;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;

/// 工具名称。作为注册表键,也对应模型 function-calling 中的函数名。
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

/// 为一组原始工具名分配唯一、function-calling 安全的公开名称。
///
/// 多个 provider 工具可能在规范化后得到同一个名字，例如 `sample.echo` 与
/// `sample_echo` 都会变为 `sample_echo`。分配器保留第一个名称，后续冲突项按
/// `_2`、`_3` 递增，并检查最终候选本身是否已被其它原始工具占用。
#[derive(Default)]
pub struct ToolNameAllocator {
    used: HashSet<ToolName>,
}

impl ToolNameAllocator {
    pub fn allocate(&mut self, original: impl AsRef<str>) -> ToolName {
        let base = ToolName::new(original.as_ref());
        if self.used.insert(base.clone()) {
            return base;
        }

        let mut suffix = 2_u64;
        loop {
            let candidate = ToolName::new(format!("{}_{suffix}", base.as_str()));
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
            suffix = suffix
                .checked_add(1)
                .expect("tool name collision suffix overflow");
        }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolSchemaError {
    tool_name: String,
    pointer: String,
    message: String,
}

impl ToolSchemaError {
    fn new(tool_name: &ToolName, pointer: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.as_str().to_string(),
            pointer: pointer.into(),
            message: message.into(),
        }
    }

    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    pub fn pointer(&self) -> &str {
        &self.pointer
    }
}

impl fmt::Display for ToolSchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "tool `{}` has an incompatible function-calling schema at {}: {}",
            self.tool_name, self.pointer, self.message
        )
    }
}

impl std::error::Error for ToolSchemaError {}

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

    pub fn from_runtime_descriptor(descriptor: &tool_runtime::RuntimeToolDescriptor) -> Self {
        Self {
            name: ToolName::new(descriptor.id.as_str()),
            description: descriptor.description.clone(),
            parameters: descriptor.input_schema.clone(),
            risk: runtime_risk_to_agent(descriptor.annotations.risk),
        }
    }

    /// 验证参数 schema 后转换为 `llm-connector` 工具定义。
    pub fn to_llm_tool(&self) -> Result<LlmTool, ToolSchemaError> {
        validate_function_calling_schema(&self.name, &self.parameters)?;
        Ok(LlmTool::function(
            self.name.as_str(),
            Some(self.description.clone()),
            self.parameters.clone(),
        ))
    }
}

/// provider 不允许出现在工具参数 schema 顶层的关键字。
///
/// OpenAI 等 provider 要求顶层是纯 `object` schema，遇到这些关键字会直接拒绝整个
/// 模型请求（例如 `Invalid request: OpenAI: Invalid schema for function
/// \`connections_save\` ...`），且报错只提函数名，难以定位到具体工具。这里在本地
/// 就拦截，并把工具名与指针写进错误。嵌套层（如 `properties.*` 内部）不受限制。
const REJECTED_ROOT_KEYWORDS: [&str; 6] = ["oneOf", "anyOf", "allOf", "enum", "const", "not"];

fn validate_function_calling_schema(
    tool_name: &ToolName,
    schema: &serde_json::Value,
) -> Result<(), ToolSchemaError> {
    let Some(root) = schema.as_object() else {
        return Err(ToolSchemaError::new(
            tool_name,
            "/",
            "root schema must be a JSON object declaring type \"object\"",
        ));
    };
    if root.contains_key("$ref") && !root.contains_key("type") {
        return Err(ToolSchemaError::new(
            tool_name,
            "/$ref",
            "a root reference cannot replace the required object schema",
        ));
    }
    if root.get("type").and_then(serde_json::Value::as_str) != Some("object") {
        return Err(ToolSchemaError::new(
            tool_name,
            "/type",
            "root schema must declare type \"object\"",
        ));
    }
    for keyword in REJECTED_ROOT_KEYWORDS {
        if root.contains_key(keyword) {
            return Err(ToolSchemaError::new(
                tool_name,
                format!("/{keyword}"),
                format!(
                    "`{keyword}` is not allowed in the root schema; express the constraint with properties, required, enum-like descriptions, and runtime validation instead"
                ),
            ));
        }
    }
    Ok(())
}

fn runtime_risk_to_agent(risk: tool_runtime::RiskLevel) -> RiskLevel {
    match risk {
        tool_runtime::RiskLevel::Read => RiskLevel::Read,
        tool_runtime::RiskLevel::Low => RiskLevel::Low,
        tool_runtime::RiskLevel::Medium => RiskLevel::Medium,
        tool_runtime::RiskLevel::High => RiskLevel::High,
        tool_runtime::RiskLevel::Critical => RiskLevel::Critical,
    }
}

#[cfg(test)]
mod tests {
    use super::{ToolName, ToolNameAllocator, ToolSchemaError, ToolSpec};
    use serde_json::{Value, json};

    fn spec(parameters: Value) -> ToolSpec {
        ToolSpec::new("runtime.tool", "Runtime tool", parameters)
    }

    fn schema_error(parameters: Value) -> ToolSchemaError {
        match spec(parameters).to_llm_tool() {
            Ok(_) => panic!("schema should be rejected"),
            Err(error) => error,
        }
    }

    #[test]
    fn tool_name_is_openai_function_name_safe() {
        assert_eq!(ToolName::new("sample.echo").as_str(), "sample_echo");
        assert_eq!(ToolName::new("执行 SQL").as_str(), "SQL");
        assert_eq!(ToolName::new("4.query").as_str(), "tool_4_query");
        assert_eq!(ToolName::new("___").as_str(), "tool");
    }

    #[test]
    fn tool_name_allocator_makes_sanitized_collisions_unique() {
        let mut allocator = ToolNameAllocator::default();

        assert_eq!(allocator.allocate("sample.echo").as_str(), "sample_echo");
        assert_eq!(allocator.allocate("sample_echo").as_str(), "sample_echo_2");
        assert_eq!(allocator.allocate("sample/echo").as_str(), "sample_echo_3");
    }

    #[test]
    fn tool_name_allocator_skips_already_reserved_suffixes() {
        let mut allocator = ToolNameAllocator::default();

        assert_eq!(
            allocator.allocate("sample_echo_2").as_str(),
            "sample_echo_2"
        );
        assert_eq!(allocator.allocate("sample.echo").as_str(), "sample_echo");
        assert_eq!(allocator.allocate("sample_echo").as_str(), "sample_echo_3");
    }

    #[test]
    fn tool_name_allocator_handles_empty_and_invalid_names() {
        let mut allocator = ToolNameAllocator::default();

        assert_eq!(allocator.allocate("").as_str(), "tool");
        assert_eq!(allocator.allocate("___").as_str(), "tool_2");
        assert_eq!(allocator.allocate("执行").as_str(), "tool_3");
    }

    #[test]
    fn tool_name_allocator_is_deterministic_for_the_same_order() {
        let originals = ["sample.echo", "sample_echo", "sample/echo", "sample_echo_2"];

        let allocate = || {
            let mut allocator = ToolNameAllocator::default();
            originals
                .iter()
                .map(|name| allocator.allocate(name).to_string())
                .collect::<Vec<_>>()
        };

        assert_eq!(allocate(), allocate());
    }

    #[test]
    fn function_calling_accepts_object_schemas_and_nullable_properties() {
        let result = spec(json!({
            "type": "object",
            "properties": {
                "query": { "type": ["string", "null"] }
            }
        }))
        .to_llm_tool();

        assert!(result.is_ok());
    }

    #[test]
    fn function_calling_rejects_boolean_and_non_object_root_schemas() {
        for schema in [
            json!(true),
            json!([]),
            json!({ "type": "string" }),
            json!({ "type": ["object", "null"] }),
        ] {
            let error = schema_error(schema).to_string();
            assert!(error.contains("runtime_tool"));
            assert!(error.contains("type \"object\""));
        }
    }

    #[test]
    fn function_calling_rejects_missing_root_type_and_root_ref() {
        let missing_type = schema_error(json!({ "properties": {} })).to_string();
        assert!(missing_type.contains("/type"));

        let root_ref = schema_error(json!({
            "$ref": "#/$defs/Arguments",
            "$defs": {
                "Arguments": { "type": "object", "properties": {} }
            }
        }))
        .to_string();
        assert!(root_ref.contains("runtime_tool"));
        assert!(root_ref.contains("/$ref"));
        assert!(root_ref.contains("reference"));
    }

    #[test]
    fn function_calling_rejects_provider_unsupported_root_keywords() {
        for keyword in ["oneOf", "anyOf", "allOf", "enum", "const", "not"] {
            let error = schema_error(json!({
                "type": "object",
                "properties": { "value": { "type": "string" } },
                keyword: if keyword == "not" {
                    json!({ "required": ["value"] })
                } else {
                    json!([{ "type": "object" }])
                }
            }));

            assert_eq!(format!("/{keyword}"), error.pointer());
            assert!(error.to_string().contains("runtime_tool"));
            assert!(error.to_string().contains("root schema"));
        }
    }

    #[test]
    fn function_calling_accepts_nullable_types_and_nested_combinators() {
        let result = spec(json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string" },
                "target": {
                    "anyOf": [
                        { "type": "string" },
                        { "type": ["integer", "null"] }
                    ]
                }
            },
            "required": ["kind"]
        }))
        .to_llm_tool();

        assert!(result.is_ok());
    }
}
