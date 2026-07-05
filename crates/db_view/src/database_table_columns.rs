use db::{ObjectViewColumn, ObjectViewColumnAlign};
use gpui::{
    AnyElement, AppContext, Context, DragMoveEvent, EntityId, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Window, div,
    px,
};
use gpui_component::{ActiveTheme, table::Column};

const HEADER_RESIZE_HANDLE_WIDTH: Pixels = px(6.0);

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

pub(crate) fn render_table_column_resize_handle<T, WidthFn, ResizeFn>(
    id_prefix: &'static str,
    group_prefix: &'static str,
    col_ix: usize,
    column: &Column,
    cx: &mut Context<T>,
    current_width: WidthFn,
    resize_column: ResizeFn,
) -> AnyElement
where
    T: 'static,
    WidthFn: Fn(&T, usize) -> Option<Pixels> + Copy + 'static,
    ResizeFn: Fn(&mut T, usize, Pixels) + Copy + 'static,
{
    if !column.resizable {
        return div().into_any_element();
    }

    let group_id = SharedString::from(format!("{group_prefix}:{col_ix}"));
    div()
        .id((id_prefix, col_ix))
        .group(group_id.clone())
        .absolute()
        .right_0()
        .top_0()
        .bottom_0()
        .w(HEADER_RESIZE_HANDLE_WIDTH)
        .cursor_col_resize()
        .occlude()
        .flex()
        .justify_end()
        .items_center()
        .child(
            div()
                .h_full()
                .w(px(1.0))
                .bg(cx.theme().table_row_border)
                .group_hover(&group_id, |el| el.bg(cx.theme().border)),
        )
        .on_drag_move(cx.listener(
            move |this, e: &DragMoveEvent<ResizeDatabaseColumn>, _window, cx| {
                let drag = e.drag(cx);
                if drag.entity_id != cx.entity_id() || drag.col_ix != col_ix {
                    return;
                }

                let Some(width) = current_width(this, col_ix) else {
                    return;
                };
                let delta = e.event.position.x - e.bounds.center().x;
                resize_column(this, col_ix, width + delta);
                cx.notify();
            },
        ))
        .on_drag(
            ResizeDatabaseColumn {
                entity_id: cx.entity_id(),
                col_ix,
            },
            |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            },
        )
        .into_any_element()
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
