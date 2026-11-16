use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div,
};
use gpui_component::{ElementExt as _, h_flex, v_flex};
use one_core::layout::TOOLBAR_WIDTH;
use one_core::sidebar_contribution::SidebarPlacement;

use super::TerminalWorkspace;
use super::resize::{WorkspaceResizeEventHandler, WorkspaceSidebarResize};
use crate::sidebar::tool_dock::TerminalToolDockLayout;
use crate::view::TerminalWorkspaceSidebarSnapshot;

impl TerminalWorkspace {
    pub(super) fn render_workspace(
        &mut self,
        layout: TerminalToolDockLayout,
        snapshot: Option<TerminalWorkspaceSidebarSnapshot>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // 结构变化(增删窗格)后先把已保存的分隔比例搬到新的槽位,
        // 再渲染,避免尺寸错位。
        self.split_sizing.reconcile(&self.split_tree, cx);
        let root = self.split_tree.root().clone();
        let split_content = self.render_node(&root, "root", cx);
        let panel_size = self.sidebar_panel_size;
        // 扩展面板与内建面板共用三边槽位：内建面板优先，扩展面板只补内建
        // 面板未占用的边（与内嵌模式同一套规则）。工作区分屏里同样要显示，
        // 否则「把终端弹成独立窗格」之后扩展面板会静默消失。
        let extension_layout = snapshot
            .as_ref()
            .map(|snapshot| snapshot.extension_layout.clone())
            .unwrap_or_default();
        let left = snapshot.as_ref().and_then(|snapshot| {
            layout
                .left
                .map(|panel| self.render_tool_panel(snapshot, panel, SidebarPlacement::Left, cx))
                .or_else(|| {
                    extension_layout.left.clone().map(|id| {
                        self.render_extension_tool_panel(snapshot, id, SidebarPlacement::Left, cx)
                    })
                })
        });
        let right = snapshot.as_ref().and_then(|snapshot| {
            layout
                .right
                .map(|panel| self.render_tool_panel(snapshot, panel, SidebarPlacement::Right, cx))
                .or_else(|| {
                    extension_layout.right.clone().map(|id| {
                        self.render_extension_tool_panel(snapshot, id, SidebarPlacement::Right, cx)
                    })
                })
        });
        let bottom = snapshot.as_ref().and_then(|snapshot| {
            layout
                .bottom
                .map(|panel| self.render_tool_panel(snapshot, panel, SidebarPlacement::Bottom, cx))
                .or_else(|| {
                    extension_layout.bottom.clone().map(|id| {
                        self.render_extension_tool_panel(snapshot, id, SidebarPlacement::Bottom, cx)
                    })
                })
        });
        let has_right = layout.right.is_some() || extension_layout.right.is_some();
        let right_width = snapshot
            .as_ref()
            .map(|_| {
                // 与内嵌模式一致：宽度按“合并后的右栏”算，只被扩展面板占用的
                // 边也要留出面板宽度（否则面板被压进 44px 工具条）。
                if has_right {
                    panel_size + TOOLBAR_WIDTH
                } else {
                    TOOLBAR_WIDTH
                }
            })
            .unwrap_or_default();

        let center = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .child(split_content),
            )
            .when_some(bottom, |this, panel| {
                this.child(
                    div()
                        .relative()
                        .w_full()
                        .h(panel_size)
                        .min_h(panel_size)
                        .max_h(panel_size)
                        .flex_shrink_0()
                        .overflow_hidden()
                        .child(
                            self.render_sidebar_resize_handle(WorkspaceSidebarResize::Bottom, cx),
                        )
                        .child(panel),
                )
            });

        let workspace = cx.entity();
        h_flex()
            .id("terminal-workspace-root")
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            // 侧栏拖动换算要用工作区根元素的真实 bounds。事件监听元素本身是
            // 0×0 的弹性子项,拿它自己的 bounds 当工作区边界会让左/下拖动
            // 永远被夹到最小宽度。
            .on_prepaint(move |bounds, _, cx| {
                workspace.update(cx, |workspace, _| {
                    workspace.workspace_bounds = bounds;
                });
            })
            .when_some(left, |this, panel| {
                this.child(
                    div()
                        .relative()
                        .h_full()
                        .w(panel_size)
                        .min_w(panel_size)
                        .max_w(panel_size)
                        .flex_shrink_0()
                        .overflow_hidden()
                        .child(self.render_sidebar_resize_handle(WorkspaceSidebarResize::Left, cx))
                        .child(panel),
                )
            })
            .child(center)
            .when_some(snapshot, |this, snapshot| {
                this.child(
                    h_flex()
                        .h_full()
                        .w(right_width)
                        .min_w(right_width)
                        .max_w(right_width)
                        .flex_shrink_0()
                        .overflow_hidden()
                        .when_some(right, |this, panel| {
                            this.child(
                                div()
                                    .relative()
                                    .h_full()
                                    .w(panel_size)
                                    .min_w(panel_size)
                                    .max_w(panel_size)
                                    .flex_shrink_0()
                                    .overflow_hidden()
                                    .child(self.render_sidebar_resize_handle(
                                        WorkspaceSidebarResize::Right,
                                        cx,
                                    ))
                                    .child(panel),
                            )
                        })
                        .child(
                            div()
                                .h_full()
                                .w(TOOLBAR_WIDTH)
                                .min_w(TOOLBAR_WIDTH)
                                .max_w(TOOLBAR_WIDTH)
                                .flex_shrink_0()
                                .child(snapshot.toolbar),
                        ),
                )
            })
            .child(WorkspaceResizeEventHandler {
                workspace: cx.entity(),
            })
            .into_any_element()
    }
}
