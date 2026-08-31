use gpui::{
    AnyElement, App, ElementId, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Pixels, Point, RenderOnce, Styled, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    popover::Popover,
};

/// 将窗口坐标锚点适配为当前宿主内的绝对定位 Popover trigger。
///
/// 组件只负责坐标转换；deferred、外部点击、Escape 和焦点生命周期仍由
/// `gpui_component::Popover` 管理。
#[derive(IntoElement)]
pub(crate) struct WindowPositionedPopover {
    id: ElementId,
    window_position: Point<Pixels>,
    host_origin: Point<Pixels>,
    open: bool,
    focus_handle: FocusHandle,
    on_open_change: Option<Box<dyn Fn(&bool, &mut Window, &mut App)>>,
    content: Option<AnyElement>,
}

impl WindowPositionedPopover {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        window_position: Point<Pixels>,
        focus_handle: FocusHandle,
    ) -> Self {
        Self {
            id: id.into(),
            window_position,
            host_origin: Point::default(),
            open: false,
            focus_handle,
            on_open_change: None,
            content: None,
        }
    }

    pub(crate) fn host_origin(mut self, origin: Point<Pixels>) -> Self {
        self.host_origin = origin;
        self
    }

    pub(crate) fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub(crate) fn on_open_change(
        mut self,
        callback: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Box::new(callback));
        self
    }

    pub(crate) fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    fn local_position(window_position: Point<Pixels>, host_origin: Point<Pixels>) -> Point<Pixels> {
        Point::new(
            window_position.x - host_origin.x,
            window_position.y - host_origin.y,
        )
    }
}

impl RenderOnce for WindowPositionedPopover {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let position = Self::local_position(self.window_position, self.host_origin);
        let trigger = Button::new("window-positioned-popover-anchor")
            .text()
            .tab_stop(false)
            .w_0()
            .h(gpui::px(0.));

        let mut popover = Popover::new(self.id)
            .anchor(gpui::Anchor::TopLeft)
            .open(self.open)
            .track_focus(&self.focus_handle)
            .trigger(trigger);
        if let Some(callback) = self.on_open_change {
            popover = popover.on_open_change(callback);
        }
        if let Some(content) = self.content {
            popover = popover.child(content);
        }

        gpui::div()
            .debug_selector(|| "window-positioned-popover-anchor".to_string())
            .absolute()
            .left(position.x)
            .top(position.y)
            .w_0()
            .h(gpui::px(0.))
            .child(popover)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, Context, Render, TestAppContext, VisualTestContext, div, px};
    use gpui_component::Root;

    struct PositionedPopoverHost {
        focus_handle: FocusHandle,
    }

    impl Render for PositionedPopoverHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().relative().child(
                div()
                    .absolute()
                    .left(px(300.))
                    .top(px(40.))
                    .w(px(200.))
                    .h(px(200.))
                    .overflow_hidden()
                    .child(
                        WindowPositionedPopover::new(
                            "test-positioned-popover",
                            Point::new(px(50.), px(120.)),
                            self.focus_handle.clone(),
                        )
                        .host_origin(Point::new(px(300.), px(40.)))
                        .open(true)
                        .content(
                            div()
                                .debug_selector(|| "window-positioned-popover-content".to_string())
                                .w(px(20.))
                                .h(px(20.)),
                        ),
                    ),
            )
        }
    }

    #[test]
    fn converts_window_position_to_host_local_position() {
        let position = WindowPositionedPopover::local_position(
            Point::new(gpui::px(50.), gpui::px(120.)),
            Point::new(gpui::px(300.), gpui::px(40.)),
        );

        assert_eq!(Point::new(gpui::px(-250.), gpui::px(80.)), position);
    }

    #[gpui::test]
    fn lays_out_the_trigger_at_the_requested_window_position(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (_, cx): (_, &mut VisualTestContext) = cx.add_window_view(|window, cx| {
            let host = cx.new(|cx| PositionedPopoverHost {
                focus_handle: cx.focus_handle(),
            });
            Root::new(host, window, cx)
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();

        let anchor = cx
            .debug_bounds("window-positioned-popover-anchor")
            .expect("positioned popover trigger is laid out");
        let content = cx
            .debug_bounds("window-positioned-popover-content")
            .expect("deferred popover content escapes host clipping");

        assert_eq!(Point::new(px(50.), px(120.)), anchor.origin);
        assert!(content.origin.x >= anchor.origin.x);
        assert!(content.origin.x < anchor.origin.x + px(40.));
        assert!(content.origin.y >= anchor.origin.y);
    }
}
