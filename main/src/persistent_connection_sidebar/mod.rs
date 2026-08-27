use gpui::{
    AnyElement, AppContext, Context, Entity, EventEmitter, Hsla, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, Styled, UniformListScrollHandle, Window, div,
};
use gpui_component::{
    ActiveTheme as _, h_flex,
    input::{InputEvent, InputState},
};
use terminal_view::TerminalColors;

use crate::home_tab::HomePage;
use selection::ConnectionSelection;

mod batch_toolbar;
mod connection_command;
mod connection_context_menu;
mod connection_copy;
mod connection_copy_menu;
mod connection_share;
mod context_menu;
mod drag;
mod header_actions;
mod navigation_sections;
mod rail;
mod resize;
#[cfg(test)]
mod resize_contract_tests;
mod row_parts;
mod rows;
mod selection;
mod state;
mod tree;
mod tree_model;
mod workspace_context_menu;

pub(crate) use connection_share::connection_full_info_text;

#[derive(Clone, Copy)]
pub(super) struct SidebarPalette {
    pub background: Hsla,
    pub rail_background: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub selected_border: Hsla,
    pub muted_foreground: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
}

impl SidebarPalette {
    fn app(cx: &gpui::App) -> Self {
        use gpui_component::ActiveTheme as _;

        let background = cx.theme().sidebar;
        Self {
            background,
            rail_background: shade(background, cx.theme().is_dark()),
            foreground: cx.theme().sidebar_foreground,
            muted: cx.theme().sidebar_accent,
            hover: cx.theme().sidebar_accent,
            selected: cx.theme().list_active,
            selected_border: cx.theme().list_active_border,
            muted_foreground: cx.theme().muted_foreground,
            border: cx.theme().sidebar_border,
            accent: cx.theme().sidebar_primary,
        }
    }
}

impl From<&TerminalColors> for SidebarPalette {
    fn from(colors: &TerminalColors) -> Self {
        Self {
            background: colors.background,
            rail_background: shade(colors.background, true),
            foreground: colors.foreground,
            muted: colors.muted,
            hover: colors.muted,
            selected: Hsla::new(
                colors.accent.hue.into_degrees(),
                colors.accent.saturation,
                colors.accent.lightness,
                0.18,
            ),
            selected_border: colors.accent,
            muted_foreground: colors.muted_foreground,
            border: colors.border,
            accent: colors.accent,
        }
    }
}

/// Slightly darken a background color so the navigation rail stays visually
/// distinct from the connection panel without turning nearly black in dark
/// terminal themes.
fn shade(color: Hsla, dark_mode: bool) -> Hsla {
    let amount = if dark_mode { -0.02 } else { -0.015 };
    Hsla::new(
        color.hue.into_degrees(),
        color.saturation,
        (color.lightness + amount).clamp(0.0, 1.0),
        color.alpha,
    )
}

pub(crate) struct PersistentConnectionSidebar {
    pub(super) home_page: Entity<HomePage>,
    connection_selection: ConnectionSelection,
    pub(super) tree_expanded: bool,
    pub(super) hide_empty_workspaces: bool,
    pub(super) auto_hide_tree: bool,
    pub(super) search_input: Entity<InputState>,
    tree_width: Pixels,
    terminal_colors: Option<TerminalColors>,
    pub(super) tree_scroll_handle: UniformListScrollHandle,
}

pub(crate) enum PersistentConnectionSidebarEvent {
    TreeVisibilityChanged { expanded: bool },
}

impl EventEmitter<PersistentConnectionSidebarEvent> for PersistentConnectionSidebar {}

impl PersistentConnectionSidebar {
    /// Render the connection tree as a floating panel that overlays the main
    /// content instead of occupying flex space, so expanding it no longer
    /// squeezes the terminal. The caller (OnetCliApp) positions it just after
    /// the navigation rail and above the terminal, and collapses it when the
    /// terminal regains focus.
    pub(crate) fn render_floating_tree(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette(cx);
        let layout = cx.theme().geometry.layout;
        let rail_width = layout.global_rail;
        let top = layout.tab_bar;
        div()
            .absolute()
            .top(top)
            .bottom_0()
            .left(rail_width)
            .w(self.tree_width)
            // 浮动侧边栏覆盖在内容区之上：occlude 让命中测试在侧边栏处终止，
            // 避免滚轮/鼠标事件穿透到下方的 tab 内容区（否则终端等会跟着滚动）。
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.render_connection_tree(palette, window, cx))
            .into_any_element()
    }

    pub(crate) fn new(
        home_page: Entity<HomePage>,
        tree_expanded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&home_page, |this, home, cx| {
            let home = home.read(cx);
            let valid_ids = home
                .connections
                .iter()
                .filter_map(|connection| {
                    let id = connection.id?;
                    home.can_move_connection(id).then_some(id)
                })
                .collect();
            this.connection_selection.retain(&valid_ids);
            cx.notify();
        })
        .detach();
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
        let tree_state = one_core::settings::AppSettings::current(cx).connection_sidebar_tree_state;
        Self {
            home_page,
            connection_selection: ConnectionSelection::default(),
            tree_expanded,
            hide_empty_workspaces: tree_state.hide_empty_workspaces,
            auto_hide_tree: tree_state.auto_hide_tree,
            search_input,
            tree_width: cx.theme().geometry.layout.context_sidebar_default,
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

    pub(crate) fn is_auto_hide_tree(&self) -> bool {
        self.auto_hide_tree
    }

    /// 非自动隐藏模式下，连接树作为与终端并排的分割面板渲染，而不是浮层。
    pub(crate) fn render_docked_connection_tree(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_connection_tree(self.palette(cx), window, cx)
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette(cx);
        h_flex()
            .h_full()
            .flex_shrink_0()
            .child(rail::render_navigation_rail(
                &self.home_page,
                cx.entity(),
                palette,
                cx,
            ))
            .into_any_element()
    }
}
