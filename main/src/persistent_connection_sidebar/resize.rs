use gpui::{
    AppContext as _, Context, DragMoveEvent, InteractiveElement, IntoElement, ParentElement,
    Pixels, StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::ActiveTheme as _;
use one_ui::resize_handle::ResizePanel;

use super::PersistentConnectionSidebar;

pub(super) const CONNECTION_TREE_DEFAULT_WIDTH: Pixels = px(260.0);
pub(super) const CONNECTION_TREE_MIN_WIDTH: Pixels = px(140.0);
pub(super) const CONNECTION_TREE_MAX_WIDTH: Pixels = px(520.0);

impl PersistentConnectionSidebar {
    pub(super) fn render_tree_resize_handle(&self, cx: &Context<Self>) -> impl IntoElement {
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
            .on_drag(ResizePanel, |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
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

    pub(super) fn resize_connection_tree(
        &mut self,
        event: &DragMoveEvent<ResizePanel>,
        _window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let _ = event.drag(cx);
        let delta = event.event.position.x - event.bounds.center().x;
        self.tree_width =
            (self.tree_width + delta).clamp(CONNECTION_TREE_MIN_WIDTH, CONNECTION_TREE_MAX_WIDTH);
        cx.notify();
    }
}
