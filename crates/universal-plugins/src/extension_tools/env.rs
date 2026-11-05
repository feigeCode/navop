//! 扩展工具的参数校验。
//!
//! 安装期（`tools_validation`）已经把 `inputSchema` 限制成 JSON Schema 的
//! 受限子集；这里在执行期按同一份 schema 校验模型给的参数：
//! - `required` 字段必须出现；
//! - 字段类型必须匹配（string/number/integer/boolean/array）；
//! - `enum` 之外的取值拒绝；
//! - 未声明的字段丢弃（debug 日志，不报错——模型啰嗦是常态）；
//! - `default` 不注入（provider 侧自己处理缺省）；
//! - 序列化后 ≤ 64KB。

use serde_json::{Map, Value};

use super::error::invalid_arguments;

/// 参数序列化后的大小上限。
pub const MAX_ARGUMENT_BYTES: usize = 64 * 1024;

/// 按受限 schema 校验并清洗参数。
///
/// 返回只保留已声明字段的参数对象；schema 必须是安装期校验过的
/// `{"type":"object", ...}` 形态（这里不重复做结构校验，防御性兜底）。
pub fn validate_arguments(schema: &Value, arguments: &Value) -> Result<Value, agent_runtime::ToolError> {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        // 没有属性声明：接受任意对象，原样返回。
        return Ok(arguments.clone());
    };
    let Some(args) = arguments.as_object() else {
        return Err(invalid_arguments(format!(
            "参数必须是 JSON 对象，实际是 {}",
            type_name(arguments)
        )));
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut cleaned = Map::new();
    for (name, spec) in properties {
        match args.get(name) {
            None | Some(Value::Null) => {
                if required.contains(&name.as_str()) {
                    return Err(invalid_arguments(format!("缺少必填参数 `{name}`")));
                }
                // default 不注入：provider 自己处理缺省。
            }
            Some(value) => {
                validate_property(name, spec, value)?;
                cleaned.insert(name.clone(), value.clone());
            }
        }
    }

    for name in args.keys() {
        if !properties.contains_key(name) {
            tracing::debug!(target: "extension_tools", field = %name, "丢弃未声明的参数字段");
        }
    }

    let cleaned = Value::Object(cleaned);
    let size = serde_json::to_vec(&cleaned).map_err(|err| {
        invalid_arguments(format!("参数序列化失败：{err}"))
    })?;
    if size.len() > MAX_ARGUMENT_BYTES {
        return Err(invalid_arguments(format!(
            "参数序列化后 {} 字节，超过上限 {} 字节",
            size.len(),
            MAX_ARGUMENT_BYTES
        )));
    }
    Ok(cleaned)
}

fn validate_property(
    name: &str,
    spec: &Value,
    value: &Value,
) -> Result<(), agent_runtime::ToolError> {
    let Some(expected) = spec.get("type").and_then(Value::as_str) else {
        // 安装期保证属性都有 type；这里兜底放行。
        return Ok(());
    };
    let matches = match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        _ => true,
    };
    if !matches {
        return Err(invalid_arguments(format!(
            "参数 `{name}` 需要 {expected}，实际是 {}",
            type_name(value)
        )));
    }
    if let Some(allowed) = spec.get("enum").and_then(Value::as_array) {
        let value_text = value.to_string();
        let ok = allowed.iter().any(|candidate| {
            candidate == value || candidate.to_string() == value_text
        });
        if !ok {
            let allowed_text: Vec<String> =
                allowed.iter().map(|c| c.to_string()).collect();
            return Err(invalid_arguments(format!(
                "参数 `{name}` 的取值 {value_text} 不在枚举 {} 里",
                allowed_text.join("/")
            )));
        }
    }
    if let Some(items) = spec.get("items") {
        if let Some(list) = value.as_array() {
            for item in list {
                validate_property(name, items, item)?;
            }
        }
    }
    Ok(())
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "count": { "type": "integer" },
                "mode": { "type": "string", "enum": ["fast", "slow"] },
                "tags": { "type": "array", "items": { "type": "string" } },
                "ratio": { "type": "number" }
            },
            "required": ["name"]
        })
    }

    #[test]
    fn accepts_valid_arguments_and_drops_unknown_fields() {
        let args = json!({ "name": "web", "count": 3, "extra": "ignored" });
        let cleaned = validate_arguments(&schema(), &args).unwrap();
        assert!(cleaned.get("extra").is_none());
        assert_eq!(cleaned.get("count"), Some(&json!(3)));
    }

    #[test]
    fn missing_required_field_is_rejected() {
        let err = validate_arguments(&schema(), &json!({ "count": 1 })).unwrap_err();
        match err {
            agent_runtime::ToolError::Execution(message) => {
                assert!(message.contains("[EXT_INVALID_ARGUMENTS]"), "{message}");
                assert!(message.contains("name"), "{message}");
            }
            other => panic!("错误类型 {other:?}"),
        }
    }

    #[test]
    fn wrong_type_is_rejected() {
        let err = validate_arguments(&schema(), &json!({ "name": 42 })).unwrap_err();
        let agent_runtime::ToolError::Execution(message) = err else {
            panic!("错误类型错误");
        };
        assert!(message.contains("`name`"), "{message}");
    }

    #[test]
    fn integer_rejects_fractional() {
        let err = validate_arguments(&schema(), &json!({ "name": "x", "count": 1.5 }))
            .unwrap_err();
        assert!(err.to_string().contains("integer"));
    }

    #[test]
    fn enum_value_outside_allowed_is_rejected() {
        let err = validate_arguments(&schema(), &json!({ "name": "x", "mode": "medium" }))
            .unwrap_err();
        assert!(err.to_string().contains("fast"), "{}", err);
    }

    #[test]
    fn array_items_are_checked() {
        let err = validate_arguments(&schema(), &json!({ "name": "x", "tags": ["a", 2] }))
            .unwrap_err();
        assert!(err.to_string().contains("`tags`"), "{}", err);
    }

    #[test]
    fn non_object_arguments_are_rejected() {
        let err = validate_arguments(&schema(), &json!("hello")).unwrap_err();
        assert!(err.to_string().contains("JSON 对象"), "{}", err);
    }

    #[test]
    fn oversized_payload_is_rejected() {
        let long_name = "x".repeat(MAX_ARGUMENT_BYTES);
        let schema = json!({
            "type": "object",
            "properties": { "name": { "type": "string" } }
        });
        let err = validate_arguments(&schema, &json!({ "name": long_name })).unwrap_err();
        assert!(err.to_string().contains("超过上限"), "{}", err);
    }

    #[test]
    fn defaults_are_not_injected() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "default": "fallback" }
            },
            "required": ["name"]
        });
        // 缺 name 时按 required 报错，而不是把 default 填进去。
        let err = validate_arguments(&schema, &json!({})).unwrap_err();
        assert!(err.to_string().contains("必填"), "{}", err);
    }
}
