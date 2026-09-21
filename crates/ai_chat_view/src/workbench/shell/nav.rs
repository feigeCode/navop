//! 工作台外壳的渲染方法：会话导航、面板头与停靠面板。
//!
//! 与 `shell/mod.rs` 共用同一个 `impl WorkbenchShell`。

use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex,
    scroll::ScrollableElement as _, v_flex,
};
use one_assets::IconName;
use one_ui::{IconButton, IconButtonRole};
use rust_i18n::t;

use crate::session_sidebar::format_timestamp;
use crate::{acp_session_placeholder, acp_session_row, acp_session_section_header};
use super::super::state::WorkbenchPanelKind;
use crate::theme::AgentChatTheme;
use super::{HEADER_HEIGHT, WorkbenchShell};
use one_core::layout::TOOLBAR_WIDTH;

impl WorkbenchShell {

    /// 会话导航：优先用外部注入的视图，否则读 `session_source` 的会话列表。
    pub(super) fn render_session_nav(
        &self,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if let Some(nav) = self.nav.clone() {
            return Some(nav.into_any_element());
        }
        let panel = self.session_source.as_ref()?.clone();
        let (summaries, current, acp_model, acp_current) = {
            let view = panel.read(cx);
            (
                view.session_summaries(cx),
                view.current_session_id(cx),
                view.acp_session_list_model(cx),
                view.acp_session_id(cx),
            )
        };

        let new_button = {
            let panel = panel.clone();
            IconButton::new("workbench-session-new", IconName::Plus)
                .role(IconButtonRole::Compact)
                .tooltip(t!("AgentUi.new_conversation").to_string())
                .on_click(cx.listener(move |_, _, _, cx| {
                    panel.update(cx, |panel, cx| panel.create_session(cx));
                }))
        };

        let mut rows: Vec<gpui::AnyElement> = Vec::with_capacity(summaries.len() + 4);
        for summary in summaries {
            let selected = current.as_deref() == Some(summary.id.as_str());
            let id = summary.id.clone();
            let hover = theme.hover_background();
            let panel = panel.clone();
            let element_id = SharedString::from(format!("workbench-session-{}", summary.id));
            rows.push(
                v_flex()
                    .id(element_id)
                    .w_full()
                    .px_2()
                    .py_1p5()
                    .gap_0p5()
                    .rounded(theme.surface_radius)
                    .cursor_pointer()
                    .when(selected, |this| this.bg(theme.panel_hover))
                    .hover(move |style| style.bg(hover))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(theme.foreground)
                            .child(summary.name.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format_timestamp(summary.updated_at)),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| {
                        panel.update(cx, |panel, cx| panel.select_session(&id, cx));
                    }))
                    .into_any_element(),
            );
        }

        // ACP 历史会话：和内置会话并排，只额外标出来源（方案 §7.1 的统一列表）。
        if let Some(model) = acp_model {
            rows.push(acp_session_section_header(
                theme,
                SharedString::from("workbench-acp-sessions-refresh"),
                cx.listener({
                    let panel = panel.clone();
                    move |_, _, _, cx| {
                        panel.update(cx, |panel, cx| panel.refresh_acp_sessions(cx));
                    }
                }),
            ));
            for summary in &model.sessions {
                let id = summary.id.clone();
                let panel = panel.clone();
                rows.push(acp_session_row(
                    theme,
                    summary,
                    acp_current.as_deref() == Some(summary.id.as_str()),
                    SharedString::from(format!("workbench-acp-session-{}", summary.id)),
                    cx.listener(move |_, _, _, cx| {
                        panel.update(cx, |panel, cx| panel.open_acp_session(&id, cx));
                    }),
                ));
            }
            if let Some(placeholder) = acp_session_placeholder(
                theme,
                cx.theme().danger,
                model.loading,
                model.error.as_deref(),
                !model.sessions.is_empty(),
            ) {
                rows.push(placeholder);
            }
        }

        Some(
            v_flex()
                .size_full()
                .min_h_0()
                .child(
                    h_flex()
                        .h(px(HEADER_HEIGHT))
                        .flex_shrink_0()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .text_color(theme.foreground)
                                .child(t!("Workbench.sessions").to_string()),
                        )
                        .child(new_button),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .size_full()
                        .overflow_y_scrollbar()
                        .px_1()
                        .py_1()
                        .gap_0p5()
                        .children(rows),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_header(
        &self,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let nav_toggle = self.has_nav().then(|| {
            let collapsed = self.state.nav_collapsed();
            IconButton::new(
                "workbench-nav-toggle",
                if collapsed {
                    IconName::PanelLeftOpen
                } else {
                    IconName::PanelLeftClose
                },
            )
            .role(IconButtonRole::Compact)
            .tooltip(if collapsed {
                t!("Workbench.expand_nav")
            } else {
                t!("Workbench.collapse_nav")
            }
            .to_string())
            .on_click(cx.listener(|this, _, _, cx| this.toggle_session_nav(cx)))
        });

        // 当前工作区常显且可点击切换：这是新建对话选择上下文的主入口。
        let workspace = self.workspace_root.as_ref().map(|root| {
            let name = root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string());
            let path = root.display().to_string();
            h_flex()
                .id("workbench-workspace-button")
                .min_w_0()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .rounded_md()
                .cursor_pointer()
                .hover(|style| style.bg(theme.panel_hover))
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(picker) = this.workspace_picker.take() {
                        (picker)(window, cx);
                        this.workspace_picker = Some(picker);
                    }
                }))
                .child(
                    Icon::new(IconName::FolderOpen)
                        .xsmall()
                        .text_color(theme.muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.foreground)
                        .child(name),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .max_w(px(260.0))
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(path),
                )
        });

        h_flex()
            .h(px(HEADER_HEIGHT))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(theme.border)
            .when_some(nav_toggle, |this, toggle| this.child(toggle))
            .child(div().flex_1())
            .when_some(workspace, |this, workspace| this.child(workspace))
    }

    pub(super) fn render_rail(
        &self,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        v_flex()
            .w(TOOLBAR_WIDTH)
            .flex_shrink_0()
            .h_full()
            .items_center()
            .gap_1()
            .py_1()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.background)
            // 中心区固定是对话；其余面板只作为侧边 dock，从 rail 开关。
            .children(
                WorkbenchPanelKind::ALL
                    .into_iter()
                    .filter(|kind| *kind != WorkbenchPanelKind::Chat)
                    .map(|kind| {
                let open = self.state.is_panel_open(kind);
                IconButton::new(
                    SharedString::from(format!("workbench-rail-{}", kind.id())),
                    kind.icon(),
                )
                .role(IconButtonRole::Compact)
                .tooltip(kind.title())
                .text_color(if open {
                    theme.accent
                } else {
                    theme.muted_foreground
                })
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_dock_panel(kind, cx)))
                .into_any_element()
                    }),
            )
    }

    pub(super) fn render_content(&self, theme: &AgentChatTheme) -> gpui::AnyElement {
        // 中心区只有对话；Review/文件/终端经侧边 dock 展示。
        match self.panels.get(&WorkbenchPanelKind::Chat) {
            Some(view) => div()
                .debug_selector(|| "workbench-content".to_string())
                .flex_1()
                .h_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .child(view.clone())
                .into_any_element(),
            None => div()
                .debug_selector(|| "workbench-content".to_string())
                .flex_1()
                .h_full()
                .min_w_0()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(t!("Workbench.panel_unavailable").to_string())
                .into_any_element(),
        }
    }
}
