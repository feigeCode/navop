//! Redis 工具页签复用的轻量布局组件:统计卡片、内存条与少量字符串辅助函数。
//! 表格已迁移至 [`crate::redis_tool_table`] 共享的 gpui-component Table。

use crate::redis_tool_data::ToolRow;
use gpui::{AnyElement, App, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, Size, h_flex, v_flex};

pub fn metric_cards(items: &[(&str, String, IconName)], cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .flex_shrink_0()
        .flex_wrap()
        .gap_3()
        .children(
            items
                .iter()
                .map(|(label, value, icon)| metric_card(label, value, icon.clone(), cx)),
        )
        .into_any_element()
}

pub fn section_title(title: &str, count: usize, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_base()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{count} rows")),
        )
        .into_any_element()
}

pub fn memory_bars(rows: &[ToolRow], cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .flex_shrink_0()
        .gap_2()
        .children([
            memory_bar(
                "Used",
                numeric(rows, "used_memory"),
                numeric(rows, "used_memory_peak").max(1.0),
                cx,
            ),
            memory_bar(
                "RSS",
                numeric(rows, "used_memory_rss"),
                numeric(rows, "total_system_memory").max(1.0),
                cx,
            ),
        ])
        .into_any_element()
}

pub fn value(rows: &[ToolRow], key: &str) -> String {
    rows.iter()
        .find(|row| row.key == key)
        .map(|row| row.value.clone())
        .unwrap_or_else(|| "--".into())
}

pub fn numeric(rows: &[ToolRow], key: &str) -> f64 {
    value(rows, key).parse().unwrap_or(0.0)
}

pub fn stat_value(raw: &str, key: &str) -> i64 {
    stat_string(raw, key).parse().unwrap_or(0)
}

pub fn stat_string(raw: &str, key: &str) -> String {
    raw.split(',')
        .filter_map(|part| part.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
        .unwrap_or_default()
}

pub fn tip_for_key(key: &str) -> &'static str {
    match key {
        "redis_version" => "Redis server version",
        "redis_mode" => "Server deployment mode",
        "used_memory" => "Memory allocated by Redis",
        "connected_clients" => "Active client connections",
        "instantaneous_ops_per_sec" => "Commands processed per second",
        _ => "",
    }
}

fn metric_card(label: &str, value: &str, icon: IconName, cx: &App) -> AnyElement {
    h_flex()
        .w(px(220.0))
        .h(px(76.0))
        .gap_3()
        .items_center()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(6.0))
        .bg(cx.theme().list_hover.opacity(0.35))
        .child(
            Icon::new(icon)
                .with_size(Size::Medium)
                .text_color(cx.theme().primary),
        )
        .child(
            v_flex()
                .gap_1()
                .min_w_0()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_lg()
                        .font_weight(gpui::FontWeight::BOLD)
                        .truncate()
                        .child(value.to_string()),
                ),
        )
        .into_any_element()
}

fn memory_bar(label: &str, value: f64, max: f64, cx: &App) -> AnyElement {
    let ratio = (value / max).clamp(0.02, 1.0) as f32;
    v_flex()
        .gap_1()
        .child(
            h_flex()
                .justify_between()
                .child(label.to_string())
                .child(format_bytes(value)),
        )
        .child(
            div()
                .h(px(10.0))
                .w_full()
                .rounded(px(5.0))
                .bg(cx.theme().muted)
                .child(
                    div()
                        .h_full()
                        .w(gpui::relative(ratio))
                        .rounded(px(5.0))
                        .bg(cx.theme().primary),
                ),
        )
        .into_any_element()
}

fn format_bytes(value: f64) -> String {
    if value >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2}G", value / 1024.0 / 1024.0 / 1024.0)
    } else if value >= 1024.0 * 1024.0 {
        format!("{:.2}M", value / 1024.0 / 1024.0)
    } else if value >= 1024.0 {
        format!("{:.2}K", value / 1024.0)
    } else {
        format!("{value:.0}B")
    }
}
