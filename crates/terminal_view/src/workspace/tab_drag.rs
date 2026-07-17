use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, Styled as _, Window, div, relative,
};
use gpui_component::{ActiveTheme as _, Placement};
use one_core::tab_container::{DragTab, TabContainer, TabContentEvent, TabItem, TabOpenMode};

use super::pane_tab_transfer::TerminalPaneTabMetadata;
use super::{TerminalPaneId, TerminalWorkspace};
use crate::view::TerminalView;

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
        div()
            .id(("terminal-tab-drop-region", pane_id.value()))
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(content)
            .children([
                self.render_tab_drop_zone(pane_id, Placement::Left, cx),
                self.render_tab_drop_zone(pane_id, Placement::Right, cx),
                self.render_tab_drop_zone(pane_id, Placement::Top, cx),
                self.render_tab_drop_zone(pane_id, Placement::Bottom, cx),
            ])
            .into_any_element()
    }

    fn render_tab_drop_zone(
        &self,
        pane_id: TerminalPaneId,
        placement: Placement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = SharedString::from(format!(
            "terminal-tab-drop-zone-{}-{placement:?}",
            pane_id.value()
        ));
        div()
            .id(id)
            .invisible()
            .absolute()
            .bg(cx.theme().drop_target)
            .map(|zone| place_drop_zone(zone, placement))
            .drag_over::<DragTab>(move |zone, _, _, _| show_drop_highlight(zone, placement))
            .on_drop(cx.listener(move |this, drag: &DragTab, window, cx| {
                this.drop_terminal_tab(pane_id, placement, drag, window, cx);
            }))
            .into_any_element()
    }

    fn drop_terminal_tab(
        &mut self,
        pane_id: TerminalPaneId,
        placement: Placement,
        drag: &DragTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.split_tree.contains(pane_id) {
            cx.notify();
            return;
        }
        let target_workspace = cx.entity();

        if drag.is_external() {
            // A split pane's external source can be this same workspace.  Run
            // the detach after the current entity update finishes to avoid a
            // re-entrant `Entity::update` panic, then insert it at the chosen
            // pane edge.
            let drag = drag.clone();
            window.defer(cx, move |window, cx| {
                let Some(tab) = drag.take_external_tab(window, cx) else {
                    return;
                };
                target_workspace.update(cx, |workspace, cx| {
                    workspace.finish_external_tab_drop(pane_id, placement, tab, window, cx);
                });
            });
            cx.notify();
            return;
        }

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

        if !self.insert_pane(pane_id, placement, source.pane, tab_metadata, window, cx) {
            restore_tab(source.container, tab, window, cx);
        }
        cx.notify();
    }

    fn finish_external_tab_drop(
        &mut self,
        pane_id: TerminalPaneId,
        placement: Placement,
        tab: TabItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source_pane = tab
            .content()
            .view()
            .downcast::<TerminalWorkspace>()
            .ok()
            .and_then(|source_workspace| {
                let source_state = source_workspace.read(cx);
                let source_pane_id = source_state.split_tree.transferable_pane()?;
                source_state.panes.get(&source_pane_id).cloned()
            });
        let Some(source_pane) = source_pane else {
            cx.emit(TabContentEvent::OpenTab {
                tab,
                mode: TabOpenMode::Background,
            });
            return;
        };
        let tab_metadata = TerminalPaneTabMetadata::from_tab(&tab);
        if !self.insert_pane(pane_id, placement, source_pane, tab_metadata, window, cx) {
            cx.emit(TabContentEvent::OpenTab {
                tab,
                mode: TabOpenMode::Background,
            });
        }
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

fn place_drop_zone(
    zone: gpui::Stateful<gpui::Div>,
    placement: Placement,
) -> gpui::Stateful<gpui::Div> {
    match placement {
        Placement::Left => zone.left_0().top_0().bottom_0().w(relative(0.25)),
        Placement::Right => zone.right_0().top_0().bottom_0().w(relative(0.25)),
        Placement::Top => zone
            .top_0()
            .left(relative(0.25))
            .w(relative(0.5))
            .h(relative(0.5)),
        Placement::Bottom => zone
            .bottom_0()
            .left(relative(0.25))
            .w(relative(0.5))
            .h(relative(0.5)),
    }
}

fn show_drop_highlight(zone: gpui::StyleRefinement, placement: Placement) -> gpui::StyleRefinement {
    match placement {
        Placement::Left | Placement::Right => zone.visible().w(relative(0.5)),
        Placement::Top | Placement::Bottom => {
            zone.visible().left_0().right_0().w_full().h(relative(0.5))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_pane_renders_four_direct_drop_zones_above_its_content() {
        let source = include_str!("tab_drag.rs");

        let content_index = source.find(".child(content)").expect("pane content");
        let zones_index = source[content_index..]
            .find("self.render_tab_drop_zone")
            .map(|offset| content_index + offset)
            .expect("drop zones");
        assert!(content_index < zones_index);
        for placement in ["Left", "Right", "Top", "Bottom"] {
            assert!(source.contains(&format!("Placement::{placement}, cx")));
        }
        assert!(source.contains("show_drop_highlight(zone, placement)"));
        assert!(source.contains("this.drop_terminal_tab(pane_id, placement"));
    }

    #[test]
    fn drop_highlight_expands_to_half_of_the_target_pane() {
        let source = include_str!("tab_drag.rs");

        assert!(
            source
                .contains("Placement::Left | Placement::Right => zone.visible().w(relative(0.5))")
        );
        assert!(source.contains("zone.visible().left_0().right_0().w_full().h(relative(0.5))"));
    }

    #[test]
    fn pane_title_drag_is_accepted_by_direct_split_drop_zones() {
        let source = include_str!("tab_drag.rs");

        assert!(source.contains("if drag.is_external()"));
        assert!(source.contains("window.defer(cx"));
        assert!(source.contains("drag.take_external_tab(window, cx)"));
        assert!(source.contains("finish_external_tab_drop"));
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
