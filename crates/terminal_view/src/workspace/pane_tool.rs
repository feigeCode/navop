use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, AppContext as _, ColorExt as _, Context, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, div, px, relative,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::tooltip::Tooltip;
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Placement, Sizable as _, Size, h_flex,
};
use one_core::tab_container::DragTab;
use rust_i18n::t;

use super::{TerminalPaneId, TerminalWorkspace};

impl TerminalWorkspace {
    pub(super) fn render_pane_floating_tool(
        &self,
        pane_id: TerminalPaneId,
        title: SharedString,
        hover_group: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.active_pane_id == pane_id;
        let split = self.panes.len() > 1;
        let split_supported = self
            .panes
            .get(&pane_id)
            .is_some_and(|pane| pane.read(cx).duplicate_supported(cx));
        let workspace = cx.entity().downgrade();
        let drag_source = split
            .then(|| self.external_tab_drag_source(pane_id, workspace.clone()))
            .flatten();
        let background = if active {
            cx.theme().tab_active.opacity(0.94)
        } else {
            cx.theme().tab_bar.opacity(0.86)
        };
        let border = if active {
            cx.theme().drag_border
        } else {
            cx.theme().border
        };

        if !split {
            return h_flex()
                .id(("terminal-pane-floating-tool", pane_id.value()))
                .absolute()
                .top_2()
                .right_2()
                .items_center()
                .rounded_md()
                .bg(background)
                .border_1()
                .border_color(border)
                .text_color(cx.theme().foreground)
                .when(cx.theme().shadow, |this| this.shadow_md())
                .invisible()
                .group_hover(hover_group, |this| this.visible())
                .child(self.render_split_button(pane_id, workspace, split_supported))
                .into_any_element();
        }

        h_flex()
            .id(("terminal-pane-floating-tool", pane_id.value()))
            .absolute()
            .top_2()
            .right_2()
            .min_w(px(190.0))
            .max_w(relative(0.8))
            .items_center()
            .gap_1()
            .px_1()
            .py_0p5()
            .rounded_md()
            .bg(background)
            .border_1()
            .border_color(border)
            .text_color(cx.theme().foreground)
            .when(cx.theme().shadow, |this| this.shadow_md())
            .child(
                Icon::new(IconName::TerminalColor)
                    .color()
                    .with_size(Size::XSmall),
            )
            .child(self.render_drag_title(pane_id, title, drag_source, cx))
            .child(self.render_split_button(pane_id, workspace.clone(), split_supported))
            .child(self.render_cancel_split_button(pane_id, workspace.clone()))
            .child(self.render_close_button(pane_id, workspace))
            .into_any_element()
    }

    fn render_drag_title(
        &self,
        pane_id: TerminalPaneId,
        title: SharedString,
        drag_source: Option<Arc<dyn one_core::tab_container::ExternalTabDragSource>>,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let drag_title = title.clone();
        let tooltip_title = title.clone();
        div()
            .id(("terminal-pane-drag-title", pane_id.value()))
            .flex_1()
            .min_w(px(60.0))
            .truncate()
            .text_xs()
            .tooltip(move |window, cx| Tooltip::new(tooltip_title.clone()).build(window, cx))
            .when_some(drag_source, |this, source| {
                this.cursor_grab()
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_mouse_move(|_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_drag(
                        DragTab::from_external(drag_title, source),
                        |drag, _, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            cx.new(|_| drag.clone())
                        },
                    )
            })
            .child(title)
            .into_any_element()
    }

    fn render_split_button(
        &self,
        pane_id: TerminalPaneId,
        workspace: gpui::WeakEntity<Self>,
        split_supported: bool,
    ) -> AnyElement {
        Button::new(("terminal-pane-split", pane_id.value()))
            .ghost()
            .xsmall()
            .icon(IconName::LayoutDashboard)
            .tooltip(t!("TerminalWorkspace.split").to_string())
            .disabled(!split_supported)
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                menu.item(split_pane_menu_item(
                    t!("TerminalWorkspace.split_left").to_string(),
                    Placement::Left,
                    pane_id,
                    workspace.clone(),
                ))
                .item(split_pane_menu_item(
                    t!("TerminalWorkspace.split_right").to_string(),
                    Placement::Right,
                    pane_id,
                    workspace.clone(),
                ))
                .item(split_pane_menu_item(
                    t!("TerminalWorkspace.split_top").to_string(),
                    Placement::Top,
                    pane_id,
                    workspace.clone(),
                ))
                .item(split_pane_menu_item(
                    t!("TerminalWorkspace.split_bottom").to_string(),
                    Placement::Bottom,
                    pane_id,
                    workspace.clone(),
                ))
            })
            .into_any_element()
    }

    fn render_cancel_split_button(
        &self,
        pane_id: TerminalPaneId,
        workspace: gpui::WeakEntity<Self>,
    ) -> Button {
        Button::new(("terminal-pane-cancel-split", pane_id.value()))
            .ghost()
            .xsmall()
            .icon(IconName::Undo2)
            .tooltip(t!("TerminalWorkspace.cancel_split").to_string())
            .on_click(move |_, window, cx| {
                let _ = workspace.update(cx, |workspace, cx| {
                    workspace.restore_pane_to_tab(pane_id, window, cx);
                });
            })
    }

    fn render_close_button(
        &self,
        pane_id: TerminalPaneId,
        workspace: gpui::WeakEntity<Self>,
    ) -> Button {
        Button::new(("terminal-pane-close", pane_id.value()))
            .ghost()
            .xsmall()
            .icon(IconName::Close)
            .tooltip(t!("Common.close").to_string())
            .on_click(move |_, window, cx| {
                let _ = workspace.update(cx, |workspace, cx| {
                    workspace.request_close_pane(pane_id, window, cx);
                });
            })
    }
}

fn split_pane_menu_item(
    label: String,
    placement: Placement,
    pane_id: TerminalPaneId,
    workspace: gpui::WeakEntity<TerminalWorkspace>,
) -> PopupMenuItem {
    PopupMenuItem::new(label).on_click(move |_, window, cx| {
        let _ = workspace.update(cx, |workspace, cx| {
            workspace.split_pane(pane_id, placement, window, cx);
        });
    })
}
