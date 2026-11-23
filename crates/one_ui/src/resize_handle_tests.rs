//! 面板分隔条抓取区测试（issue #333）
//!
//! [`crate::resize_handle`] 是所有面板分隔条的唯一实现：数据库 / Redis / MongoDB 的树
//! 与侧栏、SQL 结果面板、终端工作区侧栏都走它。旧实现把命中区整条贴在边界的一侧
//! （外带 padding 也只是往这一侧铺），另一侧完全没有反馈，拖起来一样得像素级对准。
//!
//! 这里逐形状量真实 bounds，并从边界两侧各探一次拖动：内侧（面板里）和外侧（相邻
//! 面板那边）都要抓得住，分隔线还得停在原来那条边界上。

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Axis, Bounds, Context, Entity, InteractiveElement as _, IntoElement,
    Modifiers, MouseButton, ParentElement as _, Pixels, Point, Render, Styled as _, TestAppContext,
    VisualTestContext, Window, WindowBounds, WindowOptions, div, point, px, size,
};
use gpui_component::Root;

use crate::resize_handle::{HandlePlacement, ResizePanel, band_selector, resize_handle};

/// 与 `geometry::resize()` 对齐：分隔线两侧各留的抓取区宽度。
const EDGE_PADDING: f32 = 6.;
/// 面板尺寸：边界坐标一眼能看出来。
const PANEL_WIDTH: f32 = 400.;
const PANEL_HEIGHT: f32 = 300.;
const PANEL_SELECTOR: &str = "resize-test-panel";
const LINE_SELECTOR: &str = "resize-handle-line";
/// 从边界往两侧各探 3px：旧实现只有一侧有抓取区。
const PROBE: f32 = 3.;
/// 面板四周留白，让边界落在窗口内部（含窗口外探针）。
const INSET: f32 = 40.;
/// 拖动距离，比 2px 的拖动起步阈值留够余量。
const DRAG_DISTANCE: f32 = 40.;

/// 面板分隔条实际出现的三种形状。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HandleCase {
    /// 左侧面板的右缘分隔条（`HandlePlacement::Left`）。
    PanelRightEdge,
    /// 右侧面板的左缘分隔条（`HandlePlacement::Right`）。
    PanelLeftEdge,
    /// 底部面板的上缘分隔条（`Axis::Vertical` + 无 placement）。
    PanelTopEdge,
}

impl HandleCase {
    fn axis(self) -> Axis {
        match self {
            Self::PanelTopEdge => Axis::Vertical,
            _ => Axis::Horizontal,
        }
    }

    fn placement(self) -> Option<HandlePlacement> {
        match self {
            Self::PanelRightEdge => Some(HandlePlacement::Left),
            Self::PanelLeftEdge => Some(HandlePlacement::Right),
            Self::PanelTopEdge => None,
        }
    }
}

/// 复刻宿主：一块面板 + 它自己那条分隔条（真实布局、真实元素）。
struct ResizeHandleTestHost {
    case: HandleCase,
    dragged: Rc<Cell<bool>>,
    last_offset: Option<Rc<Cell<Pixels>>>,
    inline: bool,
    width: Pixels,
    continuous_resize: bool,
    pointer_origin: Option<Rc<Cell<Option<Point<Pixels>>>>>,
}

impl ResizeHandleTestHost {
    fn handle(&self, width: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
        let dragged = self.dragged.clone();
        let handle =
            resize_handle::<ResizePanel, ResizePanel>("resize-test-handle", self.case.axis());
        let handle = match self.case.placement() {
            Some(placement) if !self.inline => handle.placement(placement),
            _ if self.inline => handle.inline(),
            _ => handle,
        };

        let last_offset = self.last_offset.clone();
        if self.continuous_resize {
            let pointer_origin = self
                .pointer_origin
                .clone()
                .unwrap_or_else(|| Rc::new(Cell::new(None)));
            let start_width = Rc::new(Cell::new(width));
            let host = cx.entity();
            handle.on_resize(move |position, _, cx| {
                if let Some(position) = position {
                    let start = pointer_origin.get().unwrap_or(position);
                    if start == position {
                        start_width.set(host.read(cx).width);
                    }
                    pointer_origin.set(Some(start));
                    let delta = position.x - start.x;
                    let next_width = start_width.get() + delta;
                    if let Some(last_offset) = &last_offset {
                        last_offset.set(delta);
                    }
                    host.update(cx, |host, cx| {
                        host.width = next_width;
                        host.dragged.set(true);
                        cx.notify();
                    });
                } else {
                    pointer_origin.set(None);
                }
            })
        } else {
            handle.on_drag(ResizePanel, move |_, cursor_offset, _, cx| {
                dragged.set(true);
                if let Some(last_offset) = last_offset.clone() {
                    last_offset.set(cursor_offset.x);
                }
                cx.new(|_| ResizePanel)
            })
        }
    }
}

impl Render for ResizeHandleTestHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = div()
            .id(PANEL_SELECTOR)
            .debug_selector(|| PANEL_SELECTOR.to_owned())
            .relative()
            .w(self.width)
            .h(px(PANEL_HEIGHT));
        div().w_full().h_full().p(px(INSET)).child(if self.inline {
            gpui_component::h_flex()
                .items_stretch()
                .h(px(PANEL_HEIGHT))
                .w_full()
                .child(panel)
                .child(self.handle(self.width, cx))
                .child(div().flex_1().min_w_0().h_full())
                .into_any_element()
        } else {
            panel.child(self.handle(self.width, cx)).into_any_element()
        })
    }
}

fn open_handle_host(
    cx: &mut TestAppContext,
    case: HandleCase,
) -> (VisualTestContext, Entity<ResizeHandleTestHost>) {
    cx.update(gpui_component::init);

    let (window, host) = cx.update(|cx| {
        let bounds = Bounds::centered(None, size(px(PANEL_WIDTH * 2.), px(PANEL_HEIGHT * 2.)), cx);
        let mut host = None;
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    let entity = cx.new(|_| ResizeHandleTestHost {
                        case,
                        dragged: Rc::new(Cell::new(false)),
                        last_offset: None,
                        inline: false,
                        width: px(PANEL_WIDTH),
                        continuous_resize: false,
                        pointer_origin: None,
                    });
                    host = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                },
            )
            .expect("open resize handle test window");
        (window, host.expect("resize handle test host"))
    });

    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let last_offset = Rc::new(Cell::new(px(0.)));
    cx.update(|_, cx| {
        host.update(cx, |host, _| host.last_offset = Some(last_offset.clone()));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    (cx, host)
}

fn panel_bounds(cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds(PANEL_SELECTOR).expect("测试面板")
}

/// 分隔线所在的边界坐标：面板被分隔条贴住的那条边。
fn boundary(panel: Bounds<Pixels>, case: HandleCase) -> Pixels {
    match case {
        HandleCase::PanelRightEdge => panel.right(),
        HandleCase::PanelLeftEdge => panel.left(),
        HandleCase::PanelTopEdge => panel.top(),
    }
}

/// 边界旁 `offset` 处的落点：`offset` 为正表示落到面板**外侧**（相邻面板那边）。
fn probe(panel: Bounds<Pixels>, case: HandleCase, offset: Pixels) -> Point<Pixels> {
    match case {
        HandleCase::PanelRightEdge => point(panel.right() + offset, panel.center().y),
        HandleCase::PanelLeftEdge => point(panel.left() - offset, panel.center().y),
        HandleCase::PanelTopEdge => point(panel.center().x, panel.top() - offset),
    }
}

/// 抓取区必须跨在边界上：边界两侧各铺出至少 [`EDGE_PADDING`]。
fn assert_band_straddles_the_boundary(cx: &mut VisualTestContext, case: HandleCase) {
    let panel = panel_bounds(cx);
    let edge = boundary(panel, case);
    let band = cx
        .debug_bounds(band_selector(case.axis(), case.placement()))
        .expect("抓取区应当被渲染出来");

    let (inside, outside) = match case {
        HandleCase::PanelRightEdge => (edge - band.left(), band.right() - edge),
        HandleCase::PanelLeftEdge => (band.right() - edge, edge - band.left()),
        HandleCase::PanelTopEdge => (edge - band.top(), band.bottom() - edge),
    };
    let least = px(EDGE_PADDING - 0.5);

    assert!(
        inside >= least,
        "面板内侧也得铺够抓取区（{inside:?}）：band={band:?}、边界={edge:?}"
    );
    assert!(
        outside >= least,
        "面板外侧同样得铺够抓取区（{outside:?}）：band={band:?}、边界={edge:?}"
    );
}

/// 分隔线仍应停在原来那条边界上，不能被抓取区带跑。
fn assert_line_stops_at_the_boundary(cx: &mut VisualTestContext, case: HandleCase) {
    let panel = panel_bounds(cx);
    let edge = boundary(panel, case);
    let band = cx
        .debug_bounds(band_selector(case.axis(), case.placement()))
        .expect("抓取区");
    let line = cx.debug_bounds(LINE_SELECTOR).expect("分隔线");

    let overshoot = match case {
        HandleCase::PanelRightEdge => line.right() - edge,
        HandleCase::PanelLeftEdge => line.left() - edge,
        HandleCase::PanelTopEdge => line.top() - edge,
    };

    assert!(
        overshoot.abs() <= px(0.5),
        "分隔线应当压在边界上（偏了 {overshoot:?}）：line={line:?}、band={band:?}、边界={edge:?}"
    );
}

/// 从 `at` 按下并拖一小段，返回这次按下是否真的抓住了分隔条。
fn drag_starts_at(
    cx: &mut VisualTestContext,
    host: &Entity<ResizeHandleTestHost>,
    at: Point<Pixels>,
) -> bool {
    let end = point(at.x + px(DRAG_DISTANCE), at.y + px(DRAG_DISTANCE));
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        point(end.x + px(1.), end.y + px(1.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();

    let started = cx.read(|cx| host.read(cx).dragged.get());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    started
}

#[gpui::test]
fn the_drag_callback_reports_the_offset_from_the_handle_origin(cx: &mut TestAppContext) {
    let case = HandleCase::PanelRightEdge;
    let (mut cx, host) = open_handle_host(cx, case);
    let panel = panel_bounds(&mut cx);
    let start = probe(panel, case, px(0.));
    assert!(drag_starts_at(&mut cx, &host, start));
    let band = cx
        .debug_bounds(band_selector(case.axis(), case.placement()))
        .expect("拖动手柄");
    let initial_offset = start.x - band.origin.x;
    let last_offset = cx.read(|cx| {
        host.read(cx)
            .last_offset
            .as_ref()
            .expect("拖动偏移记录器")
            .get()
    });
    assert_eq!(
        panel.size.width + last_offset - initial_offset,
        px(PANEL_WIDTH + DRAG_DISTANCE),
        "从面板边界拖动 40px 后，宽度应恰好增加 40px"
    );
}

#[gpui::test]
fn the_inline_handle_has_a_stable_drag_origin(cx: &mut TestAppContext) {
    let case = HandleCase::PanelRightEdge;
    let (mut cx, host) = open_handle_host(cx, case);
    cx.update(|_, cx| {
        host.update(cx, |host, _| host.inline = true);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let panel = panel_bounds(&mut cx);
    let band = cx
        .debug_bounds(band_selector(case.axis(), None))
        .expect("正常文档流拖动手柄");
    assert_eq!(band.left(), panel.right(), "inline 手柄必须落在面板右侧");
    assert_eq!(band.size.width, px(13.));
    assert_eq!(band.size.height, panel.size.height);
    cx.update(|_, cx| {
        host.update(cx, |host, _| host.width = px(PANEL_WIDTH));
    });
    let start = band.center();
    assert!(drag_starts_at(&mut cx, &host, start));
    let initial_offset = start.x - band.origin.x;
    let last_offset = cx.read(|cx| {
        host.read(cx)
            .last_offset
            .as_ref()
            .expect("拖动偏移记录器")
            .get()
    });
    assert_eq!(
        panel.size.width + last_offset - initial_offset,
        px(PANEL_WIDTH + DRAG_DISTANCE),
        "从手柄左边界拖动 40px，宽度应恰好增加 40px"
    );
}

/// issue #333：面板右缘的分隔条，边界两侧都要抓得住。
#[gpui::test]
fn the_right_edge_band_straddles_the_boundary(cx: &mut TestAppContext) {
    let case = HandleCase::PanelRightEdge;
    let (mut cx, host) = open_handle_host(cx, case);
    let panel = panel_bounds(&mut cx);

    assert_band_straddles_the_boundary(&mut cx, case);
    assert_line_stops_at_the_boundary(&mut cx, case);
    assert!(
        drag_starts_at(&mut cx, &host, probe(panel, case, px(PROBE))),
        "面板外侧（边界右边 {PROBE}px）也该抓得住分隔条"
    );
    assert!(
        drag_starts_at(&mut cx, &host, probe(panel, case, px(-PROBE))),
        "面板内侧（边界左边 {PROBE}px）也该抓得住分隔条"
    );
}

/// issue #333：面板左缘的分隔条，边界两侧都要抓得住。
#[gpui::test]
fn the_left_edge_band_straddles_the_boundary(cx: &mut TestAppContext) {
    let case = HandleCase::PanelLeftEdge;
    let (mut cx, host) = open_handle_host(cx, case);
    let panel = panel_bounds(&mut cx);

    assert_band_straddles_the_boundary(&mut cx, case);
    assert_line_stops_at_the_boundary(&mut cx, case);
    assert!(
        drag_starts_at(&mut cx, &host, probe(panel, case, px(PROBE))),
        "面板外侧（边界左边 {PROBE}px）也该抓得住分隔条"
    );
    assert!(
        drag_starts_at(&mut cx, &host, probe(panel, case, px(-PROBE))),
        "面板内侧（边界右边 {PROBE}px）也该抓得住分隔条"
    );
}

/// issue #333：底部面板上缘的分隔条，边界两侧都要抓得住。
#[gpui::test]
fn the_top_edge_band_straddles_the_boundary(cx: &mut TestAppContext) {
    let case = HandleCase::PanelTopEdge;
    let (mut cx, host) = open_handle_host(cx, case);
    let panel = panel_bounds(&mut cx);

    assert_band_straddles_the_boundary(&mut cx, case);
    assert_line_stops_at_the_boundary(&mut cx, case);
    assert!(
        drag_starts_at(&mut cx, &host, probe(panel, case, px(PROBE))),
        "面板外侧（边界上边 {PROBE}px）也该抓得住分隔条"
    );
    assert!(
        drag_starts_at(&mut cx, &host, probe(panel, case, px(-PROBE))),
        "面板内侧（边界下边 {PROBE}px）也该抓得住分隔条"
    );
}

#[gpui::test]
fn resize_moves_continuously_with_the_pointer(cx: &mut TestAppContext) {
    let (mut cx, host) = open_handle_host(cx, HandleCase::PanelRightEdge);
    cx.update(|_, cx| {
        host.update(cx, |host, _| {
            host.inline = true;
            host.continuous_resize = true;
            host.pointer_origin = Some(Rc::new(Cell::new(None)));
        });
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let band = cx
        .debug_bounds(band_selector(Axis::Horizontal, None))
        .expect("inline handle");
    let start = band.center();
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        point(start.x + px(5.), start.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    for (delta, width) in [(15., 420.), (35., 455.), (65., 520.)] {
        cx.simulate_mouse_move(
            point(start.x + px(delta), start.y),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.run_until_parked();
        let actual = panel_bounds(&mut cx).size.width;
        let state = cx.read(|cx| {
            let host = host.read(cx);
            (
                host.width,
                host.dragged.get(),
                host.last_offset.as_ref().map(|cell| cell.get()),
                host.pointer_origin.as_ref().map(|cell| cell.get()),
            )
        });
        println!("continuous resize delta={delta:?}: actual={actual:?}, state={state:?}");
        assert_eq!(actual, px(width));
    }
    cx.simulate_mouse_up(
        point(start.x + px(60.), start.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
}

#[gpui::test]
fn continuous_resize_ignores_left_button_drags_that_start_outside_the_handle(
    cx: &mut TestAppContext,
) {
    let (mut cx, host) = open_handle_host(cx, HandleCase::PanelRightEdge);
    cx.update(|_, cx| {
        host.update(cx, |host, _| {
            host.inline = true;
            host.continuous_resize = true;
        });
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let initial_width = panel_bounds(&mut cx).size.width;
    let outside = point(px(20.), px(20.));
    cx.simulate_mouse_down(outside, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(outside, MouseButton::Left, Modifiers::default());
    for delta in [5., 25., 65.] {
        cx.simulate_mouse_move(
            point(outside.x + px(delta), outside.y),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.run_until_parked();
    }
    let (width, dragged) = cx.read(|cx| {
        let host = host.read(cx);
        (host.width, host.dragged.get())
    });
    assert!(!dragged, "从手柄外开始的左键拖动不能触发资源栏 resize");
    assert_eq!(width, initial_width);
    assert_eq!(panel_bounds(&mut cx).size.width, initial_width);
    cx.simulate_mouse_up(
        point(outside.x + px(65.), outside.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
}
