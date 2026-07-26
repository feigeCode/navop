use std::{cell::Cell, rc::Rc};

use gpui::{
    AppContext as _, Context, DragMoveEvent, EntityId, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::ActiveTheme as _;

use super::PersistentConnectionSidebar;

pub(super) const CONNECTION_TREE_DEFAULT_WIDTH: Pixels = px(260.0);
pub(super) const CONNECTION_TREE_MIN_WIDTH: Pixels = px(140.0);
pub(super) const CONNECTION_TREE_MAX_WIDTH: Pixels = px(520.0);

#[derive(Clone)]
struct ConnectionTreeResize {
    entity_id: EntityId,
    initial_width: Pixels,
    initial_x: Rc<Cell<Option<Pixels>>>,
}

impl Render for ConnectionTreeResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

pub(super) fn resized_connection_tree_width(
    initial_width: Pixels,
    initial_x: Pixels,
    current_x: Pixels,
) -> Pixels {
    (initial_width + current_x - initial_x)
        .clamp(CONNECTION_TREE_MIN_WIDTH, CONNECTION_TREE_MAX_WIDTH)
}

impl PersistentConnectionSidebar {
    pub(super) fn render_tree_resize_handle(&self, cx: &Context<Self>) -> impl IntoElement {
        let initial_x = Rc::new(Cell::new(None));

        div()
            .id("persistent-connection-tree-resize")
            .group("persistent-connection-tree-resize")
            .absolute()
            .top_0()
            .right_0()
            .h_full()
            .w(px(9.0))
            .flex()
            .justify_end()
            .cursor_col_resize()
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_drag_move(cx.listener(Self::resize_connection_tree))
            .on_drag(
                ConnectionTreeResize {
                    entity_id: cx.entity_id(),
                    initial_width: self.tree_width,
                    initial_x,
                },
                |drag, _, window, cx| {
                    drag.initial_x.set(Some(window.mouse_position().x));
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                },
            )
            .child(
                div()
                    .h_full()
                    .w(px(1.0))
                    .bg(cx.theme().border)
                    .group_hover("persistent-connection-tree-resize", |this| {
                        this.bg(cx.theme().drag_border)
                    }),
            )
    }

    fn resize_connection_tree(
        &mut self,
        event: &DragMoveEvent<ConnectionTreeResize>,
        _window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let drag = event.drag(cx);
        if drag.entity_id != cx.entity_id() {
            return;
        }
        let Some(initial_x) = drag.initial_x.get() else {
            return;
        };
        self.tree_width =
            resized_connection_tree_width(drag.initial_width, initial_x, event.event.position.x);
        cx.notify();
    }
}
