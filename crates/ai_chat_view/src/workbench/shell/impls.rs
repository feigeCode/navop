use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    SharedString,
    Styled, Window, div, prelude::FluentBuilder, px,
};

use gpui_component::{Icon, Sizable as _, Size, h_flex, v_flex};
use one_assets::IconName;
use one_core::layout::SIDEBAR_DEFAULT_WIDTH;
use one_core::sidebar_contribution::SidebarPlacement;
use one_core::tab_container::{TabContent, TabContentEvent};
use one_ui::{IconButton, IconButtonRole};
use rust_i18n::t;

use super::super::state::WorkbenchPanelKind;
use crate::theme::{AgentChatTheme, with_agent_chat_theme};
use super::widgets::{next_placement, placement_icon, placement_tooltip};
use super::{DOCK_PANEL_HEIGHT, DOCK_PANEL_WIDTH, PANEL_HEADER_HEIGHT};
use super::WorkbenchShell;


impl EventEmitter<TabContentEvent> for WorkbenchShell {}

impl TabContent for WorkbenchShell {
    fn content_key(&self) -> &'static str {
        "AiWorkbench"
    }

    fn title(&self, _cx: &App) -> SharedString {
        t!("AgentUi.workbench").into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::AI.color().with_size(Size::Medium))
    }

    fn closeable(&self, _cx: &App) -> bool {
        self.tab_closeable
    }

    fn can_rename(&self, _cx: &App) -> bool {
        false
    }

    fn dump(&self, _cx: &App) -> serde_json::Value {
        serde_json::json!({ "version": 1 })
    }
}

impl Focusable for WorkbenchShell {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkbenchShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.snapshot(cx);
        let show_nav = !self.state.nav_collapsed() && self.has_nav();
        let nav = show_nav
            .then(|| self.render_session_nav(&theme, cx))
            .flatten();
        let right_panel = self.state.layout().right;
        let bottom_panel = self.state.layout().bottom;

        let body = with_agent_chat_theme(&theme, || {
            h_flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .when_some(nav, |this, nav| {
                    this.child(
                        v_flex()
                            .w(SIDEBAR_DEFAULT_WIDTH)
                            .flex_shrink_0()
                            .h_full()
                            .min_h_0()
                            .border_r_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .child(nav),
                            ),
                    )
                })
                .child(self.render_content(&theme))
                .when_some(right_panel, |this, kind| {
                    this.child(
                        div()
                            .w(px(DOCK_PANEL_WIDTH))
                            .flex_shrink_0()
                            .h_full()
                            .min_h_0()
                            .border_l_1()
                            .border_color(theme.border)
                            .child(self.render_dock(kind, SidebarPlacement::Right, &theme, cx)),
                    )
                })
                .child(self.render_rail(&theme, cx))
                .into_any_element()
        });

        let bottom = bottom_panel.map(|kind| {
            with_agent_chat_theme(&theme, || {
                div()
                    .w_full()
                    .h(px(DOCK_PANEL_HEIGHT))
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(self.render_dock(kind, SidebarPlacement::Bottom, &theme, cx))
                    .into_any_element()
            })
        });

        let header =
            with_agent_chat_theme(&theme, || self.render_header(&theme, cx).into_any_element());

        with_agent_chat_theme(&theme, || {
            v_flex()
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .bg(theme.background)
                .text_color(theme.foreground)
                .child(header)
                .child(body)
                .when_some(bottom, |this, bottom| this.child(bottom))
        })
    }
}

impl WorkbenchShell {
    fn render_dock(
        &self,
        kind: WorkbenchPanelKind,
        placement: SidebarPlacement,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(view) = self.panels.get(&kind).cloned() else {
            return div().into_any_element();
        };
        let next = next_placement(placement);
        let on_move = cx.listener(move |this, _, _, cx| this.move_dock_panel(kind, next, cx));
        let on_close =
            cx.listener(move |this, _, _, cx| this.close_dock_panel(placement, cx));

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(
                h_flex()
                    .h(px(PANEL_HEADER_HEIGHT))
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.panel)
                    .child(Icon::new(kind.icon()).with_size(Size::Small))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(theme.foreground)
                            .child(kind.title()),
                    )
                    .child(
                        IconButton::new(
                            SharedString::from(format!("workbench-dock-move-{}", kind.id())),
                            placement_icon(next),
                        )
                        .role(IconButtonRole::Compact)
                        .tooltip(placement_tooltip(next))
                        .on_click(on_move),
                    )
                    .child(
                        IconButton::new(
                            SharedString::from(format!("workbench-dock-close-{}", kind.id())),
                            IconName::Close,
                        )
                        .role(IconButtonRole::Compact)
                        .tooltip(t!("Workbench.close_panel").to_string())
                        .on_click(on_close),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(view),
            )
            .into_any_element()
    }
}
