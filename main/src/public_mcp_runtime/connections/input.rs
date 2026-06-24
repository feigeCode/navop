use serde_json::Value;
use tool_runtime::ToolError;

pub(super) fn required_object<'a>(
    input: &'a Value,
    field: &'static str,
) -> Result<&'a Value, ToolError> {
    input
        .get(field)
        .filter(|value| value.is_object())
        .ok_or_else(|| ToolError::Failed {
            message: format!("missing object field: {field}"),
        })
}

pub(super) fn required_str<'a>(
    input: &'a Value,
    field: &'static str,
) -> Result<&'a str, ToolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Failed {
            message: format!("missing string field: {field}"),
        })
}

pub(super) fn required_value_str<'a>(
    input: &'a Value,
    field: &'static str,
) -> Result<&'a str, ToolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ToolError::Failed {
            message: format!("missing string field: {field}"),
        })
}

pub(super) fn optional_str<'a>(input: &'a Value, field: &str) -> Option<&'a str> {
    input.get(field).and_then(Value::as_str)
}

pub(super) fn optional_value_str<'a>(input: &'a Value, field: &str) -> Option<&'a str> {
    input
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn optional_u16(input: &Value, field: &str) -> Option<u16> {
    optional_u64(input, field).and_then(|value| u16::try_from(value).ok())
}

pub(super) fn optional_u32(input: &Value, field: &str) -> Option<u32> {
    optional_u64(input, field).and_then(|value| u32::try_from(value).ok())
}

pub(super) fn optional_u8(input: &Value, field: &str) -> Option<u8> {
    optional_u64(input, field).and_then(|value| u8::try_from(value).ok())
}

pub(super) fn optional_u64(input: &Value, field: &str) -> Option<u64> {
    input.get(field).and_then(Value::as_u64)
}

pub(super) fn optional_i64(input: &Value, field: &str) -> Option<i64> {
    input.get(field).and_then(Value::as_i64)
}

pub(super) fn optional_bool(input: &Value, field: &str) -> Option<bool> {
    input.get(field).and_then(Value::as_bool)
}

pub(super) fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}
