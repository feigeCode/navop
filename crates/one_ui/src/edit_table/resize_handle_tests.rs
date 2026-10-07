//! 表头列宽分隔条的抓取区测试（issue #333）
//!
//! 旧的分隔条是「2px 宽、整条缩在列内、贴着右缘」的一根细带：鼠标得像素级
//! 对准才碰得到，指针稍微偏一点，col-resize 光标和悬停高亮就一起消失——表头
//! 宽度因此很难拉。修法是把抓取区做成跨在列边界上的两半边（左右各
//! [`GRAB_PADDING`] 宽），边界两侧的列各自渲染自己那半边。
//!
//! 这里走真实布局和真实鼠标事件：一部分断言来自 `debug_bounds`（抓取区到底铺在
//! 哪儿、两半是否首尾相接跨过边界），另一部分来自表格自己广播的
//! `ColumnWidthsChanged`（拖动真的改了列宽，而且只改被拖的那一列）。

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, IntoElement, Modifiers, MouseButton,
    ParentElement as _, Pixels, Point, Render, SharedString, Styled as _, TestAppContext,
    VisualTestContext, Window, WindowBounds, WindowOptions, div, point, px, size,
};
use gpui_component::Root;

use crate::edit_table::{Column, EditTable, EditTableDelegate, EditTableEvent, EditTableState};

/// 边界两侧各自的抓取区宽度，与实现里的 `COLUMN_RESIZE_GRAB_PADDING` 对齐。
const GRAB_PADDING: f32 = 4.;
/// 边界左右两半合起来的总抓手宽度：`GRAB_PADDING` × 2。
const GRAB_SPAN: f32 = 2. * GRAB_PADDING;
/// 第 0 列右缘那半边（分隔线所在的那一半）。
const COL0_TRAILING_HANDLE: &str = "resizable-handle-0";
/// 第 1 列左缘那半边（列边界右侧的抓取区）。
const COL1_LEADING_HANDLE: &str = "resizable-handle-leading-1";

const COLUMN_COUNT: usize = 3;
const COLUMN_WIDTH: f32 = 120.;
const ROW_COUNT: usize = 3;
/// 拖动距离。留够余量，别和 2px 的拖动起步阈值贴太近。
const DRAG_DISTANCE: f32 = 60.;
/// 拖动后列宽至少该增加这么多（指针相对边界有偏移，所以不写等号）。
const WIDENED_AT_LEAST: f32 = 40.;
const WINDOW_WIDTH: f32 = 600.;
const WINDOW_HEIGHT: f32 = 400.;

/// 最小 delegate：3 列 × 3 行，默认列宽、默认可缩放。
struct ResizeTestDelegate {
    rows: Vec<Vec<String>>,
}

impl EditTableDelegate for ResizeTestDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        COLUMN_COUNT
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        let name = SharedString::from(format!("col-{col_ix}"));
        Column::new(name.clone(), name).width(px(COLUMN_WIDTH))
    }

    fn get_cell_value(&self, row_ix: usize, col_ix: usize, _cx: &App) -> String {
        self.rows
            .get(row_ix)
            .and_then(|row| row.get(col_ix))
            .cloned()
            .unwrap_or_default()
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<EditTableState<Self>>,
    ) -> impl IntoElement {
        div().child(self.get_cell_value(row_ix, col_ix, cx))
    }
}

/// 复刻宿主：把表格挂进 `Root`，并订阅列宽变化事件。
///
/// 列宽是表格内部状态（`col_groups` 是私有字段），宿主能拿到的公开信号只有
/// `ColumnWidthsChanged`——真实宿主保存列宽走的也正是这条路。
struct ResizeTestHost {
    table: Entity<EditTableState<ResizeTestDelegate>>,
    width_events: Vec<Vec<Pixels>>,
}

impl ResizeTestHost {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            EditTableState::new(
                ResizeTestDelegate {
                    rows: vec![vec!["cell".to_string(); COLUMN_COUNT]; ROW_COUNT],
                },
                window,
                cx,
            )
        });

        cx.subscribe(&table, |host, _, event: &EditTableEvent, _cx| {
            if let EditTableEvent::ColumnWidthsChanged(widths) = event {
                host.width_events.push(widths.clone());
            }
        })
        .detach();

        Self {
            table,
            width_events: Vec::new(),
        }
    }
}

impl Render for ResizeTestHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(EditTable::new(&self.table))
    }
}

fn open_resize_host(cx: &mut TestAppContext) -> (VisualTestContext, Entity<ResizeTestHost>) {
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
                    let entity = cx.new(|cx| ResizeTestHost::new(window, cx));
                    host = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                },
            )
            .expect("open edit table resize test window");
        (window, host.expect("resize test host"))
    });

    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    (cx, host)
}

/// 第 0 列与第 1 列之间的列边界：边界左侧那半抓取区的右缘就是它。
fn column_boundary(cx: &mut VisualTestContext) -> Pixels {
    cx.debug_bounds(COL0_TRAILING_HANDLE)
        .expect("第 0 列右缘的抓取区")
        .right()
}

/// 从 `start` 按下、向右拖 `distance` 后再松手，返回表格广播的列宽变化。
fn drag_from(
    cx: &mut VisualTestContext,
    host: &Entity<ResizeTestHost>,
    start: Point<Pixels>,
    distance: f32,
) -> Vec<Vec<Pixels>> {
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

    cx.read(|cx| host.read(cx).width_events.clone())
}

/// 断言这次拖动只把第 0 列拖宽了，并且列边界跟着指针一起右移。
fn assert_only_the_first_column_was_widened(
    cx: &mut VisualTestContext,
    events: &[Vec<Pixels>],
    boundary_before: Pixels,
) {
    assert_eq!(1, events.len(), "一次拖动只应广播一次列宽变化");

    let widths = &events[0];
    assert!(
        widths[0] > px(COLUMN_WIDTH + WIDENED_AT_LEAST),
        "第 0 列应被向右拖宽约 {DRAG_DISTANCE}px，实际 {:?}",
        widths[0]
    );
    assert_eq!(px(COLUMN_WIDTH), widths[1], "不该动到第 1 列的宽度");
    assert_eq!(px(COLUMN_WIDTH), widths[2], "不该动到第 2 列的宽度");

    let moved = column_boundary(cx) - boundary_before;
    assert!(
        moved > px(WIDENED_AT_LEAST),
        "列边界应跟着指针一起右移（{moved:?}）"
    );
}

/// issue #333：抓取区必须跨在列边界上，而不是整条缩在左边那一列里。
#[gpui::test]
fn the_grab_band_straddles_the_column_boundary(cx: &mut TestAppContext) {
    let (mut cx, _host) = open_resize_host(cx);

    // 边界左侧的那半边：缩在第 0 列右缘内侧，分隔线就画在它的右缘。
    let left_of_boundary = cx
        .debug_bounds(COL0_TRAILING_HANDLE)
        .expect("第 0 列右缘的抓取区应当被渲染出来");
    // 边界右侧的那半边：缩在第 1 列左缘内侧。
    let right_of_boundary = cx
        .debug_bounds(COL1_LEADING_HANDLE)
        .expect("第 1 列左缘的抓取区应当被渲染出来");

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

/// issue #333：在列边界**左侧**按下也要能拖动列宽。
///
/// 旧实现的 2px 抓取区是 `[边界 - 2, 边界]`，边界左侧 3px 已经出了抓取区，按下去
/// 只会落到第 0 列的单元格上——正是「必须非常小心地悬停才拿得到光标」的位置。
#[gpui::test]
fn dragging_from_the_left_of_the_boundary_widens_the_left_column(cx: &mut TestAppContext) {
    let (mut cx, host) = open_resize_host(cx);

    let handle = cx
        .debug_bounds(COL0_TRAILING_HANDLE)
        .expect("第 0 列右缘的抓取区");
    let start = point(handle.right() - px(3.), handle.center().y);
    let events = drag_from(&mut cx, &host, start, DRAG_DISTANCE);

    assert_only_the_first_column_was_widened(&mut cx, &events, handle.right());
}

/// issue #333：在列边界**右侧**按下也要能拖动列宽。
///
/// 边界右侧属于第 1 列，旧实现在那里完全没有抓取区，按下去只会落到第 1 列的
/// 单元格上。
#[gpui::test]
fn dragging_from_the_right_of_the_boundary_widens_the_left_column(cx: &mut TestAppContext) {
    let (mut cx, host) = open_resize_host(cx);

    let leading = cx
        .debug_bounds(COL1_LEADING_HANDLE)
        .expect("第 1 列左缘的抓取区");
    // 边界右侧 2px：左半抓取区的中心。
    let start = leading.center();
    let boundary_before = column_boundary(&mut cx);
    let events = drag_from(&mut cx, &host, start, DRAG_DISTANCE);

    assert_only_the_first_column_was_widened(&mut cx, &events, boundary_before);
}

/// 抓着分隔条点一下（没有拖动）不该被当成改列宽：抓取区宽了近 4 倍，这一条防的
/// 是「点表头顺手碰到分隔条」被广播成一次列宽变更。
#[gpui::test]
fn a_click_on_the_divider_does_not_resize_any_column(cx: &mut TestAppContext) {
    let (mut cx, host) = open_resize_host(cx);

    let at = cx
        .debug_bounds(COL1_LEADING_HANDLE)
        .expect("第 1 列左缘的抓取区")
        .center();

    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();

    assert!(
        cx.read(|cx| host.read(cx).width_events.is_empty()),
        "点一下分隔条不应广播列宽变化"
    );
}
