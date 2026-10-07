use db::{ObjectViewColumn, ObjectViewColumnAlign};
use gpui::{
    AnyElement, AppContext, Context, Div, DragMoveEvent, EntityId, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Window, div, px,
};
use gpui_component::{ActiveTheme, table::Column};

/// 列宽抓取区在列边界两侧各自的宽度。
const COLUMN_RESIZE_GRAB_PADDING: Pixels = px(4.0);

/// 分隔线本身的宽度。
const COLUMN_RESIZE_LINE_WIDTH: Pixels = px(1.0);

#[derive(Clone)]
struct ResizeDatabaseColumn {
    entity_id: EntityId,
    col_ix: usize,
}

impl Render for ResizeDatabaseColumn {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.0))
    }
}

pub(crate) fn ui_columns_from_object_columns(columns: &[ObjectViewColumn]) -> Vec<Column> {
    columns
        .iter()
        .map(|column| {
            let mut ui_column = Column::new(column.key.clone(), column.label.clone())
                .width(px(column.width_px))
                .resizable(column.resizable);
            ui_column = match column.align {
                ObjectViewColumnAlign::Left => ui_column,
                ObjectViewColumnAlign::Center => ui_column.text_center(),
                ObjectViewColumnAlign::Right => ui_column.text_right(),
            };
            ui_column
        })
        .collect()
}

pub(crate) fn table_columns_width(columns: &[Column]) -> Pixels {
    columns
        .iter()
        .fold(px(0.0), |width, column| width + column.width)
        .max(px(1.0))
}

pub(crate) fn resize_table_column(columns: &mut [Column], col_ix: usize, width: Pixels) {
    let Some(column) = columns.get_mut(col_ix) else {
        return;
    };

    if !column.resizable {
        return;
    }

    column.width = width.max(column.min_width).min(column.max_width);
}

/// 渲染第 `col_ix` 列表头上、以列边界为中心的两半列宽抓取区。
///
/// 一条列边界由相邻两侧的表头单元格各渲染一半：左列渲染贴在自己右缘的那半（负责
/// 画分隔线），右列渲染贴在自己左缘的那半（只负责接住鼠标），两侧各占
/// [`COLUMN_RESIZE_GRAB_PADDING`]。两半都必须留在自己的单元格里：右侧那一列可能带
/// `overflow_hidden`，跨列的绝对定位会被裁掉；而且后画的兄弟节点先拿到鼠标，跨出去
/// 的那半也会被右侧单元格抢走。
///
/// 返回的是两半本身，调用方必须把它们直接挂到表头单元格下（`.children(...)`）：
/// gpui 的绝对定位是按直接父节点解析 `top_0`/`bottom_0` 的，多包一层尺寸为零的
/// 容器就会让抓取区高度变成 0、彻底接不到鼠标。
///
/// 以前是一条 6px、整条缩在左列右缘里的抓取区：从边界右侧靠过来完全没有反馈，鼠标
/// 得精确停在那几个像素上才拉得动，稍微偏一点光标就退回默认状态。
///
/// 非 resizable 的列返回空，调用方可以直接对每一列调用本函数。
pub(crate) fn render_table_column_resize_handles<T, WidthFn, ResizeFn>(
    id_prefix: &'static str,
    columns: &[Column],
    col_ix: usize,
    cx: &mut Context<T>,
    current_width: WidthFn,
    resize_column: ResizeFn,
) -> Vec<AnyElement>
where
    T: 'static,
    WidthFn: Fn(&T, usize) -> Option<Pixels> + Copy + 'static,
    ResizeFn: Fn(&mut T, usize, Pixels) + Copy + 'static,
{
    let trailing = columns
        .get(col_ix)
        .filter(|column| column.resizable)
        .map(|_| {
            let group_id = SharedString::from(format!("{id_prefix}:{col_ix}"));
            column_resize_band(
                format!("{id_prefix}:{col_ix}"),
                col_ix,
                cx,
                current_width,
                resize_column,
            )
            .debug_selector(move || format!("{id_prefix}-{col_ix}"))
            .right_0()
            .group(group_id.clone())
            .justify_end()
            .items_center()
            .child(
                div()
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
        .filter(|boundary| {
            columns
                .get(*boundary)
                .is_some_and(|column| column.resizable)
        })
        .map(|boundary| {
            column_resize_band(
                format!("{id_prefix}:leading:{boundary}"),
                boundary,
                cx,
                current_width,
                resize_column,
            )
            .debug_selector(move || format!("{id_prefix}-leading-{boundary}"))
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
    boundary_col_ix: usize,
    cx: &mut Context<T>,
    current_width: WidthFn,
    resize_column: ResizeFn,
) -> Stateful<Div>
where
    T: 'static,
    WidthFn: Fn(&T, usize) -> Option<Pixels> + Copy + 'static,
    ResizeFn: Fn(&mut T, usize, Pixels) + Copy + 'static,
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
            move |this, e: &DragMoveEvent<ResizeDatabaseColumn>, _window, cx| {
                let drag = e.drag(cx);
                if drag.entity_id != cx.entity_id() || drag.col_ix != boundary_col_ix {
                    return;
                }

                let Some(width) = current_width(this, boundary_col_ix) else {
                    return;
                };
                let delta = e.event.position.x - e.bounds.center().x;
                resize_column(this, boundary_col_ix, width + delta);
                cx.notify();
            },
        ))
        .on_drag(
            ResizeDatabaseColumn {
                entity_id: cx.entity_id(),
                col_ix: boundary_col_ix,
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

    #[test]
    fn table_columns_width_uses_actual_column_widths() {
        let columns = vec![
            Column::new("user", "User").width(px(140.0)),
            Column::new("host", "Host").width(px(100.0)),
            Column::new("plugin", "Plugin").width(px(180.0)),
        ];

        assert_eq!(px(420.0), table_columns_width(&columns));
        assert_eq!(px(1.0), table_columns_width(&[]));
    }

    #[test]
    fn resize_table_column_updates_width_with_minimum_bound() {
        let mut columns = vec![
            Column::new("name", "Name").width(px(200.0)),
            Column::new("type", "Type").width(px(120.0)),
        ];

        resize_table_column(&mut columns, 0, px(260.0));
        assert_eq!(px(260.0), columns[0].width);

        resize_table_column(&mut columns, 0, px(8.0));
        assert_eq!(px(20.0), columns[0].width);
    }

    #[test]
    fn resize_table_column_ignores_invalid_and_non_resizable_columns() {
        let mut columns = vec![
            Column::new("name", "Name")
                .width(px(200.0))
                .resizable(false),
        ];

        resize_table_column(&mut columns, 0, px(260.0));
        resize_table_column(&mut columns, 99, px(320.0));

        assert_eq!(px(200.0), columns[0].width);
    }
}
