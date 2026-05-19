//! Redis 图表标签页渲染。

use crate::redis_tool_data::ToolRow;
use crate::redis_tool_widgets::{metric_cards, value};
use chrono::{Duration, Local};
use gpui::{
    App, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, linear_color_stop, linear_gradient, px,
};
use gpui_component::{
    ActiveTheme, IconName,
    chart::{AreaChart, LineChart},
    h_flex, v_flex,
};

#[derive(Clone)]
pub(crate) struct ChartPoint {
    pub(crate) label: SharedString,
    pub(crate) commands: f64,
    pub(crate) memory: f64,
    pub(crate) input: f64,
    pub(crate) output: f64,
    pub(crate) network_ceiling: f64,
}

#[derive(IntoElement)]
pub struct RedisChartView {
    samples: Vec<Vec<ToolRow>>,
}

impl RedisChartView {
    pub fn new(samples: &[Vec<ToolRow>]) -> Self {
        Self {
            samples: samples.to_vec(),
        }
    }
}

impl RenderOnce for RedisChartView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let points = build_points_from_samples(&self.samples);
        let latest = self.samples.last().cloned().unwrap_or_default();
        v_flex()
            .size_full()
            .id("redis-chart-scroll")
            .gap_5()
            .p_4()
            .overflow_y_scroll()
            .child(metric_cards(
                &[
                    (
                        "Ops / Sec",
                        value(&latest, "instantaneous_ops_per_sec"),
                        IconName::Play,
                    ),
                    (
                        "Memory",
                        value(&latest, "used_memory_human"),
                        IconName::MemoryStick,
                    ),
                    (
                        "Clients",
                        value(&latest, "connected_clients"),
                        IconName::User,
                    ),
                    (
                        "DB Keys",
                        value(&latest, "selected_db_keys"),
                        IconName::TableData,
                    ),
                    (
                        "Input Kb/s",
                        value(&latest, "instantaneous_input_kbps"),
                        IconName::Network,
                    ),
                    (
                        "Output Kb/s",
                        value(&latest, "instantaneous_output_kbps"),
                        IconName::Upload,
                    ),
                ],
                cx,
            ))
            .child(chart_block(
                legend("Commands / Sec", cx.theme().chart_1),
                LineChart::new(points.clone())
                    .x(|point| point.label.clone())
                    .y(|point| point.commands)
                    .stroke(cx.theme().chart_1)
                    .dot()
                    .tick_margin(2),
            ))
            .child(chart_block(
                legend("Used Memory", cx.theme().chart_2),
                LineChart::new(points.clone())
                    .x(|point| point.label.clone())
                    .y(|point| point.memory)
                    .stroke(cx.theme().chart_2)
                    .dot()
                    .tick_margin(2),
            ))
            .child(chart_block(
                h_flex()
                    .justify_center()
                    .gap_8()
                    .child(legend("Network Input  (Kb/s)", cx.theme().chart_3))
                    .child(legend(
                        "Network Output  (Kb/s)",
                        cx.theme().muted_foreground,
                    )),
                AreaChart::new(points)
                    .x(|point| point.label.clone())
                    .y(|point| point.input)
                    .stroke(cx.theme().chart_3)
                    .fill(cx.theme().transparent)
                    .y(|point| point.output)
                    .stroke(cx.theme().muted_foreground)
                    .fill(cx.theme().transparent)
                    .y(|point| point.network_ceiling)
                    .stroke(cx.theme().transparent)
                    .fill(linear_gradient(
                        0.0,
                        linear_color_stop(cx.theme().transparent, 1.0),
                        linear_color_stop(cx.theme().transparent, 0.0),
                    ))
                    .tick_margin(2),
            ))
    }
}

fn chart_block(legend: impl IntoElement, chart: impl IntoElement) -> impl IntoElement {
    v_flex()
        .w_full()
        .h(px(230.0))
        .gap_2()
        .child(div().w_full().flex().justify_center().child(legend))
        .child(div().flex_1().px_8().child(chart))
}

fn legend(label: &'static str, color: gpui::Hsla) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(58.0))
                .h(px(16.0))
                .border_4()
                .border_color(color)
                .bg(gpui::transparent_white()),
        )
        .child(div().text_sm().child(label))
}

pub(crate) fn build_points_from_samples(samples: &[Vec<ToolRow>]) -> Vec<ChartPoint> {
    let now = Local::now();
    let samples = if samples.is_empty() {
        vec![Vec::new()]
    } else {
        samples.to_vec()
    };
    let len = samples.len();

    samples
        .into_iter()
        .enumerate()
        .map(|(index, rows)| {
            let seconds_back = len.saturating_sub(index + 1) as i64;
            let time = now - Duration::seconds(seconds_back);
            let input = numeric_metric(&rows, "instantaneous_input_kbps").max(0.0001);
            let output = numeric_metric(&rows, "instantaneous_output_kbps").max(0.0001);
            ChartPoint {
                label: time.format("%H:%M:%S").to_string().into(),
                commands: numeric_metric(&rows, "instantaneous_ops_per_sec").max(0.0001),
                memory: numeric_metric(&rows, "used_memory").max(1.0),
                input,
                output,
                network_ceiling: input.max(output).max(1.0),
            }
        })
        .collect()
}

fn numeric_metric(rows: &[ToolRow], key: &str) -> f64 {
    rows.iter()
        .find(|row| row.key == key)
        .and_then(|row| row.value.parse::<f64>().ok())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, value: &str) -> ToolRow {
        ToolRow {
            category: "Stats".to_string(),
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn chart_points_preserve_sample_history() {
        let points = build_points_from_samples(&[
            vec![
                row("instantaneous_ops_per_sec", "1"),
                row("used_memory", "10"),
            ],
            vec![
                row("instantaneous_ops_per_sec", "2"),
                row("used_memory", "20"),
            ],
        ]);

        assert_eq!(2, points.len());
        assert_eq!(1.0, points[0].commands);
        assert_eq!(20.0, points[1].memory);
    }
}
