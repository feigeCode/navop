use std::{cell::Cell, rc::Rc};

use gpui::{
    AppContext as _, Context, DragMoveEvent, EntityId, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, StatefulInteractiveElement, Styled, Window, div,
};
use gpui_component::ActiveTheme as _;

use super::PersistentConnectionSidebar;

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
    min_width: Pixels,
    max_width: Pixels,
) -> Pixels {
    (initial_width + current_x - initial_x).clamp(min_width, max_width)
}

impl PersistentConnectionSidebar {
    pub(super) fn render_tree_resize_handle(&self, cx: &Context<Self>) -> impl IntoElement {
        let initial_x = Rc::new(Cell::new(None));
        let resize = cx.theme().geometry.resize;

        div()
            .id("persistent-connection-tree-resize")
            .group("persistent-connection-tree-resize")
            .absolute()
            .top_0()
            .right_0()
            .h_full()
            .w(resize.hit_area())
            .flex()
            .justify_end()
            .cursor_col_resize()
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // 拖拽结束（手柄跟随宽度重渲染，通常仍命中光标）时落盘最终宽度。
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.persist_tree_width(cx);
                }),
            )
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
                    .w(resize.visible_line)
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
        let layout = cx.theme().geometry.layout;
        let width = resized_connection_tree_width(
            drag.initial_width,
            initial_x,
            event.event.position.x,
            layout.context_sidebar_min,
            layout.context_sidebar_max,
        );
        self.set_tree_width(width, cx);
        // 指针快速移出手柄命中区时 mouse-up 兜底可能丢失，这里按增量阈值落盘。
        self.persist_tree_width_if_moved_far(cx);
    }
}
