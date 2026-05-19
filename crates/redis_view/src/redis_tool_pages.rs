//! Redis 工具页签的页面级布局:头部摘要在 render 期间生成、表格行数据
//! 在数据/过滤变更时构造,两者共享同一份过滤后的 ToolRow 列表。

use crate::redis_tool_data::{RedisToolKind, ToolRow};
use crate::redis_tool_page_data::{
    commandstats_rows, format_timestamp, keyspace_keys, slowlog_duration_stats, slowlog_entry_rows,
    sorted_monitor_rows,
};
use crate::redis_tool_table::Cells;
use crate::redis_tool_widgets::{
    memory_bars, metric_cards, section_title, stat_string, stat_value, tip_for_key, value,
};
use gpui::{AnyElement, App, IntoElement, ParentElement, SharedString, Styled};
use gpui_component::{IconName, v_flex};

/// 构造页签顶部的摘要区域(metric_cards、memory_bars、section_title 等)。
pub fn build_header(kind: RedisToolKind, rows: &[ToolRow], cx: &App) -> AnyElement {
    match kind {
        RedisToolKind::Info => info_header(rows, cx),
        RedisToolKind::Memory => memory_header(rows, cx),
        RedisToolKind::SlowLog => slowlog_header(rows, cx),
        RedisToolKind::Monitor => monitor_header(rows, cx),
        RedisToolKind::PubSub => pubsub_header(rows, cx),
        RedisToolKind::Chart => v_flex().into_any_element(),
    }
}

/// 把 ToolRow 转换为表格单元格,顺序需匹配 [`RedisToolTableDelegate`] 中对应 kind 的列定义。
pub fn build_table_rows(kind: RedisToolKind, rows: &[ToolRow]) -> Vec<Cells> {
    match kind {
        RedisToolKind::Info => info_rows(rows),
        RedisToolKind::Memory => memory_rows(rows),
        RedisToolKind::SlowLog => slowlog_rows(rows),
        RedisToolKind::Monitor => monitor_rows(rows),
        RedisToolKind::PubSub => pubsub_rows(rows),
        RedisToolKind::Chart => Vec::new(),
    }
}

fn info_header(rows: &[ToolRow], cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .flex_shrink_0()
        .gap_3()
        .child(metric_cards(
            &[
                ("Version", value(rows, "redis_version"), IconName::Info),
                ("Mode", value(rows, "redis_mode"), IconName::Redis),
                ("Role", value(rows, "role"), IconName::Server),
                (
                    "Uptime",
                    format!("{} days", value(rows, "uptime_in_days")),
                    IconName::LoaderCircle,
                ),
                ("Clients", value(rows, "connected_clients"), IconName::User),
                (
                    "Memory",
                    value(rows, "used_memory_human"),
                    IconName::MemoryStick,
                ),
                ("Port", value(rows, "tcp_port"), IconName::Network),
                ("Keys", keyspace_keys(rows), IconName::TableData),
            ],
            cx,
        ))
        .child(section_title("Info Detail", rows.len(), cx))
        .into_any_element()
}

fn info_rows(rows: &[ToolRow]) -> Vec<Cells> {
    rows.iter()
        .map(|row| {
            vec![
                row.category.clone().into(),
                row.key.clone().into(),
                row.value.clone().into(),
                SharedString::from(tip_for_key(&row.key)),
            ]
        })
        .collect()
}

fn memory_header(rows: &[ToolRow], cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .flex_shrink_0()
        .gap_3()
        .child(metric_cards(
            &[
                (
                    "Used",
                    value(rows, "used_memory_human"),
                    IconName::MemoryStick,
                ),
                (
                    "Peak",
                    value(rows, "used_memory_peak_human"),
                    IconName::ChartPie,
                ),
                (
                    "RSS",
                    value(rows, "used_memory_rss_human"),
                    IconName::HardDrive,
                ),
                (
                    "System",
                    value(rows, "total_system_memory_human"),
                    IconName::Monitor,
                ),
                (
                    "Fragmentation",
                    value(rows, "mem_fragmentation_ratio"),
                    IconName::ChartPie,
                ),
                ("Policy", value(rows, "maxmemory_policy"), IconName::Filter),
            ],
            cx,
        ))
        .child(memory_bars(rows, cx))
        .child(section_title("Memory Detail", rows.len(), cx))
        .into_any_element()
}

fn memory_rows(rows: &[ToolRow]) -> Vec<Cells> {
    rows.iter()
        .map(|row| {
            vec![
                row.key.clone().into(),
                row.value.clone().into(),
                SharedString::from(tip_for_key(&row.key)),
            ]
        })
        .collect()
}

fn slowlog_header(rows: &[ToolRow], cx: &App) -> AnyElement {
    let entries = slowlog_entry_rows(rows);
    let (slowest, average) = slowlog_duration_stats(&entries);
    metric_cards(
        &[
            ("Entries", entries.len().to_string(), IconName::LoaderCircle),
            ("Stored", value(rows, "slowlog_len"), IconName::TableData),
            ("Slowest", format!("{slowest} us"), IconName::ChartPie),
            ("Average", format!("{average} us"), IconName::Monitor),
            (
                "Threshold",
                format!("{} us", value(rows, "slowlog-log-slower-than")),
                IconName::Filter,
            ),
            ("Max Len", value(rows, "slowlog-max-len"), IconName::Server),
        ],
        cx,
    )
}

fn slowlog_rows(rows: &[ToolRow]) -> Vec<Cells> {
    slowlog_entry_rows(rows)
        .into_iter()
        .map(|row| {
            let parts = row.value.split('|').collect::<Vec<_>>();
            vec![
                row.category.clone().into(),
                row.key.clone().into(),
                SharedString::from(format_timestamp(parts.first().copied().unwrap_or_default())),
                SharedString::from(format!(
                    "{} us",
                    parts.get(1).copied().unwrap_or_default()
                )),
                SharedString::from(parts.get(2).copied().unwrap_or_default().to_string()),
            ]
        })
        .collect()
}

fn monitor_header(rows: &[ToolRow], cx: &App) -> AnyElement {
    let command_rows = commandstats_rows(rows);
    let total_calls: i64 = command_rows
        .iter()
        .map(|row| stat_value(&row.value, "calls"))
        .sum();
    let sorted = sorted_monitor_rows(rows);
    let top_command = sorted
        .first()
        .map(|row| row.key.trim_start_matches("cmdstat_").to_uppercase())
        .unwrap_or_else(|| "--".to_string());
    metric_cards(
        &[
            (
                "Commands",
                command_rows.len().to_string(),
                IconName::Monitor,
            ),
            ("Total Calls", total_calls.to_string(), IconName::Play),
            (
                "Ops / Sec",
                value(rows, "instantaneous_ops_per_sec"),
                IconName::ChartPie,
            ),
            ("Top Command", top_command, IconName::ChartPie),
            ("Expired", value(rows, "expired_keys"), IconName::Delete),
            (
                "Rejected",
                value(rows, "rejected_connections"),
                IconName::CircleX,
            ),
        ],
        cx,
    )
}

fn monitor_rows(rows: &[ToolRow]) -> Vec<Cells> {
    sorted_monitor_rows(rows)
        .into_iter()
        .map(|row| {
            vec![
                SharedString::from(row.key.trim_start_matches("cmdstat_").to_uppercase()),
                SharedString::from(stat_value(&row.value, "calls").to_string()),
                SharedString::from(stat_value(&row.value, "usec").to_string()),
                SharedString::from(stat_string(&row.value, "usec_per_call")),
                SharedString::from(stat_value(&row.value, "failed_calls").to_string()),
            ]
        })
        .collect()
}

fn pubsub_header(rows: &[ToolRow], cx: &App) -> AnyElement {
    let channel_count = rows.iter().filter(|row| row.category == "channel").count();
    let shard_count = rows
        .iter()
        .filter(|row| row.category == "shard_channel")
        .count();
    let pattern_count = rows
        .iter()
        .find(|row| row.key == "numpat")
        .map(|row| row.value.clone())
        .unwrap_or_default();
    metric_cards(
        &[
            ("Channels", channel_count.to_string(), IconName::Network),
            ("Shard Channels", shard_count.to_string(), IconName::Server),
            ("Patterns", pattern_count, IconName::Filter),
        ],
        cx,
    )
}

fn pubsub_rows(rows: &[ToolRow]) -> Vec<Cells> {
    rows.iter()
        .filter(|row| row.category != "pattern")
        .map(|row| {
            vec![
                row.category.clone().into(),
                row.key.clone().into(),
                row.value.clone().into(),
            ]
        })
        .collect()
}
