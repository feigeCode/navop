use serde_json::Value;
use tool_runtime::ToolError;

use super::schema::MAX_DB_INDEX;

pub(super) fn required_str(input: &Value, field: &'static str) -> Result<String, ToolError> {
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
            .filter(|value| *value <= MAX_DB_INDEX)
            .map(|value| value as u8)
            .map(Some)
            .ok_or_else(|| ToolError::Failed {
                message: format!("field `{field}` must be an integer from 0 to {MAX_DB_INDEX}"),
            }),
    }
}

pub(super) fn parse_command_args(command: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            current.push(escaped_char(ch, in_single || in_double));
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            value if value.is_whitespace() && !in_single && !in_double => {
                push_arg(&mut args, &mut current);
            }
            value => current.push(value),
        }
    }
    push_arg(&mut args, &mut current);
    args
}

fn escaped_char(ch: char, in_quotes: bool) -> char {
    match (in_quotes, ch) {
        (true, 'n') => '\n',
        (true, 'r') => '\r',
        (true, 't') => '\t',
        _ => ch,
    }
}

fn push_arg(args: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        args.push(std::mem::take(current));
    }
}
