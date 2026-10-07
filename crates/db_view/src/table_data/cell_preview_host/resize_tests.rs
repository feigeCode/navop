//! 单元格预览分隔条的抓取区测试（issue #333）
//!
//! 预览分隔条也是「以分隔线为中心、两侧各一半」的抓取区：数据表格那边一半负责画
//! 分隔线，预览面板那边一半只负责接住鼠标。旧实现是一条 6px、整条压在数据表格那边
//! 的带子，从面板那边靠过来完全没有反馈，鼠标得像素级对准才拉得动。
//!
//! 这里真的把 `DataGrid` + `CellPreviewHost` 跑起来（不查库：构造本身不发起查询），
//! 断言两半确实跨在分隔线两侧，而且从面板那一半按下去也能拖动预览宽度。

use gpui::{
    AppContext as _, Bounds, Entity, IntoElement, Modifiers, MouseButton, ParentElement as _,
    Pixels, Point, Render, Styled as _, TestAppContext, VisualTestContext, Window, WindowBounds,
    WindowOptions, div, point, px, size,
};
use gpui_component::Root;

use crate::table_data::data_grid::{DataGrid, DataGridConfig};

use super::{CellPreviewHost, DEFAULT_PREVIEW_WIDTH, PREVIEW_RESIZE_GRAB_PADDING};
/// 数据表格那边那一半（分隔线所在的那一半）。
const TRAILING_HANDLE: &str = "cell-preview-resize";
/// 预览面板那边那一半。
const LEADING_HANDLE: &str = "cell-preview-resize-leading";
/// 先挪这么多像素把拖动「起步」（起步阈值 2px），剩下的才算真正的拖动距离。
const DRAG_TRIGGER: f32 = 3.;
/// 向左拖动距离：预览面板在右侧，往左拖是把面板拉宽。
const DRAG_DISTANCE: f32 = 60.;
/// 拖动后的期望宽度：`DRAG_DISTANCE` 减去起步用掉的那几像素。
const EXPECTED_WIDTH_DELTA: f32 = DRAG_DISTANCE - DRAG_TRIGGER;
/// 宽度断言容差：指针位置到宽度的换算会带上一点取整。
const WIDTH_TOLERANCE: f32 = 2.;
const WINDOW_WIDTH: f32 = 900.;
const WINDOW_HEIGHT: f32 = 400.;

struct PreviewResizeTestHost {
    host: Entity<CellPreviewHost>,
}

impl Render for PreviewResizeTestHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(self.host.clone())
    }
}

fn open_preview_host(
    cx: &mut TestAppContext,
) -> (VisualTestContext, Entity<PreviewResizeTestHost>) {
    cx.update(|cx| {
        gpui_component::init(cx);
        one_core::gpui_tokio::init(cx);
        cx.set_global(db::GlobalDbState::default());
        cx.set_global(one_core::settings::AppSettings::default());
    });

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
                    let entity = cx.new(|cx| {
                        let grid = cx.new(|cx| {
                            DataGrid::new(
                                DataGridConfig::new(
                                    "test-db",
                                    "test-table",
                                    "test-conn",
                                    one_core::storage::DatabaseType::MySQL,
                                ),
                                None,
                                window,
                                cx,
                            )
                        });
                        let preview = cx.new(|cx| CellPreviewHost::new(grid, window, cx));
                        // 预览默认关着，分隔条只有打开后才渲染。
                        preview.update(cx, |host, cx| host.open_preview(window, cx));
                        PreviewResizeTestHost { host: preview }
                    });
                    host = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                },
            )
            .expect("open cell preview test window");
        (window, host.expect("cell preview test host"))
    });

    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    (cx, host)
}

fn handle(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} 应当被渲染出来"))
}

fn preview_width(cx: &mut VisualTestContext, host: &Entity<PreviewResizeTestHost>) -> Pixels {
    cx.read(|cx| host.read(cx).host.read(cx).preview_width)
}

/// 从 `start` 按下后向左拖 `distance`：先挪 `DRAG_TRIGGER` 起步，再拖到位。
fn drag_left(cx: &mut VisualTestContext, start: Point<Pixels>, distance: f32) {
    let trigger = point(start.x - px(DRAG_TRIGGER), start.y);
    let end = point(start.x - px(distance), start.y);
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(trigger, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
}

/// issue #333：抓取区必须跨在分隔线两侧，而不是整条压在数据表格那边。
#[gpui::test]
fn the_grab_band_straddles_the_preview_divider(cx: &mut TestAppContext) {
    let (mut cx, _host) = open_preview_host(cx);

    let grid_side = handle(&mut cx, TRAILING_HANDLE);
    let panel_side = handle(&mut cx, LEADING_HANDLE);

    assert_eq!(
        PREVIEW_RESIZE_GRAB_PADDING, grid_side.size.width,
        "分隔线左侧只该铺半边的抓取区"
    );
    assert_eq!(
        PREVIEW_RESIZE_GRAB_PADDING, panel_side.size.width,
        "分隔线右侧（面板那侧）只该铺半边的抓取区"
    );
    assert!(
        (panel_side.left() - grid_side.right()).abs() < px(0.5),
        "两半必须首尾相接、跨过同一条分隔线：左侧 {grid_side:?}、右侧 {panel_side:?}"
    );
}

/// issue #333：从**预览面板那一半**按下去也要能拖动预览宽度。
///
/// 旧实现的 6px 抓取区整条压在数据表格那边，面板这一侧的 4px 里按下去只会落到
/// 面板内容上——正是「从右边靠过来完全没有反馈」的现场。
#[gpui::test]
fn dragging_from_the_panel_side_resizes_the_preview(cx: &mut TestAppContext) {
    let (mut cx, host) = open_preview_host(cx);

    let start = handle(&mut cx, LEADING_HANDLE).center();
    drag_left(&mut cx, start, DRAG_DISTANCE);

    let width = preview_width(&mut cx, &host);
    let expected = DEFAULT_PREVIEW_WIDTH + px(EXPECTED_WIDTH_DELTA);
    assert!(
        (width - expected).abs() <= px(WIDTH_TOLERANCE),
        "从面板那侧左拖 {DRAG_DISTANCE}px 后预览宽度应约 {expected:?}，实际 {width:?}"
    );
}

/// 抓着分隔条点一下（没有拖动）不该改预览宽度：抓取区宽了近 4 倍，这一条防的是
/// 「顺手点一下分隔条」被当成一次拖动。
#[gpui::test]
fn a_click_on_the_divider_keeps_the_preview_width(cx: &mut TestAppContext) {
    let (mut cx, host) = open_preview_host(cx);

    let at = handle(&mut cx, LEADING_HANDLE).center();
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();

    assert_eq!(
        DEFAULT_PREVIEW_WIDTH,
        preview_width(&mut cx, &host),
        "点一下分隔条不应改预览宽度"
    );
}
