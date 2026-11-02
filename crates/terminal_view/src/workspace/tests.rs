use gpui::{
    AnyElement, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Render, Styled as _, TestAppContext, VisualTestContext, Window, div, px,
};
use gpui_component::Placement;
use gpui_component::resizable::{ResizableState, h_resizable, resizable_panel};

use super::split_model::{TerminalPaneId, TerminalSplitTree};
use super::split_sizing::TerminalSplitSizing;

const PANE_MIN_SIZE: Pixels = px(120.);
const HARNESS_WIDTH: Pixels = px(900.);
const HARNESS_HEIGHT: Pixels = px(400.);
const PANE_SELECTORS: [&str; 3] = ["pane-0", "pane-1", "pane-2"];

/// 分屏渲染的最小可测形态。
///
/// 与 `TerminalWorkspace::render_workspace` 一样由宿主(这里是测试视图)持有
/// `TerminalSplitSizing`,再把 state 喂给 `h_resizable(..).with_state(..)`。
/// `show_split = false` 时整棵分屏元素树不再渲染,等价于切走标签页——
/// 元素缓存被丢弃,而视图实体仍然存活。
struct SplitHarness {
    tree: TerminalSplitTree,
    sizing: TerminalSplitSizing,
    show_split: bool,
}

impl SplitHarness {
    fn new(tree: TerminalSplitTree) -> Self {
        Self {
            tree,
            sizing: TerminalSplitSizing::new(),
            show_split: true,
        }
    }

    fn root_split(&mut self, cx: &mut Context<Self>) -> Entity<ResizableState> {
        let (split_id, children) = self
            .tree
            .groups()
            .first()
            .cloned()
            .expect("测试树至少应有一个拆分分组");
        self.sizing.state(split_id, children, cx)
    }
}

impl Render for SplitHarness {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyElement = if self.show_split {
            self.sizing.reconcile(&self.tree, cx);
            let (split_id, children) = self
                .tree
                .groups()
                .first()
                .cloned()
                .expect("测试树至少应有一个拆分分组");
            let pane_count = children.len();
            let state = self.sizing.state(split_id, children, cx);
            // 与生产一致:分组 id 稳定,所以切回来后元素身份不变;
            // 尺寸能保住靠的是宿主持有的 state,不是元素缓存。
            let mut group = h_resizable("split-harness").with_state(&state);
            for &selector in PANE_SELECTORS.iter().take(pane_count) {
                group = group.child(
                    resizable_panel()
                        .size_range(PANE_MIN_SIZE..Pixels::MAX)
                        .child(
                            div()
                                .size_full()
                                .debug_selector(move || selector.to_string()),
                        ),
                );
            }
            group.into_any_element()
        } else {
            div()
                .debug_selector(|| "other-tab".to_string())
                .into_any_element()
        };

        div().w(HARNESS_WIDTH).h(HARNESS_HEIGHT).child(content)
    }
}

fn split_three_panes() -> TerminalSplitTree {
    let mut tree = TerminalSplitTree::new(TerminalPaneId::new(1));
    assert!(
        tree.split(
            TerminalPaneId::new(1),
            TerminalPaneId::new(2),
            Placement::Right
        ),
        "第一次拆分应成功"
    );
    assert!(
        tree.split(
            TerminalPaneId::new(2),
            TerminalPaneId::new(3),
            Placement::Right
        ),
        "第二次拆分应成功"
    );
    tree
}

fn draw(visual: &mut VisualTestContext) {
    for _ in 0..3 {
        visual.update(|window, cx| window.draw(cx).clear(cx));
        visual.run_until_parked();
    }
}

fn pane_width(visual: &mut VisualTestContext, selector: &'static str) -> Pixels {
    visual
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} 应已渲染"))
        .size
        .width
}

fn assert_width(actual: Pixels, expected: Pixels) {
    let delta = (actual.as_f32() - expected.as_f32()).abs();
    assert!(
        delta <= 1.0,
        "窗格宽度应为 {expected:?},实际 {actual:?}(允许 1px 误差)"
    );
}

/// 回归测试:切到别的标签页再切回来,已经调整过的窗格尺寸不能被重置成默认均分。
#[gpui::test]
fn pane_sizes_survive_tab_switch(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (harness, visual) = cx.add_window_view(|_, _| SplitHarness::new(split_three_panes()));

    draw(visual);
    assert_width(pane_width(visual, PANE_SELECTORS[0]), px(300.));
    assert_width(pane_width(visual, PANE_SELECTORS[1]), px(300.));
    assert_width(pane_width(visual, PANE_SELECTORS[2]), px(300.));

    let state = harness.update(visual, |harness, cx| harness.root_split(cx));
    visual.update(|window, cx| {
        state.update(cx, |state, cx| state.resize_panel(0, px(200.), window, cx))
    });
    draw(visual);
    assert_width(pane_width(visual, PANE_SELECTORS[0]), px(200.));
    assert_width(pane_width(visual, PANE_SELECTORS[1]), px(400.));

    // 切走:分屏元素树被整体卸载。
    harness.update(visual, |harness, cx| {
        harness.show_split = false;
        cx.notify();
    });
    draw(visual);
    assert!(
        visual.debug_bounds(PANE_SELECTORS[0]).is_none(),
        "切走后分屏元素应已卸载"
    );
    assert!(visual.debug_bounds("other-tab").is_some());

    // 切回:尺寸保持用户调整后的值,而不是回到 300/300/300。
    harness.update(visual, |harness, cx| {
        harness.show_split = true;
        cx.notify();
    });
    draw(visual);
    assert_width(pane_width(visual, PANE_SELECTORS[0]), px(200.));
    assert_width(pane_width(visual, PANE_SELECTORS[1]), px(400.));
    assert_width(pane_width(visual, PANE_SELECTORS[2]), px(300.));
    visual.read(|cx| {
        let sizes = state.read(cx).sizes().clone();
        assert_eq!(vec![px(200.), px(400.), px(300.)], sizes);
    });
}

/// 关闭一个窗格后,存活窗格间的比例应保持不变,而不是重新均分。
#[gpui::test]
fn closing_pane_preserves_surviving_ratio(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (harness, visual) = cx.add_window_view(|_, _| SplitHarness::new(split_three_panes()));

    draw(visual);
    let state = harness.update(visual, |harness, cx| harness.root_split(cx));
    visual.update(|window, cx| {
        state.update(cx, |state, cx| state.resize_panel(0, px(180.), window, cx))
    });
    draw(visual);
    assert_width(pane_width(visual, PANE_SELECTORS[0]), px(180.));
    assert_width(pane_width(visual, PANE_SELECTORS[1]), px(420.));

    harness.update(visual, |harness, cx| {
        harness.tree.remove(TerminalPaneId::new(3));
        cx.notify();
    });
    draw(visual);

    // 关闭 180/420/300 里的第三个窗格后,剩下的 180:420 仍按 3:7 分掉 900。
    assert_width(pane_width(visual, PANE_SELECTORS[0]), px(270.));
    assert_width(pane_width(visual, PANE_SELECTORS[1]), px(630.));
    assert_eq!(2, visual.read(|cx| harness.read(cx).tree.panes().len()));
}

/// 侧栏拖动换算依赖 `workspace_bounds`。它必须来自工作区根元素的真实
/// bounds:事件监听元素是 0×0 的弹性子项,拿它自己的 bounds 会让左侧
/// 与底部拖动永远被夹到最小宽度(左侧面板拖不动)。
#[test]
fn sidebar_resize_bounds_come_from_the_workspace_root() {
    let layout_source = include_str!("layout.rs");
    let resize_source = include_str!("resize.rs");

    assert!(
        layout_source.contains(".on_prepaint(move |bounds, _, cx|"),
        "workspace root must capture its own bounds via on_prepaint"
    );
    assert!(
        layout_source.contains("workspace.workspace_bounds = bounds"),
        "workspace root prepaint must write workspace_bounds"
    );
    assert!(
        !resize_source.contains("workspace_bounds = bounds"),
        "resize event handler is zero-sized; its bounds must not be used as the workspace bounds"
    );
}
