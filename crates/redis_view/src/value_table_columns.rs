//! Redis 集合值表格（List / Set / ZSet / Hash）的列定义与可拖拽列宽。
//!
//! 四种集合视图共用同一套表格布局模型：
//! - 列由 [`ValueColumn`] 描述（i18n 标签、默认宽度、最小宽度）；
//! - 用户拖拽后的列宽由 [`ColumnWidths`] 保存，并按列独立生效；
//! - 表头通过 [`render_column_resize_handles`] 提供拖拽分隔条。

use std::collections::HashMap;

use gpui::{
    AnyElement, AppContext, Context, Div, DragMoveEvent, EntityId, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Window, div, px,
};
use gpui_component::ActiveTheme;

/// 列边界两侧各占的抓取区宽度（像素）
pub(crate) const COLUMN_RESIZE_GRAB_PADDING: Pixels = px(4.0);
/// 列分隔线的宽度（像素）
pub(crate) const COLUMN_RESIZE_LINE_WIDTH: Pixels = px(1.0);
/// 列宽统一下限，避免列被拖到不可用
pub(crate) const MIN_COLUMN_WIDTH: f32 = 56.0;
/// 列宽上限，避免异常拖拽把表格撑到不可用
pub(crate) const MAX_COLUMN_WIDTH: f32 = 4000.0;
/// 表格左右内边距合计（行与表头均为 `px_2`）
pub(crate) const TABLE_GUTTER_WIDTH: f32 = 16.0;

/// 集合值视图中的列。
///
/// 每个变体对应一个固定的列位置，因此枚举值本身即可作为列宽状态的 key。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValueColumn {
    ListIndex,
    ListValue,
    ListAction,
    SetMember,
    SetAction,
    ZSetRank,
    ZSetScore,
    ZSetMember,
    ZSetAction,
    HashField,
    HashValue,
    HashAction,
}

impl ValueColumn {
    /// 该列的 i18n key
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::ListIndex => "KeyValueView.column_index",
            Self::ListValue => "KeyValueView.column_value",
            Self::ListAction => "KeyValueView.column_action",
            Self::SetMember => "KeyValueView.column_member",
            Self::SetAction => "KeyValueView.column_action",
            Self::ZSetRank => "KeyValueView.column_rank",
            Self::ZSetScore => "KeyValueView.column_score",
            Self::ZSetMember => "KeyValueView.column_member",
            Self::ZSetAction => "KeyValueView.column_action",
            Self::HashField => "KeyValueView.column_field",
            Self::HashValue => "KeyValueView.column_value",
            Self::HashAction => "KeyValueView.column_action",
        }
    }

    /// 首次渲染时的列宽
    pub(crate) const fn default_width(self) -> f32 {
        match self {
            Self::ListIndex => 64.0,
            Self::ListValue => 420.0,
            Self::ListAction => 132.0,
            Self::SetMember => 420.0,
            Self::SetAction => 132.0,
            Self::ZSetRank => 64.0,
            Self::ZSetScore => 200.0,
            Self::ZSetMember => 420.0,
            Self::ZSetAction => 132.0,
            Self::HashField => 240.0,
            Self::HashValue => 420.0,
            Self::HashAction => 132.0,
        }
    }

    /// 该列允许的最小宽度
    pub(crate) const fn min_width(self) -> f32 {
        match self {
            Self::ListIndex => 56.0,
            Self::ListValue => 120.0,
            Self::ListAction => 96.0,
            Self::SetMember => 120.0,
            Self::SetAction => 96.0,
            Self::ZSetRank => 56.0,
            // 16 位整数分值在 160px 下也能完整显示（柱状图会先压缩到最小值）
            Self::ZSetScore => 160.0,
            Self::ZSetMember => 120.0,
            Self::ZSetAction => 96.0,
            Self::HashField => 120.0,
            Self::HashValue => 120.0,
            Self::HashAction => 96.0,
        }
    }

    /// 是否为右对齐的尾部操作列
    pub(crate) const fn is_action(self) -> bool {
        matches!(
            self,
            Self::ListAction | Self::SetAction | Self::ZSetAction | Self::HashAction
        )
    }

    /// 是否允许拖拽调整宽度
    ///
    /// 操作列只承载固定数量的图标按钮，调整宽度没有实际收益。
    pub(crate) const fn resizable(self) -> bool {
        !self.is_action()
    }

    /// 稳定的列标识，用于元素 id 与调试选择器
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::ListIndex => "list-index",
            Self::ListValue => "list-value",
            Self::ListAction => "list-action",
            Self::SetMember => "set-member",
            Self::SetAction => "set-action",
            Self::ZSetRank => "zset-rank",
            Self::ZSetScore => "zset-score",
            Self::ZSetMember => "zset-member",
            Self::ZSetAction => "zset-action",
            Self::HashField => "hash-field",
            Self::HashValue => "hash-value",
            Self::HashAction => "hash-action",
        }
    }

    /// 列宽状态的 key
    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

/// List 视图的列顺序
pub(crate) const LIST_COLUMNS: &[ValueColumn] = &[
    ValueColumn::ListIndex,
    ValueColumn::ListValue,
    ValueColumn::ListAction,
];

/// Set 视图的列顺序
pub(crate) const SET_COLUMNS: &[ValueColumn] = &[ValueColumn::SetMember, ValueColumn::SetAction];

/// ZSet 视图的列顺序
pub(crate) const ZSET_COLUMNS: &[ValueColumn] = &[
    ValueColumn::ZSetRank,
    ValueColumn::ZSetScore,
    ValueColumn::ZSetMember,
    ValueColumn::ZSetAction,
];

/// Hash 视图的列顺序
pub(crate) const HASH_COLUMNS: &[ValueColumn] = &[
    ValueColumn::HashField,
    ValueColumn::HashValue,
    ValueColumn::HashAction,
];

/// 把目标列宽钳制到该列允许的区间
pub(crate) fn clamp_column_width(column: ValueColumn, width: f32) -> f32 {
    width.clamp(column.min_width().max(MIN_COLUMN_WIDTH), MAX_COLUMN_WIDTH)
}

/// 用户拖拽后的列宽集合。
///
/// 只保存被用户调整过的列，其余列回落到 [`ValueColumn::default_width`]。
#[derive(Clone, Debug, Default)]
pub(crate) struct ColumnWidths {
    overrides: HashMap<usize, f32>,
}

impl ColumnWidths {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 当前生效的列宽
    pub(crate) fn width(&self, column: ValueColumn) -> f32 {
        self.overrides
            .get(&column.index())
            .copied()
            .unwrap_or_else(|| column.default_width())
    }

    /// 是否被用户调整过
    #[cfg(test)]
    pub(crate) fn is_overridden(&self, column: ValueColumn) -> bool {
        self.overrides.contains_key(&column.index())
    }

    /// 记录用户拖拽后的列宽
    pub(crate) fn resize(&mut self, column: ValueColumn, width: f32) {
        if !column.resizable() {
            return;
        }
        self.overrides
            .insert(column.index(), clamp_column_width(column, width));
    }
}

/// 一组列在当前列宽下的总宽度（不含表格左右内边距）
pub(crate) fn columns_total_width(columns: &[ValueColumn], widths: &ColumnWidths) -> f32 {
    columns
        .iter()
        .map(|column| widths.width(*column))
        .sum::<f32>()
}

/// 表格内容的最小宽度，用于驱动横向滚动
pub(crate) fn columns_min_row_width(columns: &[ValueColumn], widths: &ColumnWidths) -> f32 {
    columns_total_width(columns, widths) + TABLE_GUTTER_WIDTH
}

/// 正在拖拽的列宽分隔条。
#[derive(Clone)]
struct ResizeValueColumn {
    entity_id: EntityId,
    column: ValueColumn,
}

impl Render for ResizeValueColumn {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.0))
    }
}

/// 渲染第 `col_ix` 列表头上、以列边界为中心的两半列宽抓取区。
///
/// 一条列边界由相邻两侧的表头单元格各渲染一半：左列渲染贴在自己右缘的那半（负责
/// 画分隔线），右列渲染贴在自己左缘的那半（只负责接住鼠标），两侧各占
/// [`COLUMN_RESIZE_GRAB_PADDING`]。两半都必须留在自己的单元格里：表头单元格带
/// `overflow_hidden`，跨列的绕对定位会被裁掉。
///
/// 返回的是两半本身，调用方必须把它们直接挂到表头单元格下（`.children(...)`）：
/// gpui 的绕对定位是按直接父节点解析 `top_0`/`bottom_0` 的，多包一层尺寸为零的
/// 容器就会让抓取区高度变成 0、彻底接不到鼠标。
///
/// 以前是一条 6px、整条缩在左列右缘里的抓取区：从边界右侧靠过来完全没有反馈，鼠标
/// 得精确停在那几个像素上才拉得动，稍微偏一点光标就退回默认状态。
///
/// 非 resizable 的列返回空，调用方可以直接对每一列调用本函数。
pub(crate) fn render_column_resize_handles<T, WidthFn, ResizeFn>(
    columns: &[ValueColumn],
    col_ix: usize,
    cx: &mut Context<T>,
    current_width: WidthFn,
    resize_column: ResizeFn,
) -> Vec<AnyElement>
where
    T: 'static,
    WidthFn: Fn(&T, ValueColumn) -> Pixels + Copy + 'static,
    ResizeFn: Fn(&mut T, ValueColumn, Pixels) + Copy + 'static,
{
    let trailing = columns
        .get(col_ix)
        .filter(|column| column.resizable())
        .map(|column| {
            let group_id = SharedString::from(format!("value-column-resize-{}", column.key()));
            column_resize_band(
                format!("value-column-resize-{}", column.key()),
                *column,
                cx,
                current_width,
                resize_column,
            )
            .debug_selector(move || format!("value-column-resize-{}", column.key()))
            .group(group_id.clone())
            .right_0()
            .justify_end()
            .items_center()
            .child(
                div()
                    .debug_selector(move || format!("value-column-resize-line-{}", column.key()))
                    .h_full()
                    .w(COLUMN_RESIZE_LINE_WIDTH)
                    .bg(cx.theme().table_row_border)
                    .group_hover(&group_id, |el| el.bg(cx.theme().border)),
            )
            .into_any_element()
        });

    // 本列左缘那半属于「上一列与这一列之间」的边界：拖动它调整的是上一列的宽度。
    let leading = col_ix
        .checked_sub(1)
        .and_then(|boundary| columns.get(boundary).copied())
        .filter(|column| column.resizable())
        .map(|column| {
            column_resize_band(
                format!("value-column-resize-leading-{}", column.key()),
                column,
                cx,
                current_width,
                resize_column,
            )
            .debug_selector(move || format!("value-column-resize-leading-{}", column.key()))
            .left_0()
            .into_any_element()
        });

    let mut handles = Vec::new();
    handles.extend(trailing);
    handles.extend(leading);
    handles
}

/// 一侧抓取区的交互核心（命中、光标、拖动）：位置与外观由调用方决定。
fn column_resize_band<T, WidthFn, ResizeFn>(
    id: String,
    column: ValueColumn,
    cx: &mut Context<T>,
    current_width: WidthFn,
    resize_column: ResizeFn,
) -> Stateful<Div>
where
    T: 'static,
    WidthFn: Fn(&T, ValueColumn) -> Pixels + Copy + 'static,
    ResizeFn: Fn(&mut T, ValueColumn, Pixels) + Copy + 'static,
{
    div()
        .id(id)
        .absolute()
        .top_0()
        .bottom_0()
        .w(COLUMN_RESIZE_GRAB_PADDING)
        .flex()
        .cursor_col_resize()
        .occlude()
        .on_drag_move(cx.listener(
            move |this, e: &DragMoveEvent<ResizeValueColumn>, _window, cx| {
                let drag = e.drag(cx);
                if drag.entity_id != cx.entity_id() || drag.column != column {
                    return;
                }

                let width = current_width(this, column);
                let delta = e.event.position.x - e.bounds.center().x;
                resize_column(this, column, width + delta);
                cx.notify();
            },
        ))
        .on_drag(
            ResizeValueColumn {
                entity_id: cx.entity_id(),
                column,
            },
            |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 所有列，用于遍历式校验
    const ALL_COLUMNS: &[ValueColumn] = &[
        ValueColumn::ListIndex,
        ValueColumn::ListValue,
        ValueColumn::ListAction,
        ValueColumn::SetMember,
        ValueColumn::SetAction,
        ValueColumn::ZSetRank,
        ValueColumn::ZSetScore,
        ValueColumn::ZSetMember,
        ValueColumn::ZSetAction,
        ValueColumn::HashField,
        ValueColumn::HashValue,
        ValueColumn::HashAction,
    ];

    #[test]
    fn column_widths_default_to_declared_widths() {
        let widths = ColumnWidths::new();

        for column in [
            ValueColumn::ListIndex,
            ValueColumn::ListValue,
            ValueColumn::SetMember,
            ValueColumn::ZSetRank,
            ValueColumn::ZSetScore,
            ValueColumn::ZSetMember,
            ValueColumn::HashField,
            ValueColumn::HashValue,
        ] {
            assert_eq!(
                column.default_width(),
                widths.width(column),
                "unexpected default width for {column:?}"
            );
            assert!(!widths.is_overridden(column));
        }
    }

    #[test]
    fn resize_records_the_requested_width() {
        let mut widths = ColumnWidths::new();

        widths.resize(ValueColumn::ZSetMember, 512.0);

        assert_eq!(512.0, widths.width(ValueColumn::ZSetMember));
        assert!(widths.is_overridden(ValueColumn::ZSetMember));
    }

    #[test]
    fn resize_clamps_width_to_the_column_minimum() {
        let mut widths = ColumnWidths::new();

        widths.resize(ValueColumn::ZSetMember, 12.0);

        assert_eq!(
            ValueColumn::ZSetMember.min_width(),
            widths.width(ValueColumn::ZSetMember)
        );

        widths.resize(ValueColumn::ZSetRank, 4.0);

        assert_eq!(
            ValueColumn::ZSetRank.min_width(),
            widths.width(ValueColumn::ZSetRank)
        );
    }

    #[test]
    fn resize_clamps_width_to_the_global_bounds() {
        assert_eq!(
            MAX_COLUMN_WIDTH,
            clamp_column_width(ValueColumn::ListValue, f32::INFINITY)
        );
        assert_eq!(
            ValueColumn::ListValue.min_width(),
            clamp_column_width(ValueColumn::ListValue, -100.0)
        );
    }

    #[test]
    fn declared_widths_respect_the_global_bounds() {
        for column in ALL_COLUMNS {
            assert!(
                column.min_width() >= MIN_COLUMN_WIDTH,
                "{column:?} minimum width is below the global floor"
            );
            assert!(
                column.default_width() >= column.min_width(),
                "{column:?} default width is below its own minimum"
            );
            assert!(
                column.default_width() <= MAX_COLUMN_WIDTH,
                "{column:?} default width exceeds the global ceiling"
            );
            assert_eq!(
                column.min_width(),
                clamp_column_width(*column, 0.0),
                "{column:?} should not shrink below its minimum width"
            );
        }
    }

    #[test]
    fn resize_keeps_columns_independent_across_views() {
        let mut widths = ColumnWidths::new();

        widths.resize(ValueColumn::ZSetMember, 300.0);

        assert_eq!(300.0, widths.width(ValueColumn::ZSetMember));
        assert_eq!(
            ValueColumn::SetMember.default_width(),
            widths.width(ValueColumn::SetMember)
        );
        assert_eq!(
            ValueColumn::HashValue.default_width(),
            widths.width(ValueColumn::HashValue)
        );
    }

    #[test]
    fn action_columns_are_not_resizable() {
        let mut widths = ColumnWidths::new();

        assert!(!ValueColumn::ZSetAction.resizable());
        assert!(ValueColumn::ZSetMember.resizable());

        widths.resize(ValueColumn::ZSetAction, 400.0);

        assert_eq!(
            ValueColumn::ZSetAction.default_width(),
            widths.width(ValueColumn::ZSetAction)
        );
        assert!(!widths.is_overridden(ValueColumn::ZSetAction));
    }

    #[test]
    fn resize_replaces_the_previous_width() {
        let mut widths = ColumnWidths::new();

        widths.resize(ValueColumn::HashField, 360.0);
        widths.resize(ValueColumn::HashField, 280.0);

        assert_eq!(280.0, widths.width(ValueColumn::HashField));
        assert!(widths.is_overridden(ValueColumn::HashField));
    }

    #[test]
    fn columns_min_row_width_adds_the_table_gutter() {
        let widths = ColumnWidths::new();

        assert_eq!(
            columns_total_width(ZSET_COLUMNS, &widths) + TABLE_GUTTER_WIDTH,
            columns_min_row_width(ZSET_COLUMNS, &widths)
        );
        assert_eq!(
            ValueColumn::ZSetRank.default_width()
                + ValueColumn::ZSetScore.default_width()
                + ValueColumn::ZSetMember.default_width()
                + ValueColumn::ZSetAction.default_width(),
            columns_total_width(ZSET_COLUMNS, &widths)
        );
    }

    #[test]
    fn column_views_declare_expected_columns() {
        assert_eq!(3, LIST_COLUMNS.len());
        assert_eq!(2, SET_COLUMNS.len());
        assert_eq!(4, ZSET_COLUMNS.len());
        assert_eq!(3, HASH_COLUMNS.len());

        for columns in [LIST_COLUMNS, SET_COLUMNS, ZSET_COLUMNS, HASH_COLUMNS] {
            assert!(
                columns.last().is_some_and(|column| column.is_action()),
                "action column must be the trailing column"
            );
        }
    }
}
