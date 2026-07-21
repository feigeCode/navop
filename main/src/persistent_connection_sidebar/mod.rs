use std::collections::HashSet;

use gpui::prelude::FluentBuilder as _;
use gpui::{Context, Entity, IntoElement, ParentElement, Render, Styled, Window};
use gpui_component::h_flex;

use crate::home_tab::HomePage;

mod rail;
mod rows;
mod tree;
mod tree_model;

pub(crate) struct PersistentConnectionSidebar {
    pub(super) home_page: Entity<HomePage>,
    pub(super) tree_expanded: bool,
    pub(super) collapsed_workspaces: HashSet<i64>,
    pub(super) unassigned_collapsed: bool,
}

impl PersistentConnectionSidebar {
    pub(crate) fn new(
        home_page: Entity<HomePage>,
        tree_expanded: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&home_page, |_, _, cx| cx.notify()).detach();
        Self {
            home_page,
            tree_expanded,
            collapsed_workspaces: HashSet::new(),
            unassigned_collapsed: false,
        }
    }

    pub(crate) fn set_tree_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        if self.tree_expanded != expanded {
            self.tree_expanded = expanded;
            cx.notify();
        }
    }
}

impl Render for PersistentConnectionSidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .h_full()
            .flex_shrink_0()
            .child(rail::render_navigation_rail(&self.home_page, cx))
            .when(self.tree_expanded, |this| {
                this.child(self.render_connection_tree(window, cx))
            })
    }
}
