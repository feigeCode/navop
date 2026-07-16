use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, DragMoveEvent, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Styled as _, Window, div, relative,
};
use gpui_component::{ActiveTheme as _, Placement};
use one_core::tab_container::{DragTab, TabContainer, TabItem};

use super::pane_tab_transfer::TerminalPaneTabMetadata;
use super::{TerminalPaneId, TerminalWorkspace};
use crate::view::TerminalView;

const TAB_DROP_GROUP: &str = "terminal-tab-split";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TerminalTabDropTarget {
    pane_id: TerminalPaneId,
    placement: Placement,
}

struct TerminalTabSource {
    container: Entity<TabContainer>,
    workspace: Entity<TerminalWorkspace>,
    pane: Entity<TerminalView>,
}

impl TerminalWorkspace {
    pub(super) fn render_tab_drop_target(
        &self,
        pane_id: TerminalPaneId,
        content: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let placement = self
            .tab_drop_target
            .filter(|target| target.pane_id == pane_id)
            .map(|target| target.placement);
        div()
            .id(("terminal-tab-drop-region", pane_id.value()))
            .group(TAB_DROP_GROUP)
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .on_drag_move(
                cx.listener(move |this, drag: &DragMoveEvent<DragTab>, _, cx| {
                    this.update_tab_drop_target(pane_id, drag, cx);
                }),
            )
            .child(content)
            .child(
                div()
                    .id(("terminal-tab-drop-target", pane_id.value()))
                    .invisible()
                    .absolute()
                    .bg(cx.theme().drop_target)
                    .map(|overlay| place_overlay(overlay, placement))
                    .group_drag_over::<DragTab>(TAB_DROP_GROUP, |overlay| overlay.visible())
                    .on_drop(cx.listener(move |this, drag: &DragTab, window, cx| {
                        this.drop_terminal_tab(pane_id, drag, window, cx);
                    })),
            )
            .into_any_element()
    }

    fn update_tab_drop_target(
        &mut self,
        pane_id: TerminalPaneId,
        drag: &DragMoveEvent<DragTab>,
        cx: &mut Context<Self>,
    ) {
        let target_workspace = cx.entity();
        let transferable = terminal_tab_source(drag.drag(cx), &target_workspace, cx).is_some();
        let placement = transferable
            .then(|| normalized_drop_position(drag))
            .flatten();
        let target = placement.map(|placement| TerminalTabDropTarget { pane_id, placement });
        if self.tab_drop_target != target {
            self.tab_drop_target = target;
            cx.notify();
        }
    }

    fn drop_terminal_tab(
        &mut self,
        pane_id: TerminalPaneId,
        drag: &DragTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.tab_drop_target.take() else {
            return;
        };
        if target.pane_id != pane_id || !self.split_tree.contains(pane_id) {
            cx.notify();
            return;
        }
        let target_workspace = cx.entity();
        let Some(source) = terminal_tab_source(drag, &target_workspace, cx) else {
            cx.notify();
            return;
        };

        let tab_index = drag.tab_index;
        let source_workspace = source.workspace.clone();
        let taken = source.container.update(cx, |container, cx| {
            if !tab_matches_workspace(container, tab_index, &source_workspace) {
                return None;
            }
            container.take_tab(tab_index, window, cx)
        });
        let Some(tab) = taken else {
            cx.notify();
            return;
        };

        let tab_metadata = TerminalPaneTabMetadata::from_tab(&tab);

        if !self.insert_pane(
            pane_id,
            target.placement,
            source.pane,
            tab_metadata,
            window,
            cx,
        ) {
            restore_tab(source.container, tab, window, cx);
        }
        cx.notify();
    }
}

fn terminal_tab_source(
    drag: &DragTab,
    target_workspace: &Entity<TerminalWorkspace>,
    cx: &App,
) -> Option<TerminalTabSource> {
    let container = drag.source_pane.clone()?;
    let workspace = container
        .read(cx)
        .tabs()
        .get(drag.tab_index)?
        .content()
        .view()
        .downcast::<TerminalWorkspace>()
        .ok()?;
    if workspace == *target_workspace {
        return None;
    }
    let source_workspace = workspace.read(cx);
    if source_workspace.panes.len() != 1 {
        return None;
    }
    let pane_id = source_workspace.split_tree.transferable_pane()?;
    let pane = source_workspace.panes.get(&pane_id)?.clone();
    Some(TerminalTabSource {
        container,
        workspace,
        pane,
    })
}

fn tab_matches_workspace(
    container: &TabContainer,
    tab_index: usize,
    workspace: &Entity<TerminalWorkspace>,
) -> bool {
    container
        .tabs()
        .get(tab_index)
        .and_then(|tab| tab.content().view().downcast::<TerminalWorkspace>().ok())
        .is_some_and(|candidate| candidate == *workspace)
}

fn restore_tab(
    container: Entity<TabContainer>,
    tab: TabItem,
    window: &mut Window,
    cx: &mut Context<TerminalWorkspace>,
) {
    container.update(cx, |container, cx| {
        container.insert_tab_at_end_and_activate(tab, window, cx);
    });
}

fn normalized_drop_position(drag: &DragMoveEvent<DragTab>) -> Option<Placement> {
    let width = f32::from(drag.bounds.size.width);
    let height = f32::from(drag.bounds.size.height);
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let x = f32::from(drag.event.position.x - drag.bounds.left()) / width;
    let y = f32::from(drag.event.position.y - drag.bounds.top()) / height;
    drop_placement(x, y)
}

fn drop_placement(x: f32, y: f32) -> Option<Placement> {
    if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
        return None;
    }
    [
        (x, Placement::Left),
        (1.0 - x, Placement::Right),
        (y, Placement::Top),
        (1.0 - y, Placement::Bottom),
    ]
    .into_iter()
    .min_by(|left, right| left.0.total_cmp(&right.0))
    .map(|(_, placement)| placement)
}

fn place_overlay(
    overlay: gpui::Stateful<gpui::Div>,
    placement: Option<Placement>,
) -> gpui::Stateful<gpui::Div> {
    match placement {
        Some(Placement::Left) => overlay.left_0().top_0().bottom_0().w(relative(0.5)),
        Some(Placement::Right) => overlay.right_0().top_0().bottom_0().w(relative(0.5)),
        Some(Placement::Top) => overlay.top_0().left_0().right_0().h(relative(0.5)),
        Some(Placement::Bottom) => overlay.bottom_0().left_0().right_0().h(relative(0.5)),
        None => overlay.top_0().left_0().size_0(),
    }
}

#[cfg(test)]
mod tests {
    use gpui_component::Placement;

    use super::drop_placement;

    #[test]
    fn tab_drop_uses_the_entire_terminal_pane() {
        assert_eq!(Some(Placement::Left), drop_placement(0.35, 0.5));
        assert_eq!(Some(Placement::Bottom), drop_placement(0.49, 0.64));
    }

    #[test]
    fn tab_drop_resolves_all_four_split_directions() {
        assert_eq!(Some(Placement::Left), drop_placement(0.1, 0.5));
        assert_eq!(Some(Placement::Right), drop_placement(0.9, 0.5));
        assert_eq!(Some(Placement::Top), drop_placement(0.5, 0.1));
        assert_eq!(Some(Placement::Bottom), drop_placement(0.5, 0.9));
    }

    #[test]
    fn tab_drop_uses_the_nearest_edge_at_corners() {
        assert_eq!(Some(Placement::Top), drop_placement(0.2, 0.05));
        assert_eq!(Some(Placement::Right), drop_placement(0.95, 0.2));
    }

    #[test]
    fn current_workspace_is_rejected_before_reading_source_state() {
        let source = include_str!("tab_drag.rs");
        let start = source
            .find("fn terminal_tab_source")
            .expect("terminal tab source resolver");
        let end = source[start..]
            .find("\nfn tab_matches_workspace")
            .map(|offset| start + offset)
            .expect("terminal tab source resolver end");
        let resolver = &source[start..end];
        let guard = ["if workspace == ", "*target_workspace"].concat();
        let guard_index = resolver.find(&guard).expect("same workspace guard");
        let read_index = resolver
            .find("let source_workspace = workspace.read(cx)")
            .expect("source workspace read");

        assert!(guard_index < read_index);
    }
}
