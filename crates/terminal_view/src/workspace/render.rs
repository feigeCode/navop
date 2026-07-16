use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Axis, Context, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Render, SharedString, Styled as _, Window, div, px,
};
use gpui_component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_component::{ActiveTheme as _, v_flex};
use one_core::sidebar_contribution::SidebarPlacement;
use one_core::tab_container::TabContent as _;

use super::{TerminalPaneId, TerminalSplitNode, TerminalWorkspace};
use crate::sidebar::SidebarPanel;
use crate::sidebar::tool_dock::render_internal_tool_panel_frame;
use crate::view::TerminalWorkspaceSidebarSnapshot;

const TERMINAL_PANE_MIN_WIDTH: Pixels = px(240.0);
const TERMINAL_PANE_MIN_HEIGHT: Pixels = px(160.0);
impl TerminalWorkspace {
    pub(super) fn render_node(
        &self,
        node: &TerminalSplitNode,
        path: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            TerminalSplitNode::Pane { pane_id } => self.render_pane(*pane_id, cx),
            TerminalSplitNode::Group {
                split_id,
                axis,
                children,
            } => {
                let id =
                    SharedString::from(format!("terminal-split-group-{}-{path}", split_id.value()));
                let mut group = if *axis == Axis::Horizontal {
                    h_resizable(id)
                } else {
                    v_resizable(id)
                };
                for (index, child) in children.iter().enumerate() {
                    let child_path = format!("{path}-{index}");
                    let min_size = if *axis == Axis::Horizontal {
                        TERMINAL_PANE_MIN_WIDTH
                    } else {
                        TERMINAL_PANE_MIN_HEIGHT
                    };
                    group = group.child(
                        resizable_panel()
                            .size_range(min_size..Pixels::MAX)
                            .child(self.render_node(child, &child_path, cx)),
                    );
                }
                group.into_any_element()
            }
        }
    }

    fn render_pane(&self, pane_id: TerminalPaneId, cx: &mut Context<Self>) -> AnyElement {
        let Some(pane) = self.panes.get(&pane_id).cloned() else {
            return div().into_any_element();
        };
        let active = self.active_pane_id == pane_id;
        let split = self.panes.len() > 1;
        let title = pane.read(cx).title(cx);
        let border = if active {
            cx.theme().drag_border
        } else {
            cx.theme().border
        };

        let content = v_flex()
            .id(("terminal-workspace-pane", pane_id.value()))
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .when(split, |this| this.border_1().border_color(border))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .child(pane),
            )
            .when(split, |this| {
                this.child(self.render_pane_floating_tool(pane_id, title, cx))
            })
            .into_any_element();
        self.render_tab_drop_target(pane_id, content, cx)
    }

    pub(super) fn render_tool_panel(
        &self,
        snapshot: &TerminalWorkspaceSidebarSnapshot,
        panel: SidebarPanel,
        placement: SidebarPlacement,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(content) = snapshot.panels.get(&panel).cloned() else {
            return div().into_any_element();
        };
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(div().flex_1().min_h_0().overflow_hidden().child(
                render_internal_tool_panel_frame(
                    snapshot.sidebar.clone(),
                    panel,
                    placement,
                    content,
                    snapshot.colors.clone(),
                ),
            ))
            .into_any_element()
    }
}

impl Render for TerminalWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target = self.active_pane_id;
        let snapshot = self
            .panes
            .get(&target)
            .map(|pane| pane.read(cx).workspace_sidebar_snapshot(cx));
        let layout = snapshot
            .as_ref()
            .map(|snapshot| snapshot.layout.clone())
            .unwrap_or_default();
        self.render_workspace(layout, snapshot, cx)
    }
}
