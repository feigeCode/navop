use super::{SidebarPanel, TerminalSidebar};
use crate::theme::TerminalColors;
use gpui::{
    Anchor, AnyElement, Context, Entity, InteractiveElement, IntoElement, ParentElement, Pixels,
    SharedString, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    IconName, IconSize, Sizable,
    button::IconButton,
    h_flex,
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    panel_header::PanelHeader,
    v_flex,
};
use one_core::layout::TOOLBAR_WIDTH;
use one_core::sidebar_contribution::SidebarPlacement;
use rust_i18n::t;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TerminalToolDockLayout {
    pub(crate) left: Option<SidebarPanel>,
    pub(crate) right: Option<SidebarPanel>,
    pub(crate) bottom: Option<SidebarPanel>,
}

impl TerminalToolDockLayout {
    pub(crate) fn from_open_panels(
        open_panels: impl IntoIterator<Item = (SidebarPanel, SidebarPlacement)>,
    ) -> Self {
        let mut layout = Self::default();
        for (panel, placement) in open_panels {
            match placement {
                SidebarPlacement::Left => layout.left = Some(panel),
                SidebarPlacement::Right => layout.right = Some(panel),
                SidebarPlacement::Bottom => layout.bottom = Some(panel),
            }
        }
        layout
    }

    pub(crate) fn has_right(&self) -> bool {
        self.right.is_some()
    }
}

pub(crate) fn right_tool_region_width(
    layout: &TerminalToolDockLayout,
    panel_size: Pixels,
) -> Pixels {
    if layout.has_right() {
        panel_size + TOOLBAR_WIDTH
    } else {
        TOOLBAR_WIDTH
    }
}

pub(crate) fn right_sidebar_width(outer_right: Pixels, mouse_x: Pixels) -> Pixels {
    outer_right - TOOLBAR_WIDTH - mouse_x
}

pub(crate) fn render_internal_tool_panel_frame(
    sidebar: Entity<TerminalSidebar>,
    panel: SidebarPanel,
    placement: SidebarPlacement,
    content: impl IntoElement,
    colors: TerminalColors,
    panel_header: Pixels,
) -> AnyElement {
    let needs_header = panel.needs_internal_tool_frame_header();

    v_flex()
        .debug_selector(move || format!("terminal-internal-tool-panel-{}", panel.local_id()))
        .relative()
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .bg(colors.background)
        .border_1()
        .border_color(colors.border)
        .when(needs_header, |this| {
            this.child(render_internal_tool_panel_header(
                sidebar,
                panel,
                placement,
                colors,
                panel_header,
            ))
        })
        .child(
            div()
                .debug_selector(|| "terminal-tool-panel-content".to_string())
                .flex_1()
                .when(needs_header, |this| this.pt(panel_header))
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .child(content),
        )
        .into_any_element()
}

fn render_internal_tool_panel_header(
    sidebar: Entity<TerminalSidebar>,
    panel: SidebarPanel,
    placement: SidebarPlacement,
    colors: TerminalColors,
    panel_header: Pixels,
) -> AnyElement {
    let title = panel.title();
    PanelHeader::new(SharedString::from(format!(
        "terminal-tool-header-{}",
        panel.local_id()
    )))
    .absolute()
    .top_0()
    .left_0()
    .right_0()
    .height(panel_header)
    .background(colors.muted)
    .border_color(colors.border)
    .leading(
        panel
            .icon()
            .with_size(IconSize::Small)
            .flex_shrink_0()
            .text_color(colors.foreground),
    )
    .title(
        div()
            .flex_1()
            .min_w_0()
            .truncate()
            .text_color(colors.foreground)
            .child(title),
    )
    .trailing(
        h_flex()
            .child(options_button(sidebar.clone(), panel, placement))
            .child(close_button(sidebar, panel)),
    )
    .into_any_element()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ToolPanelMoveMenuOption {
    placement: SidebarPlacement,
    disabled: bool,
}

fn tool_panel_move_menu_options(current: SidebarPlacement) -> Vec<ToolPanelMoveMenuOption> {
    [
        SidebarPlacement::Left,
        SidebarPlacement::Right,
        SidebarPlacement::Bottom,
    ]
    .into_iter()
    .map(|placement| ToolPanelMoveMenuOption {
        placement,
        disabled: placement == current,
    })
    .collect()
}

fn placement_label(placement: SidebarPlacement) -> SharedString {
    match placement {
        SidebarPlacement::Left => t!("Sidebar.placement_left"),
        SidebarPlacement::Right => t!("Sidebar.placement_right"),
        SidebarPlacement::Bottom => t!("Sidebar.placement_bottom"),
    }
    .into()
}

fn placement_icon(placement: SidebarPlacement) -> IconName {
    match placement {
        SidebarPlacement::Left => IconName::PanelLeft,
        SidebarPlacement::Right => IconName::PanelRight,
        SidebarPlacement::Bottom => IconName::PanelBottom,
    }
}

fn options_button(
    sidebar: Entity<TerminalSidebar>,
    panel: SidebarPanel,
    placement: SidebarPlacement,
) -> impl IntoElement {
    IconButton::new(
        SharedString::from(format!("terminal-tool-options-{}", panel.local_id())),
        IconName::Ellipsis,
    )
    .tooltip(t!("Sidebar.panel_options").to_string())
    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
        build_options_menu(menu, sidebar.clone(), panel, placement, window, cx)
    })
}

fn close_button(sidebar: Entity<TerminalSidebar>, panel: SidebarPanel) -> IconButton {
    IconButton::new(
        SharedString::from(format!("terminal-tool-close-{}", panel.local_id())),
        IconName::Close,
    )
    .tooltip(t!("Sidebar.close_panel").to_string())
    .on_click(move |_, _window, cx| {
        sidebar.update(cx, |sidebar, cx| sidebar.close_tool(panel, cx));
    })
}

fn build_options_menu(
    menu: PopupMenu,
    sidebar: Entity<TerminalSidebar>,
    panel: SidebarPanel,
    placement: SidebarPlacement,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let move_sidebar = sidebar.clone();
    let remove_sidebar = sidebar.clone();

    menu.min_w(px(220.0))
        .submenu_with_icon(
            Some(IconName::PanelRight.into()),
            t!("Sidebar.move_to").to_string(),
            window,
            cx,
            move |submenu, _window, _cx| {
                tool_panel_move_menu_options(placement).into_iter().fold(
                    submenu,
                    |submenu, option| {
                        let sidebar = move_sidebar.clone();
                        submenu.item(
                            PopupMenuItem::new(placement_label(option.placement))
                                .icon(placement_icon(option.placement))
                                .checked(option.disabled)
                                .disabled(option.disabled)
                                .on_click(move |_, _, cx| {
                                    sidebar.update(cx, |sidebar, cx| {
                                        sidebar.move_tool(panel, option.placement, cx);
                                    });
                                }),
                        )
                    },
                )
            },
        )
        .separator()
        .item(
            PopupMenuItem::new(t!("Sidebar.remove_from_sidebar").to_string())
                .icon(IconName::Close)
                .on_click(move |_, _, cx| {
                    remove_sidebar.update(cx, |sidebar, cx| {
                        sidebar.close_tool(panel, cx);
                    });
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn layout_maps_open_panels_to_edges() {
        let layout = TerminalToolDockLayout::from_open_panels([
            (SidebarPanel::Settings, SidebarPlacement::Left),
            (SidebarPanel::AiChat, SidebarPlacement::Bottom),
            (SidebarPanel::FileManager, SidebarPlacement::Right),
        ]);

        assert_eq!(Some(SidebarPanel::Settings), layout.left);
        assert_eq!(Some(SidebarPanel::FileManager), layout.right);
        assert_eq!(Some(SidebarPanel::AiChat), layout.bottom);
    }

    #[test]
    fn right_region_keeps_toolbar_width_without_right_panel() {
        let layout = TerminalToolDockLayout::from_open_panels([(
            SidebarPanel::Settings,
            SidebarPlacement::Left,
        )]);

        assert_eq!(TOOLBAR_WIDTH, right_tool_region_width(&layout, px(420.0)));
    }

    #[test]
    fn right_region_includes_panel_and_toolbar_when_right_panel_is_open() {
        let layout = TerminalToolDockLayout::from_open_panels([(
            SidebarPanel::AiChat,
            SidebarPlacement::Right,
        )]);

        assert_eq!(
            px(420.0) + TOOLBAR_WIDTH,
            right_tool_region_width(&layout, px(420.0))
        );
    }

    #[test]
    fn right_sidebar_resize_excludes_toolbar_width() {
        assert_eq!(px(256.0), right_sidebar_width(px(1000.0), px(700.0)));
    }

    #[test]
    fn layout_preserves_all_three_edges() {
        let layout = TerminalToolDockLayout::from_open_panels([
            (SidebarPanel::Settings, SidebarPlacement::Left),
            (SidebarPanel::AiChat, SidebarPlacement::Right),
            (SidebarPanel::HistoryCommand, SidebarPlacement::Bottom),
        ]);

        assert_eq!(Some(SidebarPanel::Settings), layout.left);
        assert_eq!(Some(SidebarPanel::AiChat), layout.right);
        assert_eq!(Some(SidebarPanel::HistoryCommand), layout.bottom);
    }

    #[test]
    fn move_menu_options_disable_current_placement() {
        let options = tool_panel_move_menu_options(SidebarPlacement::Right);

        assert_eq!(
            vec![
                (SidebarPlacement::Left, false),
                (SidebarPlacement::Right, true),
                (SidebarPlacement::Bottom, false),
            ],
            options
                .iter()
                .map(|option| (option.placement, option.disabled))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn internal_tool_panel_title_claims_remaining_header_width() {
        let source = include_str!("tool_dock.rs");
        let header_start = source
            .find("fn render_internal_tool_panel_header")
            .expect("internal tool panel header renderer");
        let options_start = source[header_start..]
            .find("fn options_button")
            .map(|offset| header_start + offset)
            .expect("tool panel options button");
        let header_source = &source[header_start..options_start];

        assert!(
            header_source.contains(
                ".title(\n        div()\n            .flex_1()\n            .min_w_0()\n            .truncate()"
            ),
            "rich panel titles must claim the remaining header width before truncating"
        );
    }
}
