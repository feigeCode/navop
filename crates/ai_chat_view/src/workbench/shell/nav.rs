//! 工作台外壳的渲染方法：会话导航、面板头与停靠面板。
//!
//! 与 `shell/mod.rs` 共用同一个 `impl WorkbenchShell`。

use gpui::{
    Anchor, Context, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _, Size, StyledExt as _, h_flex,
    button::Button,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _, v_flex,
};
use one_assets::IconName;
use one_ui::{IconButton, IconButtonRole};
use rust_i18n::t;

use crate::session_sidebar::{
    SessionSummary, WorkspaceGroup, filter_sessions, format_timestamp,
    group_sessions_by_workspace,
};
use crate::{
    DefaultAgentChatPanel, acp_session_placeholder, acp_session_row, acp_session_section_header,
};
use super::super::state::WorkbenchPanelKind;
use super::widgets::rail_tooltip;
use crate::theme::AgentChatTheme;
use super::WorkbenchShell;
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

        // 搜索过滤 + 按工作区分组（组内近期优先，未分组垫底）。
        let query = self.search_query.clone();
        let filtered = filter_sessions(&summaries, &query);
        let groups = group_sessions_by_workspace(&filtered);

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

        // 底部固定区「新对话」：下拉选工作区（已添加过的分组都在里面），
        // 归属在会话首次落盘时定格，之后不可再切换工作区。
        let workspace_items: Vec<(SharedString, String)> = groups
            .iter()
            .filter_map(|group| group.root.clone().map(|root| (group.label(), root)))
            .collect();
        let new_button = {
            let this = cx.entity();
            Button::new("workbench-session-new")
                .icon(IconName::Plus)
                .label(t!("AgentUi.new_conversation").to_string())
                .small()
                .flex_1()
                .min_w_0()
                .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _window, _cx| {
                    let this = this.clone();
                    let mut menu = menu.item(
                        PopupMenuItem::new(t!("Workbench.new_chat_here").to_string())
                            .icon(IconName::Plus)
                            .on_click({
                                let this = this.clone();
                                move |_, _, cx| {
                                    this.update(cx, |this, cx| {
                                        if let Some(panel) = this.session_source.clone() {
                                            panel.update(cx, |panel, cx| panel.create_session(cx));
                                        }
                                    });
                                }
                            }),
                    );
                    for (label, root) in &workspace_items {
                        let this = this.clone();
                        let root = root.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                t!(
                                    "Workbench.new_chat_in_workspace",
                                    workspace = label.to_string()
                                )
                                .to_string(),
                            )
                            .icon(IconName::FolderOpen)
                            .on_click(move |_, _, cx| {
                                let root = root.clone();
                                this.update(cx, |this, cx| {
                                    this.create_session_in_workspace(
                                        std::path::Path::new(&root),
                                        cx,
                                    );
                                });
                            }),
                        );
                    }
                    menu
                })
        };

        // 工作区行：当前工作区 + 新增按钮（Finch 式，工作区入口在侧栏内）。
        let workspace_row = self.workspace_root.as_ref().map(|root| {
            let name = root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string());
            h_flex()
                .flex_shrink_0()
                .items_center()
                .gap_1()
                .px_2()
                .pt_2()
                .pb_1()
                .child(
                    h_flex()
                        .id("workbench-workspace-button")
                        .flex_1()
                        .min_w_0()
                        .h(px(28.0))
                        .items_center()
                        .gap_1()
                        .px_2()
                        .rounded(theme.surface_radius)
                        .bg(theme.panel)
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
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .font_semibold()
                                .text_color(theme.foreground)
                                .child(name),
                        ),
                )
                .child(
                    IconButton::new("workbench-workspace-add", IconName::Plus)
                        .role(IconButtonRole::Compact)
                        .tooltip(t!("Workbench.choose_workspace").to_string())
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(picker) = this.workspace_picker.take() {
                                (picker)(window, cx);
                                this.workspace_picker = Some(picker);
                            }
                        })),
                )
        });

        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        if groups.is_empty() {
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
            for group in &groups {
                rows.push(self.workspace_group_header(group, theme, cx));
                for summary in &group.sessions {
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
                .children(workspace_row)
                // 搜索行：圆角搜索框（无输入能力时整行省略）。
                .when(self.search_input.is_some(), |this| {
                    this.child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .pb_1()
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
                // 底部固定区：「新对话」下拉 + 归档开关。
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

    /// 工作区分组标题：目录名 + hover 出「新建对话」+ 右侧会话计数。
    fn workspace_group_header(
        &self,
        group: &WorkspaceGroup,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let count = group.sessions.len();
        let hover_group = SharedString::from(format!(
            "ws-group-hover-{}",
            group.root.as_deref().unwrap_or("ungrouped")
        ));
        let new_button = group.root.clone().map(|root| {
            let this = cx.entity();
            let tooltip = t!(
                "Workbench.new_chat_in_workspace",
                workspace = group.label().to_string()
            )
            .to_string();
            // 平时隐藏，hover 分组时浮现（opacity 不影响命中，区域很小）。
            div()
                .opacity(0.0)
                .group_hover(hover_group.clone(), |style| style.opacity(1.0))
                .child(
                    IconButton::new(
                        SharedString::from(format!("workbench-ws-new-{hover_group}")),
                        IconName::Plus,
                    )
                    .role(IconButtonRole::Compact)
                    .tooltip(tooltip)
                    .on_click(move |_, _, cx| {
                        let root = root.clone();
                        this.update(cx, |this, cx| {
                            this.create_session_in_workspace(std::path::Path::new(&root), cx);
                        });
                    }),
                )
        });

        h_flex()
            .group(hover_group)
            .pt_2()
            .px_2()
            .pb_0p5()
            .items_center()
            .gap_1()
            .child(
                Icon::new(IconName::Folder)
                    .xsmall()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .child(group.label()),
            )
            .when_some(new_button, |this, button| this.child(button))
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("Workbench.session_count", count = count).to_string()),
            )
            .into_any_element()
    }

    /// 内建会话列表的一行：名称 + 归档按钮 + 相对时间。
    ///
    /// 点击即打开会话；若它属于另一个工作区，外壳先经宿主切根再切换。
    fn session_row(
        summary: &SessionSummary,
        selected: bool,
        theme: &AgentChatTheme,
        panel: &Entity<DefaultAgentChatPanel>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let id = summary.id.clone();
        let workspace_root = summary.workspace_root.clone();
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
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_session_from_list(&id, workspace_root.as_deref(), cx);
            }))
            .into_any_element()
    }

    pub(super) fn render_rail(
        &self,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        // 页签模型：rail 只放可停靠面板（对话是中心区主场，不进 rail），
        // 点击一律打开/激活右侧标签组。
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
            .children(WorkbenchPanelKind::DOCKABLE.into_iter().map(|kind| {
                let open = self.state.placement_of(kind).is_some();
                let color = if open {
                    theme.foreground
                } else {
                    theme.muted_foreground
                };
                IconButton::new(
                    SharedString::from(format!("workbench-rail-{}", kind.id())),
                    kind.icon(),
                )
                .role(IconButtonRole::Compact)
                .tooltip(rail_tooltip(kind, self.state.placement_of(kind)))
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
