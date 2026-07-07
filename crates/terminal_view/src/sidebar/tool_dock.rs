use super::{SidebarPanel, TerminalSidebar};
use crate::theme::TerminalColors;
use gpui::{
    AnyElement, Entity, InteractiveElement, IntoElement, ParentElement, Pixels, SharedString,
    Styled, div, px,
};
use gpui_component::{
    Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use one_core::layout::TOOLBAR_WIDTH;
use one_core::sidebar_contribution::SidebarPlacement;

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

pub(crate) fn render_internal_tool_panel_frame(
    sidebar: Entity<TerminalSidebar>,
    panel: SidebarPanel,
    content: impl IntoElement,
    colors: TerminalColors,
) -> AnyElement {
    v_flex()
        .debug_selector(move || format!("terminal-internal-tool-panel-{}", panel.local_id()))
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .bg(colors.background)
        .border_1()
        .border_color(colors.border)
        .child(render_internal_tool_panel_header(sidebar, panel, colors))
        .child(
            div()
                .debug_selector(|| "terminal-tool-panel-content".to_string())
                .flex_1()
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
    colors: TerminalColors,
) -> AnyElement {
    let title: SharedString = panel.title().into();
    h_flex()
        .h(px(34.0))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .px_2()
        .bg(colors.muted)
        .border_b_1()
        .border_color(colors.border)
        .child(
            Icon::new(panel.icon_name())
                .with_size(Size::Small)
                .text_color(colors.foreground),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(colors.foreground)
                .child(title),
        )
        .child(move_button(
            sidebar.clone(),
            panel,
            SidebarPlacement::Left,
            IconName::PanelLeft,
        ))
        .child(move_button(
            sidebar.clone(),
            panel,
            SidebarPlacement::Right,
            IconName::PanelRight,
        ))
        .child(move_button(
            sidebar.clone(),
            panel,
            SidebarPlacement::Bottom,
            IconName::PanelBottom,
        ))
        .child(close_button(sidebar, panel))
        .into_any_element()
}

fn move_button(
    sidebar: Entity<TerminalSidebar>,
    panel: SidebarPanel,
    placement: SidebarPlacement,
    icon: IconName,
) -> Button {
    Button::new(SharedString::from(format!(
        "terminal-tool-move-{placement:?}-{}",
        panel.local_id()
    )))
    .icon(icon)
    .ghost()
    .compact()
    .on_click(move |_, _window, cx| {
        sidebar.update(cx, |sidebar, cx| sidebar.move_tool(panel, placement, cx));
    })
}

fn close_button(sidebar: Entity<TerminalSidebar>, panel: SidebarPanel) -> Button {
    Button::new(SharedString::from(format!(
        "terminal-tool-close-{}",
        panel.local_id()
    )))
    .icon(IconName::Close)
    .ghost()
    .compact()
    .on_click(move |_, _window, cx| {
        sidebar.update(cx, |sidebar, cx| sidebar.close_tool(panel, cx));
    })
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
}
