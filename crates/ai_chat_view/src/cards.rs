//! 内置示例卡片。
//!
//! 这里只放**非业务**的演示卡片，用来证明卡片机制可用、可扩展。真正的业务卡片
//! (SQL 结果、监控图表等)应由各业务模块在自己的 crate 内实现 [`ChatCard`] 并
//! 通过 [`CardRegistry::register_global`] 注册。

use crate::card::{CardMessage, CardRegistry, ChatCard};
use crate::theme::active_agent_chat_theme;
use crate::{ChartJsonBlock, ChartType, parse_chart_json_block};
use gpui::prelude::FluentBuilder;
use gpui::{AnyElement, App, FontWeight, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::{h_flex, v_flex};
use rust_i18n::t;
use std::sync::Arc;

/// 示例卡片：把消息内容当作 JSON 美化后逐行展示。
///
/// `kind = "json"`。内容不是合法 JSON 时原样展示。
pub struct JsonCard;

impl ChatCard for JsonCard {
    fn kind(&self) -> &'static str {
        "json"
    }

    fn render(&self, msg: &CardMessage, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = active_agent_chat_theme(cx);
        let pretty = serde_json::from_str::<serde_json::Value>(msg.content)
            .ok()
            .and_then(|value| serde_json::to_string_pretty(&value).ok())
            .unwrap_or_else(|| msg.content.to_string());

        let lines: Vec<AnyElement> = pretty
            .lines()
            .map(|line| {
                div()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(line.to_string())
                    .into_any_element()
            })
            .collect();

        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.panel)
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("AgentUi.json_card_example").to_string()),
            )
            .child(v_flex().w_full().min_w_0().children(lines))
            .into_any_element()
    }
}

/// 内置 chart-json 卡片：把 AI 生成的图表 JSON 渲染为轻量条形/列表视图。
///
/// `kind = "chart-json"`。完整交互图表可由业务模块用同 kind 覆盖注册。
pub struct ChartJsonCard;

impl ChatCard for ChartJsonCard {
    fn kind(&self) -> &'static str {
        "chart-json"
    }

    fn render(&self, msg: &CardMessage, _window: &mut Window, cx: &mut App) -> AnyElement {
        let Some(chart) = parse_chart_json_block(msg.content, Some("chart-json")) else {
            return invalid_chart_card(msg.content, cx);
        };
        let theme = active_agent_chat_theme(cx);
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(render_chart_header(&chart, cx))
            .child(render_chart_body(&chart, cx))
            .into_any_element()
    }
}

/// 注册所有内置示例卡片到全局注册表。
pub fn register_builtin_cards(cx: &mut App) {
    CardRegistry::register_global(cx, Arc::new(JsonCard));
    CardRegistry::register_global(cx, Arc::new(ChartJsonCard));
}

fn render_chart_header(chart: &ChartJsonBlock, cx: &mut App) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    let title = chart.title.as_deref().unwrap_or(match chart.chart_type {
        ChartType::Line => "Line Chart",
        ChartType::Bar => "Bar Chart",
        ChartType::Pie => "Pie Chart",
    });
    v_flex()
        .gap_1()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_string()),
        )
        .when_some(chart.description.clone(), |this, description| {
            this.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(description),
            )
        })
        .into_any_element()
}

fn render_chart_body(chart: &ChartJsonBlock, cx: &mut App) -> AnyElement {
    match chart.chart_type {
        ChartType::Line | ChartType::Bar => render_xy_points(chart, cx),
        ChartType::Pie => render_pie_points(chart, cx),
    }
}

fn render_xy_points(chart: &ChartJsonBlock, cx: &mut App) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    let points = chart.to_xy_points();
    let max_y = points.iter().map(|point| point.y).fold(0.0_f64, f64::max);
    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .children(points.into_iter().map(|point| {
            let width = scaled_width(point.y, max_y);
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(88.0))
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(point.x),
                )
                .child(div().h(px(10.0)).w(px(width)).rounded_sm().bg(theme.accent))
                .child(div().text_xs().child(format_number(point.y)))
                .into_any_element()
        }))
        .into_any_element()
}

fn render_pie_points(chart: &ChartJsonBlock, cx: &mut App) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .children(chart.to_pie_points().into_iter().map(|point| {
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(point.category),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.foreground)
                        .child(format_number(point.value)),
                )
                .into_any_element()
        }))
        .into_any_element()
}

fn invalid_chart_card(content: &str, cx: &mut App) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(theme.border)
        .bg(theme.panel)
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("AgentUi.invalid_chart_card").to_string()),
        )
        .child(div().text_sm().child(content.to_string()))
        .into_any_element()
}

fn scaled_width(value: f64, max: f64) -> f32 {
    if max <= 0.0 {
        return 12.0;
    }
    ((value / max).clamp(0.05, 1.0) * 180.0) as f32
}

fn format_number(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_card_kinds_are_stable() {
        assert_eq!("json", JsonCard.kind());
        assert_eq!("chart-json", ChartJsonCard.kind());
    }
}
