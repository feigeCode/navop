//! collection 模板的表格实现。
//!
//! 用 gpui-component 的 `DataTable` + `TableDelegate` 承载 manifest 声明的列与行操作,
//! 以获得主题化表头、斑马纹、行悬停、列缩放、本地排序和骨架加载态;
//! 手写 div 拼表格在亮色主题下既不像表格也不跟随主题。

use std::cmp::Ordering;

use extension_runtime::extension::manifest::ResourceWorkbenchCollection;
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Edges, Entity,
    InteractiveElement as _, IntoElement, ParentElement, Pixels, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, WeakEntity, Window, div, prelude::FluentBuilder as _,
    px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, StyledExt as _, h_flex, tag::Tag, v_flex,
    button::Button,
    table::{Column, ColumnSort, TableDelegate, TableState},
};
use serde_json::Value;

use crate::{NativeResourceWorkbench, RunningRowAction};

/// 行操作按钮宽度(仅图标时更窄)。
const ICON_ACTION_WIDTH: f32 = 34.;
const TEXT_ACTION_WIDTH: f32 = 84.;
/// 行首"可进入"指示列宽度。
const INDICATOR_WIDTH: f32 = 30.;

/// 列声明的渲染语义,来自 manifest 的 `style` 字段。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CellStyle {
    /// 默认文本。
    Plain,
    /// 状态徽章。
    Badge,
    /// 等宽字体(镜像名、id、路径)。
    Mono,
    /// 次要文本(时间、描述)。
    Muted,
}

impl CellStyle {
    fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("badge") | Some("status") => Self::Badge,
            Some("mono") | Some("code") | Some("id") => Self::Mono,
            Some("muted") | Some("secondary") | Some("time") => Self::Muted,
            _ => Self::Plain,
        }
    }
}

#[derive(Clone)]
struct TableColumn {
    key: SharedString,
    title: SharedString,
    path: String,
    style: CellStyle,
    width: Pixels,
    /// 行首指示列:不可排序、不参与交互。
    indicator: bool,
    /// 行操作列。
    actions: bool,
}

/// 行操作的渲染信息(effect 已在宿主侧解析,delegate 只负责画)。
#[derive(Clone)]
pub(crate) struct RowActionView {
    pub id: SharedString,
    pub label: SharedString,
    pub operation: String,
    pub destructive: bool,
    pub icon: Option<IconName>,
}

/// manifest collection → 表格 delegate。
pub(crate) struct CollectionTableDelegate {
    columns: Vec<TableColumn>,
    rows: Vec<Value>,
    key_paths: Vec<String>,
    actions: Vec<RowActionView>,
    view: WeakEntity<NativeResourceWorkbench>,
    loading: bool,
    /// 行是否可点击进入详情页。
    openable: bool,
    sort: Option<(usize, ColumnSort)>,
}

impl CollectionTableDelegate {
    pub(crate) fn new(
        collection: &ResourceWorkbenchCollection,
        items: Vec<Value>,
        actions: Vec<RowActionView>,
        view: WeakEntity<NativeResourceWorkbench>,
        loading: bool,
    ) -> Self {
        let openable = collection.open.is_some();
        let mut columns = Vec::with_capacity(collection.columns.len() + 2);
        if openable {
            columns.push(TableColumn {
                key: SharedString::from("__indicator__"),
                title: SharedString::default(),
                path: String::new(),
                style: CellStyle::Plain,
                width: px(INDICATOR_WIDTH),
                indicator: true,
                actions: false,
            });
        }
        for column in &collection.columns {
            let style = CellStyle::parse(column.style.as_deref());
            columns.push(TableColumn {
                key: SharedString::from(column.id.clone()),
                title: SharedString::from(column.title.clone()),
                path: column.path.clone(),
                style,
                width: px(column_width(style, columns.len())),
                indicator: false,
                actions: false,
            });
        }
        if !actions.is_empty() {
            let width: f32 = actions
                .iter()
                .map(|action| {
                    if action.icon.is_some() {
                        ICON_ACTION_WIDTH
                    } else {
                        TEXT_ACTION_WIDTH
                    }
                })
                .sum::<f32>()
                + 16.;
            columns.push(TableColumn {
                key: SharedString::from("__actions__"),
                title: SharedString::from("Actions"),
                path: String::new(),
                style: CellStyle::Plain,
                width: px(width.max(96.)),
                indicator: false,
                actions: true,
            });
        }
        Self {
            columns,
            rows: items,
            key_paths: collection.key_paths.clone(),
            actions,
            view,
            loading,
            openable,
            sort: None,
        }
    }

    fn row(&self, row_ix: usize) -> Option<&Value> {
        self.rows.get(row_ix)
    }

    fn cell_text(&self, row_ix: usize, path: &str) -> String {
        self.row(row_ix)
            .map(|row| display(&lookup(row, path)))
            .unwrap_or_default()
    }

    /// 正在执行的行操作(读取宿主视图的实时状态)。
    fn running(&self, cx: &App) -> Option<RunningRowAction> {
        let view = self.view.upgrade()?;
        view.read(cx).row_action_running.clone()
    }

    fn render_indicator(&self, cx: &App) -> AnyElement {
        if !self.openable {
            return div().into_any_element();
        }
        Icon::new(IconName::ChevronRight)
            .size_3p5()
            .text_color(cx.theme().muted_foreground)
            .into_any_element()
    }

    fn render_cell_content(&self, row_ix: usize, column: &TableColumn, cx: &App) -> AnyElement {
        let text = self.cell_text(row_ix, &column.path);
        let theme = cx.theme();
        match column.style {
            CellStyle::Badge if !text.is_empty() => status_tag(&text, cx).into_any_element(),
            CellStyle::Mono => div()
                .min_w_0()
                .truncate()
                .font_family(theme.mono_font_family.clone())
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element(),
            CellStyle::Muted => div()
                .min_w_0()
                .truncate()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element(),
            style => div()
                .min_w_0()
                .truncate()
                .text_sm()
                // 可进入的行,首列用主色提示可点击。
                .when(
                    self.openable && style == CellStyle::Plain && column_is_primary(self, column),
                    |this| this.text_color(theme.foreground).font_medium(),
                )
                .child(text)
                .into_any_element(),
        }
    }

    fn render_actions(&self, row_ix: usize, cx: &App) -> AnyElement {
        let running = self.running(cx);
        let row_key = self
            .row(row_ix)
            .map(|row| row_key(row, &self.key_paths))
            .unwrap_or_default();
        let row = self.row(row_ix).cloned().unwrap_or(Value::Null);
        let entity = self.view.clone();
        let mut cell = h_flex().w_full().min_w_0().items_center().justify_end().gap_1();
        for action in &self.actions {
            let is_running = running.as_ref().is_some_and(|running| {
                running.operation == action.operation && running.row_key == row_key
            });
            let id = SharedString::from(format!("row-action:{}:{}", row_ix, action.id));
            let mut button = Button::new(id)
                .with_size(Size::XSmall)
                .loading(is_running)
                .tooltip(action.label.clone());
            button = match action.icon.clone() {
                Some(icon) => button.icon(icon),
                None => button.label(action.label.clone()),
            };
            if action.destructive {
                button = button.text_color(cx.theme().danger);
            }
            let entity = entity.clone();
            let operation = action.operation.clone();
            let payload = row.clone();
            cell = cell.child(button.on_click(move |_event: &ClickEvent, _window, cx: &mut App| {
                // 行操作按钮不回落到行点击(导航)。
                cx.stop_propagation();
                let Some(view) = entity.upgrade() else {
                    return;
                };
                let operation = operation.clone();
                let payload = payload.clone();
                view.update(cx, |this, cx| {
                    this.run_row_action(operation, payload, false, cx);
                });
            }));
        }
        cell.into_any_element()
    }

    /// 空态:图标 + 标题 + 说明。
    fn render_empty_state(&self, cx: &App) -> AnyElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                Icon::new(IconName::Inbox)
                    .size_8()
                    .text_color(cx.theme().muted_foreground.opacity(0.6)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child("Nothing to show"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No rows were returned for this view."),
            )
            .into_any_element()
    }
}

/// 首列(不含指示列)按主色高亮。
fn column_is_primary(delegate: &CollectionTableDelegate, column: &TableColumn) -> bool {
    delegate
        .columns
        .iter()
        .find(|candidate| !candidate.indicator && !candidate.actions)
        .is_some_and(|candidate| candidate.key == column.key)
}

impl TableDelegate for CollectionTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        let Some(column) = self.columns.get(col_ix) else {
            return Column::default();
        };
        let paddings = Edges {
            top: px(0.),
            bottom: px(0.),
            left: px(12.),
            right: px(12.),
        };
        let mut built = Column::new(column.key.clone(), column.title.clone())
            .width(column.width)
            .min_width(px(72.))
            .resizable(!column.indicator)
            .movable(false)
            .paddings(paddings);
        if column.actions || column.indicator {
            built = built.selectable(false);
        } else if let Some((sorted, sort)) = self.sort {
            built = if sorted == col_ix {
                built.sort(sort)
            } else {
                built.sortable()
            };
        } else {
            built = built.sortable();
        }
        built
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let title = self
            .columns
            .get(col_ix)
            .map(|column| column.title.clone())
            .unwrap_or_default();
        h_flex()
            .size_full()
            .items_center()
            .text_xs()
            .font_medium()
            .text_color(cx.theme().table_head_foreground)
            .child(title)
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<gpui::Div> {
        let row = div().id(("resource-row", row_ix));
        let Some(entity) = self.view.upgrade() else {
            return row;
        };
        let _ = cx;
        if !self.openable {
            return row;
        }
        let payload = self.rows.get(row_ix).cloned().unwrap_or(Value::Null);
        row.cursor_pointer().on_click(window.listener_for(
            &entity,
            move |this: &mut NativeResourceWorkbench,
                  _event: &ClickEvent,
                  _window: &mut Window,
                  cx: &mut Context<NativeResourceWorkbench>| {
                this.open_collection_row(payload.clone(), cx);
            },
        ))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(column) = self.columns.get(col_ix).cloned() else {
            return div().into_any_element();
        };
        if column.indicator {
            return h_flex()
                .size_full()
                .items_center()
                .child(self.render_indicator(cx))
                .into_any_element();
        }
        if column.actions {
            return self.render_actions(row_ix, cx);
        }
        let content = self.render_cell_content(row_ix, &column, cx);
        h_flex().size_full().min_w_0().items_center().child(content).into_any_element()
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) {
        let Some(column) = self.columns.get(col_ix).cloned() else {
            return;
        };
        if column.indicator || column.actions {
            return;
        }
        self.rows.sort_by(|left, right| {
            compare_values(
                &display(&lookup(left, &column.path)),
                &display(&lookup(right, &column.path)),
            )
        });
        if sort == ColumnSort::Descending {
            self.rows.reverse();
        }
        self.sort = Some((col_ix, sort));
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        self.render_empty_state(cx)
    }

    fn loading(&self, _cx: &App) -> bool {
        self.loading
    }
}

/// 构建表格实体(在渲染期按数据版本重建)。
pub(crate) fn build_table_state(
    delegate: CollectionTableDelegate,
    window: &mut Window,
    cx: &mut Context<NativeResourceWorkbench>,
) -> Entity<TableState<CollectionTableDelegate>> {
    cx.new(|cx| {
        // 行点击与行操作都是显式交互,关掉框架自带的行/列选中高亮以免语义冲突。
        TableState::new(delegate, window, cx)
            .row_selectable(false)
            .col_selectable(false)
            .col_movable(false)
            .sortable(true)
    })
}

/// 行标识:按 manifest 声明的 `keyPaths` 取值拼接,用于定位"正在执行的行操作"。
pub(crate) fn row_key(row: &Value, key_paths: &[String]) -> String {
    key_paths
        .iter()
        .map(|path| display(&lookup(row, path)))
        .collect::<Vec<_>>()
        .join("|")
}

/// 从页面 load 结果中取出 collection 的行集。
pub(crate) fn items_of(collection: &ResourceWorkbenchCollection, value: &Value) -> Vec<Value> {
    if value.is_null() {
        return Vec::new();
    }
    let pointer = if collection.items_path.starts_with('/') {
        collection.items_path.clone()
    } else {
        format!("/{}", collection.items_path)
    };
    value
        .pointer(&pointer)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

impl RowActionView {
    /// 由 manifest 声明构造渲染信息;`destructive` 由宿主按 operation effect 判定。
    pub(crate) fn from_manifest(
        action: &extension_runtime::extension::manifest::ResourceWorkbenchRowAction,
        destructive: bool,
    ) -> Self {
        Self {
            id: SharedString::from(action.id.clone()),
            label: SharedString::from(action.label.clone()),
            operation: action.operation.clone(),
            destructive,
            // 常见操作给出图标,未知操作退化为文字按钮。
            icon: action_icon(&action.id).or_else(|| action_icon(&action.operation)),
        }
    }
}

fn action_icon(id: &str) -> Option<IconName> {
    let id = id.to_ascii_lowercase();
    if id.contains("restart") {
        Some(IconName::RotateCw)
    } else if id.contains("start") || id.contains("run") || id.contains("play") {
        Some(IconName::Play)
    } else if id.contains("stop") || id.contains("pause") || id.contains("kill") {
        Some(IconName::Pause)
    } else if id.contains("remove") || id.contains("delete") || id.contains("prune") {
        Some(IconName::Delete)
    } else if id.contains("log") {
        Some(IconName::FileText)
    } else if id.contains("open") || id.contains("detail") || id.contains("view") {
        Some(IconName::Eye)
    } else {
        None
    }
}

fn lookup(row: &Value, path: &str) -> Value {
    if path.is_empty() {
        return Value::Null;
    }
    let pointer = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    row.pointer(&pointer).cloned().unwrap_or(Value::Null)
}

fn display(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Bool(flag) => flag.to_string(),
        other => other.to_string(),
    }
}

fn compare_values(left: &str, right: &str) -> Ordering {
    match (left.parse::<f64>(), right.parse::<f64>()) {
        (Ok(left), Ok(right)) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
        _ => left.to_lowercase().cmp(&right.to_lowercase()),
    }
}

/// 状态语义:决定徽章配色。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StatusTone {
    Good,
    Bad,
    Warn,
    Neutral,
}

impl StatusTone {
    /// 覆盖容器/进程/任务的常见状态词,未知值回落中性。
    fn of(value: &str) -> Self {
        let normalized = value.trim().to_ascii_lowercase();
        if starts_with_any(
            &normalized,
            &["running", "healthy", "active", "up", "ready", "ok", "started", "open"],
        ) {
            Self::Good
        } else if starts_with_any(
            &normalized,
            &["exit", "dead", "unhealthy", "fail", "error", "down", "crash", "remove", "stopped"],
        ) {
            Self::Bad
        } else if starts_with_any(
            &normalized,
            &[
                "paused", "restarting", "pending", "degrad", "warning", "creating", "starting",
                "stopping", "queued",
            ],
        ) {
            Self::Warn
        } else {
            Self::Neutral
        }
    }

    fn color(self, cx: &App) -> gpui::Hsla {
        match self {
            Self::Good => cx.theme().success,
            Self::Bad => cx.theme().danger,
            Self::Warn => cx.theme().warning,
            Self::Neutral => cx.theme().muted_foreground,
        }
    }
}

/// 状态徽章:描边样式 + 前置状态点,配色全部取自主题。
fn status_tag(text: &str, cx: &App) -> Tag {
    let tone = StatusTone::of(text);
    let tag = match tone {
        StatusTone::Good => Tag::success(),
        StatusTone::Bad => Tag::danger(),
        StatusTone::Warn => Tag::warning(),
        StatusTone::Neutral => Tag::secondary(),
    };
    tag.with_size(Size::Small)
        .rounded_full()
        .gap_1()
        .outline()
        .child(div().size_1p5().rounded_full().bg(tone.color(cx)))
        .child(text.to_owned())
}

fn starts_with_any(value: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| value.starts_with(prefix))
}

/// 列宽策略:状态窄、镜像/ID 宽,其余中等;全部可拖拽调整。
fn column_width(style: CellStyle, index: usize) -> f32 {
    match style {
        CellStyle::Badge => 132.,
        CellStyle::Mono => 240.,
        _ if index == 0 => 260.,
        _ => 200.,
    }
}
