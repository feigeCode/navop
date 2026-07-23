use std::collections::HashSet;

use gpui::{
    AppContext, Context, Entity, Hsla, IntoElement, ParentElement, Pixels, Render, Styled,
    UniformListScrollHandle, Window,
};
use gpui_component::{
    h_flex,
    input::{InputEvent, InputState},
};
use terminal_view::TerminalColors;

use crate::home_tab::HomePage;

mod connection_command;
mod connection_context_menu;
mod connection_copy;
mod connection_copy_menu;
mod connection_share;
mod context_menu;
mod drag;
mod rail;
mod resize;
#[cfg(test)]
mod resize_contract_tests;
mod row_parts;
mod rows;
mod tree;
mod tree_model;
mod workspace_context_menu;

#[derive(Clone, Copy)]
pub(super) struct SidebarPalette {
    pub background: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub muted_foreground: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub accent_foreground: Hsla,
}

impl SidebarPalette {
    fn app(cx: &gpui::App) -> Self {
        use gpui_component::ActiveTheme as _;

        Self {
            background: cx.theme().sidebar,
            foreground: cx.theme().foreground,
            muted: cx.theme().sidebar_accent,
            muted_foreground: cx.theme().muted_foreground,
            border: cx.theme().border,
            accent: cx.theme().accent,
            accent_foreground: cx.theme().accent_foreground,
        }
    }
}

impl From<&TerminalColors> for SidebarPalette {
    fn from(colors: &TerminalColors) -> Self {
        Self {
            background: colors.background,
            foreground: colors.foreground,
            muted: colors.muted,
            muted_foreground: colors.muted_foreground,
            border: colors.border,
            accent: colors.accent,
            accent_foreground: colors.accent_foreground,
        }
    }
}

pub(crate) struct PersistentConnectionSidebar {
    pub(super) home_page: Entity<HomePage>,
    pub(super) tree_expanded: bool,
    pub(super) collapsed_workspaces: HashSet<i64>,
    pub(super) unassigned_collapsed: bool,
    pub(super) search_input: Entity<InputState>,
    tree_width: Pixels,
    terminal_colors: Option<TerminalColors>,
    pub(super) tree_scroll_handle: UniformListScrollHandle,
}

impl PersistentConnectionSidebar {
    pub(crate) fn new(
        home_page: Entity<HomePage>,
        tree_expanded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&home_page, |_, _, cx| cx.notify()).detach();
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("Connection.search_placeholder").to_string())
                .clean_on_escape()
        });
        cx.subscribe_in(&search_input, window, |_, _, event, _, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
        })
        .detach();
        Self {
            home_page,
            tree_expanded,
            collapsed_workspaces: HashSet::new(),
            unassigned_collapsed: false,
            search_input,
            tree_width: resize::CONNECTION_TREE_DEFAULT_WIDTH,
            terminal_colors: None,
            tree_scroll_handle: UniformListScrollHandle::new(),
        }
    }

    pub(crate) fn set_tree_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        if self.tree_expanded != expanded {
            self.tree_expanded = expanded;
            cx.notify();
        }
    }

    pub(crate) fn is_expanded(&self) -> bool {
        self.tree_expanded
    }

    pub(crate) fn set_terminal_colors(
        &mut self,
        colors: Option<TerminalColors>,
        cx: &mut Context<Self>,
    ) {
        self.terminal_colors = colors;
        cx.notify();
    }

    pub(super) fn palette(&self, cx: &gpui::App) -> SidebarPalette {
        self.terminal_colors
            .as_ref()
            .map(SidebarPalette::from)
            .unwrap_or_else(|| SidebarPalette::app(cx))
    }
}

impl Render for PersistentConnectionSidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette(cx);
        h_flex()
            .h_full()
            .flex_shrink_0()
            .child(rail::render_navigation_rail(&self.home_page, palette, cx))
            .child(self.render_connection_tree(palette, window, cx))
    }
}
