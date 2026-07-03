use serde_json::Value;
use tool_runtime::ToolError;

pub(super) fn required_string(input: &Value, field: &'static str) -> Result<String, ToolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| ToolError::Failed {
            message: format!("missing required string field `{field}`"),
        })
}

pub(super) fn optional_u8(input: &Value, field: &'static str) -> Result<Option<u8>, ToolError> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|value| u8::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| ToolError::Failed {
                message: format!("field `{field}` must be an integer from 0 to 255"),
            }),
    }
}

pub(super) fn redis_arg(value: &str) -> String {
    if value
        .chars()
        .any(|ch| ch.is_whitespace() || ch == '"' || ch == '\\')
    {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.to_string()
    }
}
