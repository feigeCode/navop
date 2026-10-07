//! 表头列宽分隔条的抓取区测试（issue #333）
//!
//! 对象列表和用户列表的表头共用 `render_table_column_resize_handles`，这里用一个最小
//! 宿主（两列表头）把那段真实渲染代码跑起来：断言抓取区确实以列边界为中心、两侧各铺
//! 一半，而且边界左右按下去都能拖动同一列的宽度。
//!
//! 旧实现是一条 6px、整条缩在左列右缘里的抓取区：从边界右侧靠过来完全没有反馈，
//! 鼠标得像素级对准才拉得动，稍微偏一点光标就退回默认状态。

use gpui::{
    AppContext as _, Bounds, Entity, IntoElement, Modifiers, MouseButton, ParentElement as _,
    Pixels, Point, Render, Styled as _, TestAppContext, VisualTestContext, Window, WindowBounds,
    WindowOptions, div, point, px, size,
};
use gpui_component::{Root, table::Column};

use crate::database_table_columns::{render_table_column_resize_handles, resize_table_column};

/// 分隔条选择器前缀：宿主传给共享渲染器的 id 前缀。
const HANDLE_SELECTOR_PREFIX: &str = "test-column-resize";
/// 第 0 列右缘那一半（分隔线所在的那一半）。
const TRAILING_HANDLE: &str = "test-column-resize-0";
/// 第 1 列左缘那一半（列边界右侧的抓取区）。
const LEADING_HANDLE: &str = "test-column-resize-leading-0";
/// 边界两侧各自的抓取区宽度，与实现里的 `COLUMN_RESIZE_GRAB_PADDING` 对齐。
const GRAB_PADDING: f32 = 4.;
/// 边界左右两半合起来的总抓手宽度：`GRAB_PADDING` × 2。
const GRAB_SPAN: f32 = 2. * GRAB_PADDING;

const COLUMN_WIDTH: f32 = 100.;
const HEADER_HEIGHT: f32 = 28.;
/// 拖动距离。留够余量，别和 2px 的拖动起步阈值贴太近。
const DRAG_DISTANCE: f32 = 60.;
/// 拖动后列宽至少该增加这么多（指针相对边界有偏移，所以不写等号）。
const WIDENED_AT_LEAST: f32 = 40.;
const WINDOW_WIDTH: f32 = 480.;
const WINDOW_HEIGHT: f32 = 200.;

/// 最小宿主：两列表头，直接复用生产代码里的分隔条渲染器。
struct ResizeHandleTestHost {
    columns: Vec<Column>,
}

impl ResizeHandleTestHost {
    fn new() -> Self {
        Self {
            columns: vec![
                Column::new("first", "first").width(px(COLUMN_WIDTH)),
                Column::new("second", "second").width(px(COLUMN_WIDTH)),
            ],
        }
    }
}

impl Render for ResizeHandleTestHost {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let columns = self.columns.clone();
        let cells = self
            .columns
            .iter()
            .enumerate()
            .map(|(col_ix, column)| {
                div()
                    .relative()
                    .w(column.width)
                    .h(px(HEADER_HEIGHT))
                    .child(column.name.clone())
                    .children(render_table_column_resize_handles(
                        HANDLE_SELECTOR_PREFIX,
                        &columns,
                        col_ix,
                        cx,
                        |this: &Self, col_ix| this.columns.get(col_ix).map(|it| it.width),
                        |this: &mut Self, col_ix, width| {
                            resize_table_column(&mut this.columns, col_ix, width);
                        },
                    ))
            })
            .collect::<Vec<_>>();

        div().flex().w_full().h(px(HEADER_HEIGHT)).children(cells)
    }
}

fn open_resize_handle_host(
    cx: &mut TestAppContext,
) -> (VisualTestContext, Entity<ResizeHandleTestHost>) {
    cx.update(gpui_component::init);

    let (window, host) = cx.update(|cx| {
        let bounds = Bounds::centered(None, size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)), cx);
        let mut host = None;
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    let entity = cx.new(|_| ResizeHandleTestHost::new());
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
    (cx, host)
}

fn column_width(
    cx: &mut VisualTestContext,
    host: &Entity<ResizeHandleTestHost>,
    col_ix: usize,
) -> Pixels {
    cx.read(|cx| host.read(cx).columns[col_ix].width)
}

fn handle(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} 应当被渲染出来"))
}

/// 从 `start` 按下、向右拖 `distance` 后再松手。
fn drag_from(cx: &mut VisualTestContext, start: Point<Pixels>, distance: f32) {
    let end = point(start.x + px(distance), start.y);
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    // 多走一格再松手，确保拖动本身真正起步（起步阈值 2px）。
    cx.simulate_mouse_move(
        point(end.x + px(1.), end.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
}

/// issue #333：抓取区必须骑在列边界上，而不是整条缩在左边那一列里。
#[gpui::test]
fn the_header_grab_straddles_the_column_boundary(cx: &mut TestAppContext) {
    let (mut cx, _host) = open_resize_handle_host(cx);

    let left_of_boundary = handle(&mut cx, TRAILING_HANDLE);
    let right_of_boundary = handle(&mut cx, LEADING_HANDLE);

    assert_eq!(
        px(GRAB_PADDING),
        left_of_boundary.size.width,
        "边界左侧只该铺半边的抓取区"
    );
    assert_eq!(
        px(GRAB_PADDING),
        right_of_boundary.size.width,
        "边界右侧只该铺半边的抓取区"
    );
    assert!(
        (right_of_boundary.left() - left_of_boundary.right()).abs() < px(0.5),
        "两半必须首尾相接、跨过同一条列边界：左半 {left_of_boundary:?}、右半 {right_of_boundary:?}"
    );
    assert_eq!(
        px(GRAB_SPAN),
        right_of_boundary.right() - left_of_boundary.left(),
        "边界两侧合起来才是完整的抓手"
    );
}

/// issue #333：列边界**左侧**按下要能拖动列宽（旧实现这条路径是好的，别改坏）。
#[gpui::test]
fn dragging_from_the_left_half_widens_the_left_column(cx: &mut TestAppContext) {
    let (mut cx, host) = open_resize_handle_host(cx);

    let trailing = handle(&mut cx, TRAILING_HANDLE);
    // 边界左侧 3px：旧实现里这已经在 6px 抓取区之外了。
    let start = point(trailing.right() - px(3.), trailing.center().y);
    drag_from(&mut cx, start, DRAG_DISTANCE);

    let widened = column_width(&mut cx, &host, 0);
    assert!(
        widened > px(COLUMN_WIDTH + WIDENED_AT_LEAST),
        "第 0 列应被向右拖宽约 {DRAG_DISTANCE}px，实际 {widened:?}"
    );
    assert_eq!(
        px(COLUMN_WIDTH),
        column_width(&mut cx, &host, 1),
        "不该动到第 1 列的宽度"
    );
}

/// issue #333：列边界**右侧**按下也要能拖动列宽。
///
/// 边界右侧属于第 1 列，旧实现在那里完全没有抓取区，按下去只会落到第 1 列的表头上。
#[gpui::test]
fn dragging_from_the_right_half_widens_the_left_column(cx: &mut TestAppContext) {
    let (mut cx, host) = open_resize_handle_host(cx);

    let leading = handle(&mut cx, LEADING_HANDLE);
    drag_from(&mut cx, leading.center(), DRAG_DISTANCE);

    let widened = column_width(&mut cx, &host, 0);
    assert!(
        widened > px(COLUMN_WIDTH + WIDENED_AT_LEAST),
        "从边界右侧拖动的也应是第 0 列被拖宽，实际 {widened:?}"
    );
    assert_eq!(
        px(COLUMN_WIDTH),
        column_width(&mut cx, &host, 1),
        "不该动到第 1 列的宽度"
    );
}

/// 抓着分隔条点一下（没有拖动）不该改列宽：抓取区宽了近 4 倍，这一条防的是
/// 「点表头顺手碰到分隔条」被当成一次列宽变更。
#[gpui::test]
fn a_click_on_the_divider_keeps_the_column_width(cx: &mut TestAppContext) {
    let (mut cx, host) = open_resize_handle_host(cx);

    let at = handle(&mut cx, LEADING_HANDLE).center();
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();

    assert_eq!(
        px(COLUMN_WIDTH),
        column_width(&mut cx, &host, 0),
        "点一下分隔条不应改列宽"
    );
}
