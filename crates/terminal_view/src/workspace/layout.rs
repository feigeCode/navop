use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div,
};
use gpui_component::{h_flex, v_flex};
use one_core::layout::TOOLBAR_WIDTH;
use one_core::sidebar_contribution::SidebarPlacement;

use super::TerminalWorkspace;
use super::resize::{WorkspaceResizeEventHandler, WorkspaceSidebarResize};
use crate::sidebar::tool_dock::{TerminalToolDockLayout, right_tool_region_width};
use crate::view::TerminalWorkspaceSidebarSnapshot;

impl TerminalWorkspace {
    pub(super) fn render_workspace(
        &self,
        layout: TerminalToolDockLayout,
        snapshot: Option<TerminalWorkspaceSidebarSnapshot>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let split_content = self.render_node(self.split_tree.root(), "root", cx);
        let panel_size = self.sidebar_panel_size;
        let left = snapshot.as_ref().and_then(|snapshot| {
            layout
                .left
                .map(|panel| self.render_tool_panel(snapshot, panel, SidebarPlacement::Left, cx))
        });
        let right = snapshot.as_ref().and_then(|snapshot| {
            layout
                .right
                .map(|panel| self.render_tool_panel(snapshot, panel, SidebarPlacement::Right, cx))
        });
        let bottom = snapshot.as_ref().and_then(|snapshot| {
            layout
                .bottom
                .map(|panel| self.render_tool_panel(snapshot, panel, SidebarPlacement::Bottom, cx))
        });
        let right_width = snapshot
            .as_ref()
            .map(|_| right_tool_region_width(&layout, panel_size))
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

        h_flex()
            .id("terminal-workspace-root")
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
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
