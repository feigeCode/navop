//! Redis 工具页签页面统计辅助。

use crate::redis_tool_data::ToolRow;
use crate::redis_tool_widgets::{stat_string, stat_value};
use chrono::{Local, TimeZone};

pub(crate) fn slowlog_entry_rows(rows: &[ToolRow]) -> Vec<&ToolRow> {
    rows.iter()
        .filter(|row| row.category != "summary" && row.category != "config")
        .collect()
}

pub(crate) fn keyspace_keys(rows: &[ToolRow]) -> String {
    rows.iter()
        .filter(|row| row.category == "Keyspace")
        .filter_map(|row| stat_string(&row.value, "keys").parse::<i64>().ok())
        .sum::<i64>()
        .to_string()
}

pub(crate) fn slowlog_duration_stats(rows: &[&ToolRow]) -> (i64, i64) {
    let durations = rows
        .iter()
        .filter_map(|row| row.value.split('|').nth(1)?.parse::<i64>().ok())
        .collect::<Vec<_>>();
    let slowest = durations.iter().copied().max().unwrap_or(0);
    let average = if durations.is_empty() {
        0
    } else {
        durations.iter().sum::<i64>() / durations.len() as i64
    };
    (slowest, average)
}

pub(crate) fn format_timestamp(raw: &str) -> String {
    let Ok(timestamp) = raw.parse::<i64>() else {
        return raw.to_string();
    };
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| raw.to_string())
}

pub(crate) fn sorted_monitor_rows(rows: &[ToolRow]) -> Vec<&ToolRow> {
    let mut rows = commandstats_rows(rows);
    rows.sort_by_key(|row| std::cmp::Reverse(stat_value(&row.value, "calls")));
    rows
}

pub(crate) fn commandstats_rows(rows: &[ToolRow]) -> Vec<&ToolRow> {
    rows.iter()
        .filter(|row| row.category == "Commandstats")
        .collect()
}
