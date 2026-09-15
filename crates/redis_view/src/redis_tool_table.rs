//! Redis 工具页签共享的 Table delegate。

use crate::redis_tool_data::RedisToolKind;
use gpui::{App, Context, IntoElement, ParentElement, SharedString, Styled, Window, div, px};
use gpui_component::{
    ActiveTheme, h_flex,
    table::{Column, ColumnFixed, TableDelegate, TableState},
};
use rust_i18n::t;

/// 通用单元格,字段为 SharedString 以共享底层缓冲。
pub type Cells = Vec<SharedString>;

/// 各页签的列配置 + 当前行数据。
pub struct RedisToolTableDelegate {
    columns: Vec<Column>,
    rows: Vec<Cells>,
    empty_title: SharedString,
    empty_detail: SharedString,
}

impl RedisToolTableDelegate {
    pub fn new(kind: RedisToolKind) -> Self {
        let (empty_title, empty_detail) = empty_message(kind);
        Self {
            columns: columns_for(kind),
            rows: Vec::new(),
            empty_title,
            empty_detail,
        }
    }

    pub fn set_rows(&mut self, rows: Vec<Cells>) {
        self.rows = rows;
    }
}

impl TableDelegate for RedisToolTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let text = self
            .rows
            .get(row_ix)
            .and_then(|row| row.get(col_ix))
            .cloned()
            .unwrap_or_default();
        div().w_full().truncate().child(text)
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let title = self.empty_title.clone();
        let detail = self.empty_detail.clone();
        h_flex().size_full().items_center().justify_center().child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_base()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(cx.theme().muted_foreground)
                        .child(title),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground.opacity(0.7))
                        .child(detail),
                ),
        )
    }
}

fn columns_for(kind: RedisToolKind) -> Vec<Column> {
    match kind {
        RedisToolKind::Info => vec![
            Column::new("tag", "Tag")
                .width(px(130.0))
                .min_width(px(80.0))
                .max_width(px(220.0))
                .fixed(ColumnFixed::Left),
            Column::new("key", "Key")
                .width(px(220.0))
                .min_width(px(120.0))
                .max_width(px(360.0))
                .fixed(ColumnFixed::Left),
            Column::new("value", "Value")
                .width(px(360.0))
                .min_width(px(160.0))
                .max_width(px(720.0)),
            Column::new("tip", "Tip")
                .width(px(260.0))
                .min_width(px(120.0))
                .max_width(px(420.0)),
        ],
        RedisToolKind::Memory => vec![
            Column::new("metric", "Metric")
                .width(px(260.0))
                .min_width(px(160.0))
                .max_width(px(360.0))
                .fixed(ColumnFixed::Left),
            Column::new("value", "Value")
                .width(px(200.0))
                .min_width(px(120.0))
                .max_width(px(320.0)),
            Column::new("tip", "Tip")
                .width(px(420.0))
                .min_width(px(160.0))
                .max_width(px(720.0)),
        ],
        RedisToolKind::SlowLog => vec![
            Column::new("id", "ID")
                .width(px(90.0))
                .min_width(px(70.0))
                .max_width(px(140.0))
                .fixed(ColumnFixed::Left)
                .text_center(),
            Column::new("command", "Command")
                .width(px(420.0))
                .min_width(px(200.0))
                .max_width(px(720.0)),
            Column::new("time", "Time")
                .width(px(180.0))
                .min_width(px(140.0))
                .max_width(px(220.0)),
            Column::new("duration", "Duration")
                .width(px(120.0))
                .min_width(px(90.0))
                .max_width(px(180.0))
                .text_right(),
            Column::new("client", "Client")
                .width(px(220.0))
                .min_width(px(140.0))
                .max_width(px(320.0)),
        ],
        RedisToolKind::Monitor => vec![
            Column::new("command", "Command")
                .width(px(220.0))
                .min_width(px(140.0))
                .max_width(px(360.0))
                .fixed(ColumnFixed::Left),
            Column::new("calls", "Calls")
                .width(px(130.0))
                .min_width(px(80.0))
                .max_width(px(200.0))
                .text_right(),
            Column::new("usec", "Usec")
                .width(px(150.0))
                .min_width(px(100.0))
                .max_width(px(220.0))
                .text_right(),
            Column::new("usec_per_call", "Usec / Call")
                .width(px(150.0))
                .min_width(px(100.0))
                .max_width(px(220.0))
                .text_right(),
            Column::new("failed", "Failed")
                .width(px(120.0))
                .min_width(px(80.0))
                .max_width(px(180.0))
                .text_right(),
        ],
        RedisToolKind::PubSub => vec![
            Column::new("kind", t!("RedisPubSub.column_kind").to_string())
                .width(px(150.0))
                .min_width(px(100.0))
                .max_width(px(220.0))
                .fixed(ColumnFixed::Left),
            Column::new("name", t!("RedisPubSub.column_name").to_string())
                .width(px(360.0))
                .min_width(px(160.0))
                .max_width(px(640.0)),
            Column::new(
                "subscribers",
                t!("RedisPubSub.column_subscribers").to_string(),
            )
            .width(px(160.0))
            .min_width(px(100.0))
            .max_width(px(240.0))
            .text_right(),
        ],
        // Chart 实际上不会使用本 delegate,但保留一个占位避免 panic。
        RedisToolKind::Chart => vec![Column::new("info", "Info").width(px(200.0))],
    }
}

fn empty_message(kind: RedisToolKind) -> (SharedString, SharedString) {
    match kind {
        RedisToolKind::Info => (
            t!("RedisTool.empty_info_title").to_string().into(),
            t!("RedisTool.empty_info_detail").to_string().into(),
        ),
        RedisToolKind::Memory => (
            t!("RedisTool.empty_memory_title").to_string().into(),
            t!("RedisTool.empty_memory_detail").to_string().into(),
        ),
        RedisToolKind::SlowLog => (
            t!("RedisTool.empty_slowlog_title").to_string().into(),
            t!("RedisTool.empty_slowlog_detail").to_string().into(),
        ),
        RedisToolKind::Monitor => (
            t!("RedisTool.empty_monitor_title").to_string().into(),
            t!("RedisTool.empty_monitor_detail").to_string().into(),
        ),
        RedisToolKind::PubSub => (
            t!("RedisPubSub.empty_no_channels_title").to_string().into(),
            t!("RedisPubSub.empty_no_channels_detail")
                .to_string()
                .into(),
        ),
        RedisToolKind::Chart => ("--".into(), "".into()),
    }
}
