use std::{cell::Cell, rc::Rc};

use crate::geometry;
use gpui::{
    AnyElement, App, AppContext as _, Axis, Element, ElementId, Empty, Entity, GlobalElementId,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement as _, Pixels, Point, Render, StatefulInteractiveElement, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::ActiveTheme as _;

/// Legacy default resize-handle edge padding.
#[deprecated(note = "Use one_ui::theme_geometry().resize.edge_padding")]
pub const HANDLE_PADDING: Pixels = px(4.);

/// Legacy default resize-handle visible line size.
#[deprecated(note = "Use one_ui::theme_geometry().resize.visible_line")]
pub const HANDLE_SIZE: Pixels = px(1.);

/// 分隔线的调试选择器名（[`crate::resize_handle`] 的测试按它取真实 bounds）。
const LINE_SELECTOR: &str = "resize-handle-line";

/// 抓取区的调试选择器名：按形状分名，方便测试直接量到命中区到底铺在哪儿。
///
/// `VisualTestContext::debug_bounds` 只收字面量选择器，所以这里是 `'static str`。
pub(crate) fn band_selector(axis: Axis, placement: Option<HandlePlacement>) -> &'static str {
    match (axis, placement) {
        (Axis::Horizontal, Some(HandlePlacement::Left)) => "resize-handle-band-horizontal-left",
        (Axis::Horizontal, Some(HandlePlacement::Right)) => "resize-handle-band-horizontal-right",
        (Axis::Horizontal, None) => "resize-handle-band-horizontal-none",
        (Axis::Vertical, _) => "resize-handle-band-vertical",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlePlacement {
    Left,
    Right,
}

#[derive(Clone)]
pub struct ResizePanel;

impl Render for ResizePanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub fn resize_handle<T: 'static, E: 'static + Render>(
    id: impl Into<ElementId>,
    axis: Axis,
) -> ResizeHandle<T, E> {
    ResizeHandle::new(id, axis)
}

pub struct ResizeHandle<T: 'static, E: 'static + Render> {
    id: ElementId,
    axis: Axis,
    drag_value: Option<Rc<T>>,
    placement: Option<HandlePlacement>,
    inline: bool,
    idle_line_color: Option<gpui::Hsla>,
    on_drag: Option<Rc<dyn Fn(&Point<Pixels>, &mut Window, &mut App) -> Entity<E>>>,
    on_resize: Option<Rc<dyn Fn(Option<Point<Pixels>>, &mut Window, &mut App)>>,
}

impl<T: 'static, E: 'static + Render> ResizeHandle<T, E> {
    fn new(id: impl Into<ElementId>, axis: Axis) -> Self {
        let id = id.into();
        Self {
            id: id.clone(),
            on_drag: None,
            on_resize: None,
            drag_value: None,
            placement: None,
            inline: false,
            idle_line_color: None,
            axis,
        }
    }

    pub fn on_drag(
        mut self,
        value: T,
        f: impl Fn(Rc<T>, &Point<Pixels>, &mut Window, &mut App) -> Entity<E> + 'static,
    ) -> Self {
        let value = Rc::new(value);
        self.drag_value = Some(value.clone());
        self.on_drag = Some(Rc::new(move |p, window, cx| {
            f(value.clone(), p, window, cx)
        }));
        self
    }

    /// 连续调整尺寸。`Some(position)` 是启动或移动时的鼠标位置，`None` 表示结束。
    /// 与 `on_drag` 的启动偏移分开，调用方必须用“当前点 − 启动点”计算位移。
    pub fn on_resize(
        mut self,
        f: impl Fn(Option<Point<Pixels>>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_resize = Some(Rc::new(f));
        self
    }

    pub fn placement(mut self, placement: HandlePlacement) -> Self {
        self.placement = Some(placement);
        self.inline = false;
        self
    }

    /// 手柄作为正常文档流中的固定宽度抓取区，适合应用资源栏。
    pub fn inline(mut self) -> Self {
        self.inline = true;
        self.placement = None;
        self
    }

    /// 覆盖静止时的分隔线颜色；hover 和拖动仍使用主题反馈色。
    pub fn idle_line_color(mut self, color: gpui::Hsla) -> Self {
        self.idle_line_color = Some(color);
        self
    }
}

#[derive(Default, Debug, Clone)]
struct ResizeHandleState {
    // GPUI 会克隆元素状态用于不同事件回调；必须共享 Cell，不能在 Clone 时复制布尔值。
    active: Rc<Cell<bool>>,
    pointer_origin: Rc<Cell<Option<Point<Pixels>>>>,
}

impl ResizeHandleState {
    fn set_active(&self, active: bool) {
        self.active.set(active);
    }

    fn is_active(&self) -> bool {
        self.active.get()
    }
}

impl<T: 'static, E: 'static + Render> IntoElement for ResizeHandle<T, E> {
    type Element = ResizeHandle<T, E>;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl<T: 'static, E: 'static + Render> Element for ResizeHandle<T, E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let axis = self.axis;
        let placement = self.placement;

        window.with_element_state(id.unwrap(), |state, window| {
            let state = state.unwrap_or(ResizeHandleState::default());
            let resize = geometry::resize();
            let handle_padding = resize.edge_padding;
            let handle_size = resize.visible_line;
            let hit_area = resize.hit_area();

            let bg_color = if state.is_active() {
                cx.theme().drag_border
            } else {
                self.idle_line_color.unwrap_or(cx.theme().border)
            };

            let is_horizontal = axis == Axis::Horizontal;

            let mut el = div()
                .id(self.id.clone())
                .debug_selector(move || band_selector(axis, placement).to_owned())
                .occlude()
                .when(!self.inline, |this| this.absolute())
                .flex()
                .flex_shrink_0()
                .group("handle")
                .when_some(self.on_drag.clone(), |this, on_drag| {
                    this.on_drag(
                        self.drag_value.clone().unwrap(),
                        move |_, position, window, cx| on_drag(&position, window, cx),
                    )
                })
                .when_some(self.on_resize.clone(), |this, on_resize| {
                    let state = state.clone();
                    this.on_drag(ResizePanel, move |_, _, window, cx| {
                        state.set_active(true);
                        on_resize(Some(window.mouse_position()), window, cx);
                        cx.new(|_| ResizePanel)
                    })
                })
                .map(|this| {
                    if self.inline {
                        return this
                            .when(is_horizontal, |this| {
                                // 内联手柄排在面板之后、占一条抓取区宽度；分隔线贴右对齐，
                                // 正好落在面板与内容的分界上（铺底色由调用方负责）。
                                this.cursor_col_resize().h_full().w(hit_area).justify_end()
                            })
                            .when(!is_horizontal, |this| {
                                this.cursor_row_resize().w_full().h(hit_area).items_center()
                            });
                    }
                    match placement {
                        // 抓取区以边界为中心铺开：往边框外多铺一格（`mr/ml` 负边距），
                        // 分隔线仍旧停在面板自己那一像素边框上。
                        Some(HandlePlacement::Left) => this
                            .cursor_col_resize()
                            .top_0()
                            .right_0()
                            .mr(-handle_padding)
                            .h_full()
                            .w(hit_area)
                            .justify_center(),
                        Some(HandlePlacement::Right) => this
                            .cursor_col_resize()
                            .top_0()
                            .left_0()
                            .ml(-handle_padding)
                            .h_full()
                            .w(hit_area)
                            .justify_center(),
                        None => this
                            .when(is_horizontal, |this| {
                                this.cursor_col_resize()
                                    .top_0()
                                    .left_0()
                                    .ml(-handle_padding)
                                    .h_full()
                                    .w(hit_area)
                                    .justify_center()
                            })
                            .when(!is_horizontal, |this| {
                                this.cursor_row_resize()
                                    .top_0()
                                    .left_0()
                                    .mt(-handle_padding)
                                    .w_full()
                                    .h(hit_area)
                                    .items_center()
                            }),
                    }
                })
                .child(
                    div()
                        .id(LINE_SELECTOR)
                        .debug_selector(|| LINE_SELECTOR.to_owned())
                        .bg(bg_color)
                        .group_hover("handle", |this| this.bg(cx.theme().drag_border))
                        .when(is_horizontal, |this| this.h_full().w(handle_size))
                        .when(!is_horizontal, |this| this.w_full().h(handle_size)),
                )
                .into_any_element();

            let layout_id = el.request_layout(window, cx);

            ((layout_id, el), state)
        })
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        request_layout.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        request_layout.paint(window, cx);

        window.with_element_state(id.unwrap(), |state: Option<ResizeHandleState>, window| {
            let state = state.unwrap_or(ResizeHandleState::default());

            window.on_mouse_event({
                let state = state.clone();
                move |ev: &MouseDownEvent, phase, window, _| {
                    if ev.button == MouseButton::Left
                        && bounds.contains(&ev.position)
                        && phase.bubble()
                    {
                        state.set_active(true);
                        window.refresh();
                    }
                }
            });

            window.on_mouse_event({
                let state = state.clone();
                let on_resize = self.on_resize.clone();
                move |ev: &MouseMoveEvent, _, window, cx| {
                    if state.is_active()
                        && ev.pressed_button == Some(MouseButton::Left)
                        && let Some(on_resize) = &on_resize
                    {
                        on_resize(Some(ev.position), window, cx);
                    }
                }
            });

            window.on_mouse_event({
                let state = state.clone();
                let on_resize = self.on_resize.clone();
                move |_: &MouseUpEvent, phase, window, cx| {
                    if state.is_active() && phase.bubble() {
                        state.set_active(false);
                        state.pointer_origin.set(None);
                        if let Some(on_resize) = &on_resize {
                            on_resize(None, window, cx);
                        }
                        window.refresh();
                    }
                }
            });

            ((), state)
        });
    }
}
