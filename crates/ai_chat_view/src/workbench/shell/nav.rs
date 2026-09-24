//! 工作台外壳的渲染方法：会话导航、面板头与停靠面板。
//!
//! 与 `shell/mod.rs` 共用同一个 `impl WorkbenchShell`。

use gpui::{
    Context, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _, Size, StyledExt as _, h_flex,
    input::Input,
    scroll::ScrollableElement as _, v_flex,
};
use one_assets::IconName;
use one_ui::{IconButton, IconButtonRole};
use rust_i18n::t;

use crate::session_sidebar::{SessionSummary, filter_sessions, format_timestamp, group_sessions};
use crate::{
    DefaultAgentChatPanel, acp_session_placeholder, acp_session_row, acp_session_section_header,
};
use super::super::state::{WorkbenchPanelKind, WorkbenchPlacement};
use super::widgets::rail_tooltip;
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

        let showing_archived = panel.read(cx).showing_archived_sessions(cx);
        // 底部固定区的「新对话」主入口（Finch 式：整宽按钮 + 归档开关）。
        let new_button = {
            let panel = panel.clone();
            h_flex()
                .id("workbench-session-new")
                .flex_1()
                .min_w_0()
                .h(px(30.0))
                .items_center()
                .justify_center()
                .gap_1p5()
                .rounded(theme.surface_radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.panel)
                .cursor_pointer()
                .hover(|style| style.bg(theme.panel_hover))
                .on_click(cx.listener(move |_, _, _, cx| {
                    panel.update(cx, |panel, cx| panel.create_session(cx));
                }))
                .child(
                    Icon::new(IconName::Plus)
                        .xsmall()
                        .text_color(theme.foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .font_medium()
                        .text_color(theme.foreground)
                        .child(t!("AgentUi.new_conversation").to_string()),
                )
        };
        let archive_toggle = {
            let panel = panel.clone();
            IconButton::new("workbench-session-archived", IconName::WindowRestore)
                .role(IconButtonRole::Compact)
                .when(showing_archived, |button| button.outline())
                .tooltip(
                    if showing_archived {
                        t!("Workbench.show_active_conversations")
                    } else {
                        t!("Workbench.show_archived_conversations")
                    }
                    .to_string(),
                )
                .on_click(cx.listener(move |_, _, _, cx| {
                    panel.update(cx, |panel, cx| panel.toggle_archived_sessions(cx));
                }))
        };

        // 侧栏：搜索过滤 + 按本地日历日分组。输入按更新时间降序。
        let query = self.search_query.clone();
        let filtered = filter_sessions(&summaries, &query);
        let now = chrono::Local::now().fixed_offset();
        let sections = group_sessions(&filtered, now);

        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        if sections.is_empty() {
            if !query.trim().is_empty() {
                rows.push(
                    div()
                        .px_2()
                        .py_3()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t!("Workbench.no_matching_sessions").to_string())
                        .into_any_element(),
                );
            }
        } else {
            for (group, items) in sections {
                // Finch 式分组标题：左侧组名，右侧「N 个会话」计数。
                let count = items.len();
                rows.push(
                    h_flex()
                        .pt_2()
                        .px_2()
                        .pb_0p5()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .text_color(theme.muted_foreground)
                                .child(group.label()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(t!("Workbench.session_count", count = count).to_string()),
                        )
                        .into_any_element(),
                );
                for summary in items {
                    rows.push(Self::session_row(
                        summary,
                        current.as_deref() == Some(summary.id.as_str()),
                        theme,
                        &panel,
                        cx,
                    ));
                }
            }
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
                // 顶部搜索行：圆角搜索框（无输入能力时整行省略）。
                .when(self.search_input.is_some(), |this| {
                    this.child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(28.0))
                                    .items_center()
                                    .gap_1()
                                    .px_2()
                                    .rounded(theme.surface_radius)
                                    .bg(theme.panel)
                                    .when_some(
                                        self.search_input.as_ref(),
                                        |row, state| {
                                            row.child(
                                                Icon::new(IconName::Search)
                                                    .small()
                                                    .text_color(theme.muted_foreground),
                                            )
                                            .child(
                                                Input::new(state)
                                                    .with_size(Size::Small)
                                                    .appearance(false),
                                            )
                                        },
                                    ),
                            ),
                    )
                })
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .size_full()
                        .overflow_y_scrollbar()
                        .px_1()
                        .pb_1()
                        .gap_0p5()
                        .children(rows),
                )
                // 底部固定区：归档开关 + 「新对话」主入口。
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py_2()
                        .border_t_1()
                        .border_color(theme.border)
                        .child(new_button)
                        .child(archive_toggle),
                )
                .into_any_element(),
        )
    }

    /// 内建会话列表的一行：名称 + 归档按钮 + 相对时间。
    fn session_row(
        summary: &SessionSummary,
        selected: bool,
        theme: &AgentChatTheme,
        panel: &Entity<DefaultAgentChatPanel>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let id = summary.id.clone();
        let hover = theme.hover_background();
        let panel = panel.clone();
        let element_id = SharedString::from(format!("workbench-session-{}", summary.id));
        let archive_panel = panel.clone();
        let archive_uid = id.clone();

        // Finch 式单行会话条目：名称居左，归档按钮与相对时间居右。
        h_flex()
            .id(element_id)
            .w_full()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(theme.surface_radius)
            .cursor_pointer()
            .when(selected, |this| this.bg(theme.panel_hover))
            .hover(move |style| style.bg(hover))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(summary.name.clone()),
            )
            .child(
                IconButton::new(
                    SharedString::from(format!("workbench-session-archive-{id}")),
                    IconName::Delete,
                )
                .role(IconButtonRole::Compact)
                .tooltip(t!("Workbench.archive_conversation").to_string())
                .on_click(move |_, _window, cx| {
                    let uid = archive_uid.clone();
                    archive_panel.update(cx, |panel, cx| {
                        panel.archive_session(&uid, cx);
                    });
                }),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format_timestamp(summary.updated_at)),
            )
            .on_click(cx.listener(move |_, _, _, cx| {
                panel.update(cx, |panel, cx| panel.select_session(&id, cx));
            }))
            .into_any_element()
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

        // Finch 式顶栏：「工作区 > 会话」面包屑。工作区常显可点（切换上下文
        // 的主入口），会话段只读、随当前会话变化。
        let session_name = self.session_source.as_ref().and_then(|panel| {
            let view = panel.read(cx);
            let current = view.current_session_id(cx)?;
            view.session_summaries(cx)
                .into_iter()
                .find(|summary| summary.id == current)
                .map(|summary| summary.name)
        });

        h_flex()
            .h(px(HEADER_HEIGHT))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(theme.border)
            .when_some(nav_toggle, |this, toggle| this.child(toggle))
            .when_some(self.workspace_root.as_ref(), |this, root| {
                let name = root
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.display().to_string());
                this.child(
                    h_flex()
                        .id("workbench-workspace-button")
                        .min_w_0()
                        .flex_shrink_0()
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
                        ),
                )
                .when_some(session_name, |this, name| {
                    this.child(
                        Icon::new(IconName::ChevronRight)
                            .xsmall()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(name),
                    )
                })
            })
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
            .children(WorkbenchPanelKind::ALL.into_iter().map(|kind| {
                let placement = self.state.placement_of(kind);
                // 在中心区的面板是当前主角，用强调色；停靠中的用前景色；
                // 未打开的用弱色。点击统一走 activate_panel（开/提焦点/关）。
                let color = match placement {
                    Some(WorkbenchPlacement::Center) => theme.accent,
                    Some(_) => theme.foreground,
                    None => theme.muted_foreground,
                };
                IconButton::new(
                    SharedString::from(format!("workbench-rail-{}", kind.id())),
                    kind.icon(),
                )
                .role(IconButtonRole::Compact)
                .tooltip(rail_tooltip(kind, placement))
                .text_color(color)
                .on_click(cx.listener(move |this, _, _, cx| this.activate_panel(kind, cx)))
                .into_any_element()
            }))
    }

    pub(super) fn render_content(&self, theme: &AgentChatTheme) -> gpui::AnyElement {
        // 中心区跟随状态单选；「对话」不可关闭，所以正常路径总有面板可显示。
        let center = self.state.center().unwrap_or(WorkbenchPanelKind::Chat);
        let body = self
            .panels
            .get(&center)
            .map(|view| view.clone().into_any_element());

        match body {
            Some(view) => div()
                .debug_selector(|| "workbench-content".to_string())
                .flex_1()
                .h_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .child(view)
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
