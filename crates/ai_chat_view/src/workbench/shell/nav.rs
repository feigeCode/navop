//! 工作台外壳的渲染方法：会话导航、面板头与停靠面板。
//!
//! 与 `shell/mod.rs` 共用同一个 `impl WorkbenchShell`。

use gpui::{
    Anchor, Context, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px,
};
use gpui_component::tooltip::Tooltip;
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _, Size, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    v_flex,
};
use one_assets::IconName;
use one_ui::{IconButton, IconButtonRole};
use rust_i18n::t;

use super::super::state::{WorkbenchPanelKind, WorkbenchTab};
use super::WorkbenchShell;
use crate::session_sidebar::{
    SessionSummary, WorkspaceGroup, date_bucket_header, filter_sessions, format_timestamp,
    group_sessions_by_date, group_sessions_by_workspace, now_unix,
};
use crate::theme::AgentChatTheme;
use crate::{
    DefaultAgentChatPanel, acp_session_placeholder, acp_session_row, acp_session_section_header,
};

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
        let (summaries, current, acp_model, acp_current, backend_is_acp) = {
            let view = panel.read(cx);
            (
                view.session_summaries(cx),
                view.current_session_id(cx),
                view.acp_session_list_model(cx),
                view.acp_session_id(cx),
                view.backend_is_acp(cx),
            )
        };

        let showing_archived = panel.read(cx).showing_archived_sessions(cx);
        // 列表里所有相对时间/日期分组共用同一个 `now`:同一帧内算法一致,
        // 不会出现两条会话因取时差跨到不同桶。
        let now = now_unix();

        // 搜索过滤 + 按工作区分组（组内近期优先，未分组垫底），
        // 用户「移除」过的工作区不参与分组。
        let query = self.search_query.clone();
        let filtered = filter_sessions(&summaries, &query);
        let groups: Vec<WorkspaceGroup> = group_sessions_by_workspace(&filtered)
            .into_iter()
            .filter(|group| {
                group
                    .root
                    .as_deref()
                    .map_or(true, |root| !self.state.is_workspace_hidden(root))
            })
            .collect();
        // 工作区选择器的候选来自**全量**会话，与搜索词无关——搜索只过滤列表。
        // 现在改由输入框下方上下文栏呈现（宿主经 `ComposerContextSource` 注入），
        // 这里不再重复算一遍分组。

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
                let key = group_key(group);
                rows.push(self.workspace_group_header(group, theme, cx));
                if !self.group_collapsed(&key) {
                    // 工作区分组之内再按时间分桶:同一项目的会话往往是「今天调的」
                    // 与「上周调的」混在一起，插一行日期标题就能把当前这一阵的
                    // 上下文和旧记录分开。只有一个桶时不插——那时候标题是纯噪音。
                    let buckets = group_sessions_by_date(&group.sessions, now);
                    let show_dates = buckets.len() > 1;
                    for (bucket, sessions) in buckets {
                        if show_dates {
                            rows.push(date_bucket_header(bucket, cx).into_any_element());
                        }
                        for summary in sessions {
                            rows.push(Self::session_row(
                                summary,
                                current.as_deref() == Some(summary.id.as_str()),
                                true,
                                theme,
                                &panel,
                                cx,
                            ));
                        }
                    }
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
        } else if backend_is_acp {
            // 外接 agent 没声明 `session/list`：这一区此前会**整块消失**，
            // 用户既看不到会话、也看不到任何解释。给一句话说明会话归谁管。
            rows.push(
                div()
                    .px_2()
                    .py_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("AgentUi.acp_external_managed").to_string())
                    .into_any_element(),
            );
        }

        Some(
            v_flex()
                .size_full()
                .min_h_0()
                // 搜索行：圆角搜索框（无输入能力时整行省略）。
                .when(self.search_input.is_some(), |this| {
                    this.child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py_2()
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
                                    .when_some(self.search_input.as_ref(), |row, state| {
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
                                    }),
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
                // 底部固定区：归档开关。工作区切换已收口到输入框下方的上下文栏
                // （宿主经 `ComposerContextSource` 注入的工作区下拉），这里不再重复。
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .items_center()
                        .justify_end()
                        .gap_1()
                        .px_2()
                        .py_2()
                        .border_t_1()
                        .border_color(theme.border)
                        .bg(theme.panel)
                        .child(archive_toggle),
                )
                .into_any_element(),
        )
    }

    /// 工作区分组标题：chevron + 目录名；会话计数平时显示，hover 时换成
    /// 「+ 新对话」与「…」菜单（菜单项见 [`workspace_group_menu`]）。
    ///
    /// 整行可点：点击展开/收起该组（仅会话内记忆）。
    fn workspace_group_header(
        &self,
        group: &WorkspaceGroup,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let count = group.sessions.len();
        let key = group_key(group);
        let click_key = key.clone();
        let collapsed = self.group_collapsed(&key);
        let hover_group = SharedString::from(format!(
            "ws-group-hover-{}",
            group.root.as_deref().unwrap_or(super::GROUP_KEY_UNGROUPED)
        ));

        h_flex()
            .id(SharedString::from(format!("workbench-ws-group-{key}")))
            .group(hover_group.clone())
            .pt_2()
            .px_2()
            .pb_0p5()
            .items_center()
            .gap_1()
            .rounded(theme.surface_radius)
            .cursor_pointer()
            // 分组名（目录名）过长会截断，完整路径放 tooltip。
            .tooltip({
                let tip = match group.root.as_deref() {
                    Some(root) => root.to_string(),
                    None => group.label().to_string(),
                };
                move |window, cx| Tooltip::new(tip.clone()).build(window, cx)
            })
            .hover(|style| style.bg(theme.hover_background()))
            .on_click(
                cx.listener(move |this, _, _, cx| this.toggle_workspace_group(&click_key, cx)),
            )
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .xsmall()
                .text_color(theme.muted_foreground),
            )
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
            // 平时显示计数，hover 让位给操作按钮（两侧 opacity 互换）。
            .when(group.root.is_some(), |header| {
                header
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .group_hover(hover_group.clone(), |style| style.opacity(0.0))
                            .child(t!("Workbench.session_count", count = count).to_string()),
                    )
                    .child(
                        // 拦住冒泡：点按钮不能顺带把组折叠了。
                        h_flex()
                            .id(SharedString::from(format!("workbench-ws-actions-{key}")))
                            .flex_shrink_0()
                            .items_center()
                            .gap_0p5()
                            .opacity(0.0)
                            .group_hover(hover_group.clone(), |style| style.opacity(1.0))
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .children(self.group_new_button(group, hover_group, cx))
                            .children(self.group_menu_button(group, cx)),
                    )
            })
            .when(group.root.is_none(), |header| {
                header.child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t!("Workbench.session_count", count = count).to_string()),
                )
            })
            .into_any_element()
    }

    /// 分组头的「+ 新对话」按钮（hover 浮现）。
    fn group_new_button(
        &self,
        group: &WorkspaceGroup,
        hover_group: SharedString,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let root = group.root.clone()?;
        let this = cx.entity();
        let tooltip = t!(
            "Workbench.new_chat_in_workspace",
            workspace = group.label().to_string()
        )
        .to_string();
        Some(
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
            })
            .into_any_element(),
        )
    }

    /// 分组头的「…」菜单按钮（hover 浮现）：新对话 / 在文件管理器中显示 /
    /// 归档不活跃对话 / 移除。设置别名与图标需要工作区元数据存储，暂未做。
    fn group_menu_button(
        &self,
        group: &WorkspaceGroup,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let root = group.root.clone()?;
        let this = cx.entity();
        let inactive = inactive_session_count(group);
        let menu_id = SharedString::from(format!("workbench-ws-menu-{}", group_key(group)));
        Some(
            Button::new(menu_id)
                .icon(IconName::Ellipsis)
                .small()
                .ghost()
                .tooltip(t!("Workbench.workspace_menu").to_string())
                .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _window, _cx| {
                    let root = root.clone();
                    menu.item(
                        PopupMenuItem::new(t!("AgentUi.new_conversation").to_string())
                            .icon(IconName::Plus)
                            .on_click({
                                let this = this.clone();
                                let root = root.clone();
                                move |_, _, cx| {
                                    this.update(cx, |this, cx| {
                                        this.create_session_in_workspace(
                                            std::path::Path::new(&root),
                                            cx,
                                        );
                                    });
                                }
                            }),
                    )
                    .item(
                        PopupMenuItem::new(t!("Workbench.ws_menu_reveal").to_string())
                            .icon(IconName::ExternalLink)
                            .on_click({
                                let root = root.clone();
                                move |_, _, _| {
                                    reveal_in_file_manager(&root);
                                }
                            }),
                    )
                    .item(
                        PopupMenuItem::new(
                            t!("Workbench.ws_menu_archive_inactive", count = inactive).to_string(),
                        )
                        .icon(IconName::Archive)
                        .disabled(inactive == 0)
                        .on_click({
                            let this = this.clone();
                            let root = root.clone();
                            move |_, _, cx| {
                                this.update(cx, |this, cx| {
                                    this.archive_inactive_sessions(
                                        &root,
                                        INACTIVE_ARCHIVE_DAYS,
                                        cx,
                                    );
                                });
                            }
                        }),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new(t!("Workbench.ws_menu_remove").to_string())
                            .icon(IconName::Delete)
                            .on_click({
                                let this = this.clone();
                                let root = root.clone();
                                move |_, _, cx| {
                                    this.update(cx, |this, cx| {
                                        this.hide_workspace_group(&root, cx);
                                    });
                                }
                            }),
                    )
                })
                .into_any_element(),
        )
    }

    /// 内建会话列表的一行：名称 + hover 浮现的归档按钮 + 相对时间。
    ///
    /// 点击即打开会话；若它属于另一个工作区，外壳先经宿主切根再切换。
    /// 分组内的行带缩进（Finch 式树形）。
    fn session_row(
        summary: &SessionSummary,
        selected: bool,
        indented: bool,
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
        // 会话名过长会截断，完整名放 tooltip。
        let row_tooltip = summary.name.to_string();

        // Finch 式单行会话条目：名称居左，归档按钮与相对时间居右。
        h_flex()
            .id(element_id)
            .group("session-row")
            .w_full()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .when(indented, |this| this.pl_6())
            .rounded(theme.surface_radius)
            .cursor_pointer()
            .tooltip(move |window, cx| Tooltip::new(row_tooltip.clone()).build(window, cx))
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
            // 平时隐藏，hover 行时浮现；换用 Archive 图标（Delete 是删除语义）。
            .child(
                div()
                    .id(SharedString::from(format!(
                        "workbench-session-archive-guard-{id}"
                    )))
                    .flex_shrink_0()
                    .opacity(0.0)
                    .group_hover("session-row", |style| style.opacity(1.0))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        IconButton::new(
                            SharedString::from(format!("workbench-session-archive-{id}")),
                            IconName::Archive,
                        )
                        .role(IconButtonRole::Compact)
                        .tooltip(t!("Workbench.archive_conversation").to_string())
                        .on_click(move |_, _window, cx| {
                            let uid = archive_uid.clone();
                            archive_panel.update(cx, |panel, cx| {
                                panel.archive_session(&uid, cx);
                            });
                        }),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(match summary.external_agent.as_ref() {
                        Some(agent) => {
                            format!("{} · {}", agent, format_timestamp(summary.updated_at))
                        }
                        None => format_timestamp(summary.updated_at),
                    }),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_session_from_list(&id, workspace_root.as_deref(), cx);
            }))
            .into_any_element()
    }

    pub(super) fn render_content(&self, theme: &AgentChatTheme) -> gpui::AnyElement {
        // 中心区跟随状态单选；「对话」不可关闭，所以正常路径总有面板可显示。
        let center = self
            .state
            .center()
            .unwrap_or(WorkbenchTab::new(WorkbenchPanelKind::Chat));
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

/// 折叠表里的组键：工作区根目录；未分组会话用固定键。
fn group_key(group: &WorkspaceGroup) -> String {
    group
        .root
        .clone()
        .unwrap_or_else(|| super::GROUP_KEY_UNGROUPED.to_string())
}

/// 「归档不活跃对话」的天数阈值。
const INACTIVE_ARCHIVE_DAYS: u64 = 7;

/// 组内超过 [`INACTIVE_ARCHIVE_DAYS`] 天没动静的会话数（供菜单项文案与禁用态）。
fn inactive_session_count(group: &WorkspaceGroup) -> usize {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0);
    let cutoff = now.saturating_sub((INACTIVE_ARCHIVE_DAYS * 24 * 3600) as i64);
    group
        .sessions
        .iter()
        .filter(|summary| summary.updated_at < cutoff)
        .count()
}

/// 在系统文件管理器里显示工作区目录（macOS Finder / Windows 资源管理器 /
/// Linux xdg-open）。失败静默——这是顺手功能，不值得打扰用户。
fn reveal_in_file_manager(root: &str) {
    let spawned = reveal_command(root).and_then(|mut command| command.spawn().ok());
    if spawned.is_none() {
        tracing::warn!(root, "failed to reveal workspace in file manager");
    }
}

fn reveal_command(root: &str) -> Option<std::process::Command> {
    #[cfg(target_os = "macos")]
    {
        let mut command = std::process::Command::new("open");
        command.arg("-R").arg(root);
        Some(command)
    }
    #[cfg(target_os = "windows")]
    {
        let mut command = std::process::Command::new("explorer");
        command.arg(format!("/select,{root}"));
        Some(command)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(root);
        Some(command)
    }
}
