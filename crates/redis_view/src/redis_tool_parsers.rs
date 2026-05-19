//! Redis 工具页签原始响应解析。

use crate::RedisValue;
use crate::redis_tool_data::ToolRow;

pub(crate) fn rows_from_slowlog_value(value: RedisValue) -> Vec<ToolRow> {
    let RedisValue::Bulk(entries) = value else {
        return rows_from_value("slowlog", value);
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let RedisValue::Bulk(fields) = entry else {
                return None;
            };
            let id = fields.first().map(value_text).unwrap_or_default();
            let timestamp = fields.get(1).map(value_text).unwrap_or_default();
            let duration = fields.get(2).map(value_text).unwrap_or_default();
            let command = fields.get(3).map(command_text).unwrap_or_default();
            let client = fields.get(4).map(value_text).unwrap_or_default();
            Some(ToolRow {
                category: id,
                key: command,
                value: format!("{timestamp}|{duration}|{client}"),
            })
        })
        .collect()
}

pub(crate) fn rows_from_slowlog_len_value(value: RedisValue) -> Vec<ToolRow> {
    vec![ToolRow {
        category: "summary".to_string(),
        key: "slowlog_len".to_string(),
        value: value_text(&value),
    }]
}

pub(crate) fn rows_from_slowlog_config_value(value: RedisValue) -> Vec<ToolRow> {
    let RedisValue::Bulk(items) = value else {
        return Vec::new();
    };
    items
        .chunks(2)
        .filter_map(|chunk| {
            Some(ToolRow {
                category: "config".to_string(),
                key: value_text(chunk.first()?),
                value: value_text(chunk.get(1)?),
            })
        })
        .collect()
}

pub(crate) fn rows_from_pubsub_values(
    channels: RedisValue,
    subscribers: RedisValue,
    patterns: RedisValue,
    shard_channels: RedisValue,
    shard_subscribers: RedisValue,
) -> Vec<ToolRow> {
    let mut rows = channel_rows("channel", &channels, subscriber_counts(subscribers));
    rows.push(ToolRow {
        category: "pattern".to_string(),
        key: "numpat".to_string(),
        value: value_text(&patterns),
    });
    rows.extend(channel_rows(
        "shard_channel",
        &shard_channels,
        subscriber_counts(shard_subscribers),
    ));
    rows
}

pub(crate) fn quote_command_arg(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_:.".contains(ch))
    {
        return value.to_string();
    }
    // 加双引号包裹,并把反斜杠 / 双引号 / 常见控制字符转义,
    // 保证命令字符串单行可读,且与 parse_command_args 的反向解析保持往返一致。
    let mut buf = String::with_capacity(value.len() + 2);
    buf.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => buf.push_str("\\\\"),
            '"' => buf.push_str("\\\""),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            other => buf.push(other),
        }
    }
    buf.push('"');
    buf
}

pub(crate) fn build_publish_command(channel: &str, message: &str) -> String {
    format!(
        "PUBLISH {} {}",
        quote_command_arg(channel),
        quote_command_arg(message)
    )
}

pub(crate) fn slowlog_reset_command() -> &'static str {
    "SLOWLOG RESET"
}

pub(crate) fn value_text(value: &RedisValue) -> String {
    match value {
        RedisValue::Nil => String::new(),
        RedisValue::String(value) | RedisValue::Status(value) | RedisValue::Error(value) => {
            value.clone()
        }
        RedisValue::Integer(value) => value.to_string(),
        RedisValue::Float(value) => value.to_string(),
        RedisValue::Binary(value) => String::from_utf8_lossy(value).to_string(),
        RedisValue::Bulk(items) => items.iter().map(value_text).collect::<Vec<_>>().join(" "),
    }
}

fn channel_rows(
    category: &str,
    channels: &RedisValue,
    counts: Vec<(String, String)>,
) -> Vec<ToolRow> {
    string_list(channels)
        .into_iter()
        .map(|channel| ToolRow {
            category: category.to_string(),
            key: channel.clone(),
            value: counts
                .iter()
                .find(|(name, _)| name == &channel)
                .map(|(_, count)| count.clone())
                .unwrap_or_else(|| "0".to_string()),
        })
        .collect()
}

fn rows_from_value(category: &str, value: RedisValue) -> Vec<ToolRow> {
    match value {
        RedisValue::Bulk(items) => items
            .into_iter()
            .enumerate()
            .map(|(index, item)| ToolRow {
                category: category.to_string(),
                key: format!("#{}", index + 1),
                value: item.to_display_string(),
            })
            .collect(),
        other => vec![ToolRow {
            category: category.to_string(),
            key: "result".to_string(),
            value: other.to_display_string(),
        }],
    }
}

fn subscriber_counts(value: RedisValue) -> Vec<(String, String)> {
    let RedisValue::Bulk(items) = value else {
        return Vec::new();
    };
    items
        .chunks(2)
        .filter_map(|chunk| Some((value_text(chunk.first()?), value_text(chunk.get(1)?))))
        .collect()
}

fn string_list(value: &RedisValue) -> Vec<String> {
    let RedisValue::Bulk(items) = value else {
        return Vec::new();
    };
    items.iter().map(value_text).collect()
}

fn command_text(value: &RedisValue) -> String {
    match value {
        RedisValue::Bulk(items) => items.iter().map(value_text).collect::<Vec<_>>().join(" "),
        other => value_text(other),
    }
}
