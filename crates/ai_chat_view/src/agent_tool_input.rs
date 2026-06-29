//! 工具入参展示:生成卡片头部摘要与展开区 JSON,并递归脱敏敏感字段。

use serde_json::{Map, Value};

const MAX_SUMMARY_CHARS: usize = 160;
const MAX_JSON_CHARS: usize = 4000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ToolInputDisplay {
    pub summary: String,
    pub json: String,
}

pub(crate) fn build_tool_input_display(tool_name: &str, arguments: &Value) -> ToolInputDisplay {
    let redacted = redact_value(arguments);
    if input_is_empty(&redacted) {
        return ToolInputDisplay::default();
    }
    let summary = summarize_tool_input(tool_name, &redacted);
    ToolInputDisplay {
        summary: truncate_chars(&summary, MAX_SUMMARY_CHARS),
        json: truncate_chars(&pretty_json(&redacted), MAX_JSON_CHARS),
    }
}

fn summarize_tool_input(tool_name: &str, arguments: &Value) -> String {
    let tool = tool_name.to_lowercase();
    let key_groups: &[&[&str]] = if tool.contains("sql") || tool.contains("query") {
        &[&["sql", "query"], &["command", "cmd", "shell_command"]]
    } else {
        &[
            &["command", "cmd", "shell_command"],
            &["sql", "query"],
            &["path", "file_path", "target_path"],
            &["method", "url"],
            &["url"],
            &["text", "prompt", "message"],
        ]
    };
    for keys in key_groups {
        if let Some(summary) = summarize_keys(arguments, keys) {
            return summary;
        }
    }
    compact_json(arguments)
}

fn summarize_keys(arguments: &Value, keys: &[&str]) -> Option<String> {
    let object = arguments.as_object()?;
    if keys == ["method", "url"] {
        return Some(format!(
            "{} {}",
            field_summary(object, "method")?,
            field_summary(object, "url")?
        ));
    }
    keys.iter().find_map(|key| field_summary(object, key))
}

fn field_summary(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .or_else(|| {
            object
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v)
        })
        .map(value_summary)
        .filter(|value| !value.trim().is_empty())
}

fn value_summary(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_string(),
        _ => compact_json(value),
    }
}

fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(redact_object(object)),
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        _ => value.clone(),
    }
}

fn redact_object(object: &Map<String, Value>) -> Map<String, Value> {
    object
        .iter()
        .map(|(key, value)| {
            let redacted = if is_sensitive_key(key) {
                Value::String("***".to_string())
            } else {
                redact_value(value)
            };
            (key.clone(), redacted)
        })
        .collect()
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    [
        "password",
        "passwd",
        "token",
        "apikey",
        "secret",
        "authorization",
        "cookie",
        "privatekey",
        "accesskey",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn input_is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Object(object) => object.is_empty(),
        _ => false,
    }
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| compact_json(value))
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn command_summary_uses_command_field() {
        let display = build_tool_input_display(
            "exec_command",
            &json!({"command": "rtk cargo check -p main", "timeout": 30}),
        );

        assert_eq!("rtk cargo check -p main", display.summary);
        assert!(display.json.contains("rtk cargo check -p main"));
    }

    #[test]
    fn sql_summary_prefers_sql_for_sql_tools() {
        let display = build_tool_input_display(
            "SQL",
            &json!({"query": "select * from users", "command": "ignored"}),
        );

        assert_eq!("select * from users", display.summary);
    }

    #[test]
    fn sensitive_values_are_redacted_recursively() {
        let display = build_tool_input_display(
            "http",
            &json!({
                "url": "https://example.test",
                "headers": {"Authorization": "Bearer abc", "cookie": "sid=1"},
                "password": "plain"
            }),
        );

        assert!(display.json.contains("\"Authorization\": \"***\""));
        assert!(display.json.contains("\"cookie\": \"***\""));
        assert!(display.json.contains("\"password\": \"***\""));
        assert!(!display.json.contains("Bearer abc"));
        assert!(!display.json.contains("sid=1"));
        assert!(!display.json.contains("plain"));
    }

    #[test]
    fn empty_arguments_do_not_render_input() {
        assert_eq!(
            ToolInputDisplay::default(),
            build_tool_input_display("echo", &json!({}))
        );
        assert_eq!(
            ToolInputDisplay::default(),
            build_tool_input_display("echo", &Value::Null)
        );
    }
}
