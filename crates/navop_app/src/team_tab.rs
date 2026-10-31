//! 原生「团队」tab。
//!
//! 团队管理过去用 `cx.open_url` 跳到 navop-website 的 dashboard，并把
//! `access_token` / `refresh_token` 拼进 URL —— 既不安全，也让网站无法下线。
//! 这里用桌面端已有的云端 API 重建第一期视图：左侧团队列表，右侧团队资料、
//! 我的角色、团队密钥与成员目录。
//!
//! 团队密钥管理原本挂在设置页（`SettingsPanel` 的 TeamKeys 页），这次整体搬到这里，
//! 设置页不再保留任何入口，避免两处 UI 维护同一份缓存。

use std::sync::Arc;

use chrono::{DateTime, Local};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Sizable, WindowExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use one_assets::IconName;
use one_core::cloud_sync::{
    CloudApiClient as _, CloudSyncService, GlobalCloudUser, SyncEngine, Team, TeamKeyCacheStatus,
    TeamMemberWithEmail, TeamOption, TeamRole, get_cached_team_options,
};
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::crypto;
use one_core::storage::{GlobalStorageState, TeamMembershipState};
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;

use crate::auth::get_auth_service;

/// 右栏「成员目录」的加载状态。
#[derive(Clone, Debug)]
enum MemberDirectory {
    /// 还没有选中团队。
    Idle,
    /// 正在请求云端成员列表。
    Loading,
    /// 加载成功，按服务端返回顺序展示。
    Loaded(Vec<TeamMemberWithEmail>),
    /// 请求失败，保留错误文案供用户重试。
    Failed(String),
}

pub(crate) struct TeamTab {
    focus_handle: FocusHandle,
    /// 当前选中的团队；列表刷新后若团队已不在缓存里会被清空。
    selected_team_id: Option<String>,
    /// 团队资料（描述 / 拥有者 / 创建时间）来自 `list_teams`，可能比缓存更新。
    team_profile: Option<Team>,
    member_directory: MemberDirectory,
    /// 每次加载自增，用来丢弃迟到的过期响应。
    load_generation: u64,
    /// 登录态 / 同步事件到达时重绘，让密钥状态徽标跟上缓存变化。
    _notifier_subscription: Option<Subscription>,
}

impl TeamTab {
    pub(crate) fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let notifier_subscription = get_notifier(cx)
            .map(|notifier| cx.observe(&notifier, |_, _, cx: &mut Context<Self>| cx.notify()));
        Self {
            focus_handle: cx.focus_handle(),
            selected_team_id: None,
            team_profile: None,
            member_directory: MemberDirectory::Idle,
            load_generation: 0,
            _notifier_subscription: notifier_subscription,
        }
    }

    /// tab 第一次打开时选中第一个团队并加载详情。
    pub(crate) fn ensure_loaded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let teams = get_cached_team_options(cx);
        if self.selected_team_id.is_none() {
            self.selected_team_id = teams.first().map(|team| team.id.clone());
        }
        if matches!(self.member_directory, MemberDirectory::Idle) {
            self.load_selected_team(window, cx);
        }
    }

    fn select_team(&mut self, team_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_team_id.as_deref() == Some(team_id.as_str()) {
            return;
        }
        self.selected_team_id = Some(team_id);
        self.load_selected_team(window, cx);
        cx.notify();
    }

    /// 加载选中团队的资料与成员列表。两次请求互不影响：资料失败仍然展示成员。
    fn load_selected_team(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(team_id) = self.selected_team_id.clone() else {
            self.team_profile = None;
            self.member_directory = MemberDirectory::Idle;
            return;
        };

        self.load_generation += 1;
        let generation = self.load_generation;
        self.team_profile = None;
        self.member_directory = MemberDirectory::Loading;

        let client = get_auth_service(cx).cloud_client();
        cx.spawn(async move |this, cx| {
            let teams = client.list_teams().await;
            let members = client.list_team_members_with_email(&team_id).await;
            this.update(cx, |team_tab, cx| {
                // 切换过团队：这次响应已经过期，直接丢弃。
                if team_tab.load_generation != generation {
                    return;
                }
                team_tab.team_profile = teams
                    .ok()
                    .and_then(|teams| teams.into_iter().find(|team| team.id == team_id));
                team_tab.member_directory = match members {
                    Ok(members) => MemberDirectory::Loaded(members),
                    Err(error) => MemberDirectory::Failed(error.to_string()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_team_list(&self, teams: &[TeamOption], cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity();
        let selected = self.selected_team_id.clone();
        let mut list = v_flex().w_full().gap_1();

        if teams.is_empty() {
            list = list.child(
                v_flex()
                    .gap_1()
                    .p_3()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("TeamManagement.no_teams").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("TeamManagement.no_teams_hint").to_string()),
                    ),
            );
        }

        for team in teams.iter().cloned() {
            let is_selected = selected.as_deref() == Some(team.id.as_str());
            let team_id = team.id.clone();
            let click_entity = entity.clone();
            list = list.child(
                v_flex()
                    .id(SharedString::from(format!("team-list-row-{}", team.id)))
                    .w_full()
                    .gap_1()
                    .p_2()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .when(is_selected, |this| this.bg(cx.theme().muted))
                    .hover(|style| style.bg(cx.theme().muted))
                    .on_click(move |_, window, cx| {
                        click_entity.update(cx, |team_tab, cx| {
                            team_tab.select_team(team_id.clone(), window, cx)
                        });
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(team.name.clone()),
                            )
                            .child(team_role_badge(team.role.as_deref(), cx)),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .child(team_key_status_badge(team.key_status, cx))
                            .when_some(membership_notice(&team, cx), |this, badge| {
                                this.child(badge)
                            }),
                    ),
            );
        }

        list.into_any_element()
    }

    fn render_team_detail(&self, team: &TeamOption, cx: &mut Context<Self>) -> AnyElement {
        let mut detail = v_flex().w_full().gap_4().p_4();

        detail = detail.child(self.render_profile_section(team, cx));
        detail = detail.child(render_team_key_management_section(team, cx));
        detail = detail.child(self.render_member_section(team, cx));

        detail
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("TeamManagement.read_only_hint").to_string()),
            )
            .into_any_element()
    }

    fn render_profile_section(&self, team: &TeamOption, cx: &mut Context<Self>) -> AnyElement {
        let description = self
            .team_profile
            .as_ref()
            .and_then(|profile| profile.description.clone())
            .filter(|description| !description.trim().is_empty());
        let owner_id = self
            .team_profile
            .as_ref()
            .map(|profile| profile.owner_id.clone())
            .unwrap_or_else(|| "-".to_string());
        let created_at = self
            .team_profile
            .as_ref()
            .map(|profile| unix_millis_to_date(profile.created_at))
            .unwrap_or_else(|| "-".to_string());
        let profile_description = description.clone().unwrap_or_default();
        let tab = cx.entity();

        card(cx)
            .child(
                h_flex()
                    .w_full()
                    .items_start()
                    .justify_between()
                    .gap_3()
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(team.name.clone()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        description
                                            .unwrap_or_else(|| {
                                                t!("TeamManagement.description_empty").to_string()
                                            })
                                            .clone(),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_2()
                            .child(team_role_badge(team.role.as_deref(), cx))
                            .when(can_edit_team_profile(team.role.as_deref()), |this| {
                                let tab = tab.clone();
                                let team_id = team.id.clone();
                                let team_name = team.name.clone();
                                let description = profile_description.clone();
                                this.child(
                                    Button::new("team-profile-edit")
                                        .icon(IconName::Edit)
                                        .label(t!("TeamManagement.edit_profile").to_string())
                                        .small()
                                        .on_click(move |_, window, cx| {
                                            show_team_profile_dialog(
                                                tab.clone(),
                                                team_id.clone(),
                                                team_name.clone(),
                                                description.clone(),
                                                window,
                                                cx,
                                            );
                                        }),
                                )
                            }),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(profile_meta_row(
                        t!("TeamManagement.owner").to_string(),
                        shorten_user_id(&owner_id),
                        cx,
                    ))
                    .child(profile_meta_row(
                        t!("TeamManagement.created_at").to_string(),
                        created_at,
                        cx,
                    ))
                    .when_some(membership_notice(team, cx), |this, badge| this.child(badge)),
            )
            .into_any_element()
    }

    fn render_member_section(&self, team: &TeamOption, cx: &mut Context<Self>) -> AnyElement {
        let current_user_id = GlobalCloudUser::get_user(cx).map(|user| user.id);
        let actor_role = team.role.clone();
        let tab = cx.entity();
        let mut header_actions = h_flex().items_center().gap_2().child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(match &self.member_directory {
                    MemberDirectory::Loaded(members) => {
                        t!("TeamManagement.members_count", count = members.len()).to_string()
                    }
                    _ => String::new(),
                }),
        );
        if can_manage_members(actor_role.as_deref()) {
            let tab = tab.clone();
            let team_id = team.id.clone();
            let actor_role = actor_role.clone();
            header_actions = header_actions.child(
                Button::new("team-member-invite")
                    .icon(IconName::Plus)
                    .label(t!("TeamManagement.add_member").to_string())
                    .small()
                    .on_click(move |_, window, cx| {
                        show_add_member_dialog(
                            tab.clone(),
                            team_id.clone(),
                            actor_role.clone(),
                            window,
                            cx,
                        );
                    }),
            );
        }
        let mut section = card(cx).child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("TeamManagement.members").to_string()),
                )
                .child(header_actions),
        );

        section = match &self.member_directory {
            MemberDirectory::Idle | MemberDirectory::Loading => section.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("TeamManagement.members_loading").to_string()),
            ),
            MemberDirectory::Failed(error) => section.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(t!("TeamManagement.members_load_failed", error = error).to_string()),
            ),
            MemberDirectory::Loaded(members) if members.is_empty() => section.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("TeamManagement.members_empty").to_string()),
            ),
            MemberDirectory::Loaded(members) => {
                let mut rows = v_flex().w_full().gap_2();
                for member in members {
                    rows = rows.child(render_member_row(
                        member,
                        actor_role.as_deref(),
                        current_user_id.as_deref(),
                        &tab,
                        &team.id,
                        cx,
                    ));
                }
                section.child(rows)
            }
        };

        section.into_any_element()
    }
}

impl Focusable for TeamTab {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for TeamTab {}

impl TabContent for TeamTab {
    fn content_key(&self) -> &'static str {
        "Team"
    }

    fn title(&self, _cx: &App) -> SharedString {
        t!("TeamManagement.title").into()
    }

    fn icon(&self, _cx: &App) -> Option<gpui_component::Icon> {
        Some(IconName::TeamLine.into())
    }
}

impl Render for TeamTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let teams = get_cached_team_options(cx);
        // 缓存里已经没有当前团队（被移出团队 / 退出登录）时回落到第一个。
        if let Some(selected) = self.selected_team_id.clone()
            && !teams.iter().any(|team| team.id == selected)
        {
            self.selected_team_id = teams.first().map(|team| team.id.clone());
            self.team_profile = None;
            self.member_directory = MemberDirectory::Idle;
        }

        let selected_team = self
            .selected_team_id
            .as_deref()
            .and_then(|selected| teams.iter().find(|team| team.id == selected).cloned());
        let entity = cx.entity();
        let (team_count, missing_key_count) = team_key_summary(&teams);

        // 左栏宽度固定，标题、统计与操作按钮各占一行：单行塞不下「新建团队 + 刷新团队列表」，
        // 溢出部分会被右侧详情面板盖住（只露出半截按钮）。
        let left = v_flex()
            .w(px(264.0))
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .p_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .text_sm()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("TeamManagement.title").to_string()),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .text_xs()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                t!(
                                    "TeamManagement.status_summary",
                                    teams = team_count,
                                    missing = missing_key_count
                                )
                                .to_string(),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .when(GlobalCloudUser::get_user(cx).is_some(), |this| {
                                let entity = entity.clone();
                                this.child(
                                    Button::new("team-tab-create")
                                        .icon(IconName::Plus)
                                        .label(t!("TeamManagement.create_team").to_string())
                                        .small()
                                        .on_click(move |_, window, cx| {
                                            show_team_create_dialog(entity.clone(), window, cx);
                                        }),
                                )
                            })
                            .child(
                                Button::new("team-tab-refresh")
                                    .icon(IconName::Refresh)
                                    .label(t!("TeamSync.refresh_teams").to_string())
                                    .small()
                                    .on_click(move |_, window, cx| {
                                        refresh_team_key_cache(window, cx);
                                        entity.update(cx, |team_tab, cx| {
                                            // 刷新是异步的：这里先把缓存里已有的团队重读一遍，
                                            // 后续缓存更新会通过全局通知事件驱动重绘。
                                            team_tab.sync_selection_from_cache(cx);
                                            cx.notify();
                                        });
                                    }),
                            ),
                    ),
            )
            // 滚动条必须挂在真正的滚动容器上：外层 flex 项只负责限高 + 裁切。
            .child(
                div()
                    .id("team-tab-list")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .size_full()
                            .overflow_y_scrollbar()
                            .p_2()
                            .child(self.render_team_list(&teams, cx)),
                    ),
            );

        let right = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(
                v_flex()
                    .id("team-tab-detail")
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(match selected_team.as_ref() {
                        Some(team) => self.render_team_detail(team, cx),
                        None => v_flex()
                            .w_full()
                            .p_6()
                            .gap_2()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t!("TeamManagement.no_teams").to_string()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("TeamManagement.no_teams_hint").to_string()),
                            )
                            .into_any_element(),
                    }),
            );

        h_flex().size_full().child(left).child(right)
    }
}

impl TeamTab {
    /// 缓存里的团队变化后，让选中项与成员目录保持一致。
    fn sync_selection_from_cache(&mut self, cx: &mut Context<Self>) {
        let teams = get_cached_team_options(cx);
        let still_exists = self
            .selected_team_id
            .as_deref()
            .is_some_and(|selected| teams.iter().any(|team| team.id == selected));
        if !still_exists {
            self.selected_team_id = teams.first().map(|team| team.id.clone());
            self.team_profile = None;
            self.member_directory = MemberDirectory::Idle;
        }
    }
}

fn card(cx: &App) -> gpui::Div {
    v_flex()
        .w_full()
        .gap_3()
        .p_4()
        .rounded(px(8.0))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
}

fn profile_meta_row(label: String, value: String, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .child(
            div()
                .w(px(96.0))
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().text_sm().child(value))
        .into_any_element()
}

/// 角色徽标：owner / admin / member 三档。
fn team_role_badge(role: Option<&str>, cx: &App) -> AnyElement {
    let label = team_role_label(role);
    // accent 是表面色（用作背景/强调填充），拿它当文字色在浅色主题下几乎看不清，
    // 所以 owner 用 primary，admin 用 info。
    let color = match role {
        Some("owner") => cx.theme().primary,
        Some("admin") => cx.theme().info,
        _ => cx.theme().muted_foreground,
    };
    div()
        .flex_shrink_0()
        .px_2()
        .py_0p5()
        .rounded(px(999.0))
        .bg(cx.theme().muted)
        .text_xs()
        .text_color(color)
        .child(label)
        .into_any_element()
}

/// 团队成员状态异常时的提示徽标。
fn membership_notice(team: &TeamOption, cx: &App) -> Option<AnyElement> {
    match team.membership_state {
        TeamMembershipState::Active => None,
        TeamMembershipState::Departed => Some(
            div()
                .text_xs()
                .text_color(cx.theme().warning)
                .child(t!("TeamSync.membership_departed").to_string())
                .into_any_element(),
        ),
        TeamMembershipState::Unknown => Some(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!("TeamSync.membership_unknown").to_string())
                .into_any_element(),
        ),
    }
}

/// 成员目录的一行：邮箱 + 角色徽标 + 加入日期 +（有权限时的）改角色 / 移除操作。
fn render_member_row(
    member: &TeamMemberWithEmail,
    actor_role: Option<&str>,
    current_user_id: Option<&str>,
    tab: &Entity<TeamTab>,
    team_id: &str,
    cx: &mut App,
) -> AnyElement {
    let is_current_user = current_user_id == Some(member.user_id.as_str());
    let can_change_role = can_change_member_role(actor_role, member.role);
    let can_remove = can_remove_member(actor_role, member.role, is_current_user);
    h_flex()
        .id(SharedString::from(format!("team-member-row-{}", member.id)))
        .w_full()
        .items_center()
        .gap_2()
        .p_2()
        .rounded(px(6.0))
        .bg(cx.theme().muted.opacity(0.4))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    h_flex()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .text_sm()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(member.email.clone()),
                        )
                        .when(is_current_user, |this| {
                            this.child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(cx.theme().primary)
                                    .child(t!("TeamManagement.member_you").to_string()),
                            )
                        }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "{} {}",
                            t!("TeamManagement.member_joined"),
                            unix_millis_to_date(member.joined_at)
                        )),
                ),
        )
        .child(team_role_badge(Some(member.role.as_str()), cx))
        .when(can_change_role, |this| {
            let tab = tab.clone();
            let team_id = team_id.to_string();
            let member = member.clone();
            let actor_role = actor_role.map(str::to_string);
            this.child(
                Button::new(SharedString::from(format!(
                    "team-member-role-{}",
                    member.id
                )))
                .label(t!("TeamManagement.change_role").to_string())
                .small()
                .on_click(move |_, window, cx| {
                    show_change_member_role_dialog(
                        tab.clone(),
                        team_id.clone(),
                        member.clone(),
                        actor_role.clone(),
                        window,
                        cx,
                    );
                }),
            )
        })
        .when(can_remove, |this| {
            let tab = tab.clone();
            let team_id = team_id.to_string();
            let member = member.clone();
            this.child(
                Button::new(SharedString::from(format!(
                    "team-member-remove-{}",
                    member.id
                )))
                .label(t!("TeamManagement.remove_member").to_string())
                .small()
                .on_click(move |_, window, cx| {
                    show_remove_member_dialog(
                        tab.clone(),
                        team_id.clone(),
                        member.clone(),
                        window,
                        cx,
                    );
                }),
            )
        })
        .into_any_element()
}

/// 团队密钥管理区：状态徽标 + 保存本机密钥 / 轮换 / 移除。
/// 刷新入口只在左栏顶部保留一个，这里不再重复放按钮。
fn render_team_key_management_section(team: &TeamOption, cx: &mut App) -> AnyElement {
    card(cx)
        .child(
            v_flex()
                .w_full()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("TeamSync.manage_keys").to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("TeamSync.page_desc").to_string()),
                ),
        )
        .child(render_team_key_row(team, cx))
        .when(
            matches!(team.key_status, TeamKeyCacheStatus::Missing),
            |this| {
                this.child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .p_3()
                        .rounded(px(8.0))
                        .bg(cx.theme().muted.opacity(0.4))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t!("TeamSync.empty_title").to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("TeamSync.key_help").to_string()),
                        ),
                )
            },
        )
        .into_any_element()
}

fn render_team_key_row(team: &TeamOption, cx: &mut App) -> AnyElement {
    let can_rotate = team_key_role_can_rotate(team.role.as_deref());
    let can_forget = !matches!(team.key_status, TeamKeyCacheStatus::Missing);
    let team_for_save = team.clone();
    let team_for_rotate = team.clone();
    let team_id_for_forget = team.id.clone();

    v_flex()
        .w_full()
        .gap_3()
        .p_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(8.0))
        .child(
            h_flex()
                .w_full()
                .items_start()
                .justify_between()
                .gap_3()
                .child(
                    v_flex()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(team.name.clone()),
                        )
                        .child(render_team_key_meta(team, cx)),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new(format!("team-key-save-{}", team.id))
                                .icon(IconName::Key)
                                .label(t!("TeamSync.save_local_key").to_string())
                                .small()
                                .on_click(move |_, window, cx| {
                                    show_team_key_entry_dialog(team_for_save.clone(), window, cx);
                                }),
                        )
                        .when(can_forget, |this| {
                            this.child(
                                Button::new(format!("team-key-forget-{}", team.id))
                                    .icon(IconName::Key)
                                    .label(t!("TeamSync.forget_local_key").to_string())
                                    .small()
                                    .danger()
                                    .on_click(move |_, window, cx| {
                                        forget_team_key(team_id_for_forget.clone(), window, cx);
                                    }),
                            )
                        })
                        .when(can_rotate, |this| {
                            this.child(
                                Button::new(format!("team-key-rotate-{}", team.id))
                                    .icon(IconName::Refresh)
                                    .label(t!("TeamSync.rotate_key").to_string())
                                    .small()
                                    .on_click(move |_, window, cx| {
                                        show_team_key_rotation_dialog(
                                            team_for_rotate.clone(),
                                            window,
                                            cx,
                                        );
                                    }),
                            )
                        }),
                ),
        )
        .into_any_element()
}

fn render_team_key_meta(team: &TeamOption, cx: &mut App) -> AnyElement {
    h_flex()
        .gap_3()
        .flex_wrap()
        .child(team_key_status_badge(team.key_status, cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{} {}", t!("TeamSync.version"), team.key_version)),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!(
                    "{} {}",
                    t!("TeamSync.role"),
                    team.role.as_deref().unwrap_or("-")
                )),
        )
        .into_any_element()
}

fn team_key_status_badge(status: TeamKeyCacheStatus, cx: &mut App) -> AnyElement {
    let (label, color) = match status {
        TeamKeyCacheStatus::Missing => (
            t!("TeamSync.status_missing").to_string(),
            cx.theme().warning,
        ),
        TeamKeyCacheStatus::Cached => {
            (t!("TeamSync.status_cached").to_string(), cx.theme().success)
        }
        TeamKeyCacheStatus::VersionMismatch => (
            t!("TeamSync.status_version_mismatch").to_string(),
            cx.theme().danger,
        ),
        TeamKeyCacheStatus::Invalid => {
            (t!("TeamSync.status_invalid").to_string(), cx.theme().danger)
        }
    };
    div()
        .text_xs()
        .text_color(color)
        .child(label)
        .into_any_element()
}

fn show_team_key_entry_dialog(team: TeamOption, window: &mut Window, cx: &mut App) {
    let team_id = team.id.clone();
    let team_name = team.name.clone();
    let key_input = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(t!("TeamSync.key_placeholder").to_string())
            .masked(true)
    });
    let error_message = cx.new(|_| Option::<String>::None);
    let key_input_for_ok = key_input.clone();
    let key_input_for_render = key_input.clone();
    let error_for_ok = error_message.clone();
    let error_for_render = error_message.clone();
    let team_for_ok = team.clone();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let team_id_ok = team_id.clone();
        let team_ok = team_for_ok.clone();
        let key_input_ok = key_input_for_ok.clone();
        let error_ok = error_for_ok.clone();
        dialog
            .title(format!("{} - {}", t!("TeamSync.save_local_key"), team_name))
            .width(gpui::px(460.))
            .confirm()
            .on_ok(move |_, window, cx: &mut App| {
                let team_key = key_input_ok.read(cx).text().to_string();
                if team_key.is_empty() {
                    set_team_key_dialog_error(&error_ok, t!("TeamSync.key_empty").to_string(), cx);
                    return false;
                }
                if team_ok.key_verification.is_none() {
                    if !team_key_role_can_rotate(team_ok.role.as_deref()) {
                        set_team_key_dialog_error(
                            &error_ok,
                            t!("TeamSync.initialize_requires_manager").to_string(),
                            cx,
                        );
                        return false;
                    }
                    save_or_initialize_team_key(
                        TeamKeySaveRequest::initialize(team_id_ok.clone(), team_key),
                        window,
                        cx,
                    );
                    return true;
                }
                save_or_initialize_team_key(
                    TeamKeySaveRequest::save(team_id_ok.clone(), team_key),
                    window,
                    cx,
                );
                true
            })
            .child(
                v_flex()
                    .gap_4()
                    .p_4()
                    .child(Input::new(&key_input_for_render).mask_toggle().w_full())
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("TeamSync.key_help").to_string()),
                    )
                    .when_some(error_for_render.read(cx).clone(), |this, msg| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(msg))
                    }),
            )
    });
}

fn show_team_key_rotation_dialog(team: TeamOption, window: &mut Window, cx: &mut App) {
    let team_id = team.id.clone();
    let team_name = team.name.clone();
    let old_key_input = team_key_input(window, cx, t!("TeamSync.key_placeholder").to_string());
    let new_key_input = team_key_input(window, cx, t!("TeamSync.new_key_placeholder").to_string());
    let error_message = cx.new(|_| Option::<String>::None);
    let old_for_ok = old_key_input.clone();
    let new_for_ok = new_key_input.clone();
    let old_for_render = old_key_input.clone();
    let new_for_render = new_key_input.clone();
    let error_for_ok = error_message.clone();
    let error_for_render = error_message.clone();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let team_id_ok = team_id.clone();
        let old_ok = old_for_ok.clone();
        let new_ok = new_for_ok.clone();
        let error_ok = error_for_ok.clone();
        dialog
            .title(format!("{} - {}", t!("TeamSync.rotate_key"), team_name))
            .width(gpui::px(500.))
            .confirm()
            .on_ok(move |_, window, cx: &mut App| {
                let old_key = old_ok.read(cx).text().to_string();
                let new_key = new_ok.read(cx).text().to_string();
                if !team_key_rotation_inputs_valid(&old_key, &new_key) {
                    set_team_key_dialog_error(
                        &error_ok,
                        t!("TeamSync.rotate_key_empty").to_string(),
                        cx,
                    );
                    return false;
                }
                rotate_team_key(team_id_ok.clone(), old_key, new_key, window, cx);
                true
            })
            .child(
                v_flex()
                    .gap_4()
                    .p_4()
                    .child(Input::new(&old_for_render).mask_toggle().w_full())
                    .child(Input::new(&new_for_render).mask_toggle().w_full())
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("TeamSync.rotate_help").to_string()),
                    )
                    .when_some(error_for_render.read(cx).clone(), |this, msg| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(msg))
                    }),
            )
    });
}

fn team_key_input(window: &mut Window, cx: &mut App, placeholder: String) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(true)
    })
}

fn team_key_rotation_inputs_valid(old_key: &str, new_key: &str) -> bool {
    !old_key.is_empty() && !new_key.is_empty()
}

fn set_team_key_dialog_error(error: &Entity<Option<String>>, message: String, cx: &mut App) {
    error.update(cx, |msg, cx| {
        *msg = Some(message);
        cx.notify();
    });
}

fn team_key_change_completed(window: &mut Window, cx: &mut App) {
    if !cfg!(test) {
        window.refresh();
    }
    emit_team_key_change_event(cx);
}

fn emit_team_key_change_event(cx: &mut App) {
    let Some(notifier) = get_notifier(cx) else {
        tracing::warn!("团队密钥状态变化后无法通知首页同步：GlobalConnectionNotifier 未初始化");
        return;
    };
    notifier.update(cx, |_, cx| {
        cx.emit(ConnectionDataEvent::CloudSyncRequested);
    });
}

fn team_key_refresh_success_message(count: usize) -> String {
    t!("TeamSync.refresh_success", count = count).to_string()
}

fn refresh_team_key_cache(window: &mut Window, cx: &mut App) {
    let Some(user) = GlobalCloudUser::get_user(cx) else {
        window.push_notification(t!("Home.cloud_need_login").to_string(), cx);
        return;
    };
    let Some(storage) = cx.try_global::<GlobalStorageState>() else {
        window.push_notification(t!("Common.storage_unavailable").to_string(), cx);
        return;
    };
    let sync_service = Arc::new(std::sync::RwLock::new(CloudSyncService::new()));
    if let Ok(mut service) = sync_service.write() {
        service.set_logged_in(user.id);
    }
    let engine = SyncEngine::new(
        get_auth_service(cx).cloud_client(),
        sync_service,
        storage.storage.clone(),
    );
    window.push_notification(t!("TeamSync.refresh_started").to_string(), cx);
    window
        .spawn(cx, async move |cx| {
            let result = engine.refresh_team_key_cache().await;
            let (message, completed_event) = match result {
                Ok(count) => (
                    team_key_refresh_success_message(count),
                    Some(ConnectionDataEvent::TeamCacheUpdated),
                ),
                Err(error) => (error.to_string(), None),
            };
            if let Err(error) = cx.update(|window, cx: &mut App| {
                window.push_notification(message, cx);
                if let Some(event) = completed_event
                    && let Some(notifier) = get_notifier(cx)
                {
                    notifier.update(cx, |_, cx| {
                        cx.emit(event);
                    });
                }
                window.refresh();
            }) {
                tracing::warn!("团队列表刷新完成后更新窗口失败: {error}");
            }
        })
        .detach();
}

struct TeamKeySaveRequest {
    team_id: String,
    team_key: String,
    started_message: Option<String>,
    success_message: String,
}

impl TeamKeySaveRequest {
    fn initialize(team_id: String, team_key: String) -> Self {
        Self {
            team_id,
            team_key,
            started_message: Some(t!("TeamSync.initialize_started").to_string()),
            success_message: t!("TeamSync.initialize_success").to_string(),
        }
    }

    fn save(team_id: String, team_key: String) -> Self {
        Self {
            team_id,
            team_key,
            started_message: None,
            success_message: t!("TeamSync.save_success").to_string(),
        }
    }
}

fn save_or_initialize_team_key(request: TeamKeySaveRequest, window: &mut Window, cx: &mut App) {
    let Some(personal_key) = crypto::get_raw_master_key() else {
        window.push_notification(t!("Encryption.key_locked_tooltip").to_string(), cx);
        return;
    };
    let Some(engine) = team_key_engine(window, cx) else {
        return;
    };
    if let Some(message) = request.started_message {
        window.push_notification(message, cx);
    }
    window
        .spawn(cx, async move |cx| {
            let result = engine
                .save_or_initialize_team_key_for_cached_team(
                    &request.team_id,
                    &request.team_key,
                    &personal_key,
                )
                .await;
            let (message, key_changed) = match result {
                Ok(_) => (request.success_message, true),
                Err(error) => (error.to_string(), false),
            };
            if let Err(error) = cx.update(|window, cx: &mut App| {
                window.push_notification(message, cx);
                if key_changed {
                    team_key_change_completed(window, cx);
                } else {
                    window.refresh();
                }
            }) {
                tracing::warn!("团队密钥保存完成后更新窗口失败: {error}");
            }
        })
        .detach();
}

fn forget_team_key(team_id: String, window: &mut Window, cx: &mut App) {
    let Some(engine) = team_key_engine(window, cx) else {
        return;
    };
    window
        .spawn(cx, async move |cx| {
            let result = engine.forget_team_key_for_cached_team(&team_id).await;
            let (message, key_changed) = match result {
                Ok(()) => (t!("TeamSync.forget_success").to_string(), true),
                Err(error) => (error.to_string(), false),
            };
            if let Err(error) = cx.update(|window, cx: &mut App| {
                window.push_notification(message, cx);
                if key_changed {
                    team_key_change_completed(window, cx);
                } else {
                    window.refresh();
                }
            }) {
                tracing::warn!("团队密钥移除完成后更新窗口失败: {error}");
            }
        })
        .detach();
}

fn team_key_engine(window: &mut Window, cx: &mut App) -> Option<SyncEngine> {
    let Some(user) = GlobalCloudUser::get_user(cx) else {
        window.push_notification(t!("Home.cloud_need_login").to_string(), cx);
        return None;
    };
    let Some(storage) = cx.try_global::<GlobalStorageState>() else {
        window.push_notification(t!("Common.storage_unavailable").to_string(), cx);
        return None;
    };
    let sync_service = Arc::new(std::sync::RwLock::new(CloudSyncService::new()));
    if let Ok(mut service) = sync_service.write() {
        service.set_logged_in(user.id);
    }
    Some(SyncEngine::new(
        get_auth_service(cx).cloud_client(),
        sync_service,
        storage.storage.clone(),
    ))
}

fn rotate_team_key(
    team_id: String,
    old_key: String,
    new_key: String,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(user) = GlobalCloudUser::get_user(cx) else {
        window.push_notification(t!("Home.cloud_need_login").to_string(), cx);
        return;
    };
    let Some(storage) = cx.try_global::<GlobalStorageState>() else {
        window.push_notification(t!("Common.storage_unavailable").to_string(), cx);
        return;
    };
    let sync_service = Arc::new(std::sync::RwLock::new(CloudSyncService::new()));
    if let Ok(mut service) = sync_service.write() {
        service.set_logged_in(user.id);
    }
    let engine = SyncEngine::new(
        get_auth_service(cx).cloud_client(),
        sync_service,
        storage.storage.clone(),
    );
    window.push_notification(t!("TeamSync.rotate_started").to_string(), cx);
    window
        .spawn(cx, async move |cx| {
            let result = engine.rotate_team_key(&team_id, &old_key, &new_key).await;
            let (message, key_changed) = match result {
                Ok(rotation) => (
                    t!(
                        "TeamSync.rotate_success",
                        count = rotation.re_encrypted,
                        version = rotation.key_version
                    )
                    .to_string(),
                    true,
                ),
                Err(error) => (error.to_string(), false),
            };
            if let Err(error) = cx.update(|window, cx: &mut App| {
                window.push_notification(message, cx);
                if key_changed {
                    team_key_change_completed(window, cx);
                } else {
                    window.refresh();
                }
            }) {
                tracing::warn!("团队密钥轮换完成后更新窗口失败: {error}");
            }
        })
        .detach();
}

// ========================================================================
// 团队管理写操作（团队资料 / 成员 / 邀请）
// ========================================================================
//
// 权限语义严格对齐线上 RPC（错误文案由 crates/core 的 team_admin 模块映射成中文）：
// - owner：可改团队资料、可改任意成员角色（含 owner）、可移除 admin 与 member；
// - admin：团队资料只读，只能把 member 改成 admin/member，只能移除 member；
// - owner 永远不能被直接移除；成员也不能移除自己（RPC 不支持自助退出团队）。

/// 团队资料只有 owner 能改：RLS 只放行 owner，admin 写入会是 0 行假成功。
fn can_edit_team_profile(actor_role: Option<&str>) -> bool {
    actor_role == Some("owner")
}

/// 可以邀请成员、管理成员的角色。
fn can_manage_members(actor_role: Option<&str>) -> bool {
    matches!(actor_role, Some("owner" | "admin"))
}

/// 当前角色可以授予的角色集合；空表示没有增员权限。
fn assignable_team_roles(actor_role: Option<&str>) -> Vec<TeamRole> {
    match actor_role {
        Some("owner") => vec![TeamRole::Owner, TeamRole::Admin, TeamRole::Member],
        Some("admin") => vec![TeamRole::Admin, TeamRole::Member],
        _ => Vec::new(),
    }
}

/// 改角色：owner 可以改任何人；admin 只能动 member，且不能授 owner。
fn can_change_member_role(actor_role: Option<&str>, target_role: TeamRole) -> bool {
    match actor_role {
        Some("owner") => true,
        Some("admin") => target_role == TeamRole::Member,
        _ => false,
    }
}

/// 移除成员：owner 不可移除；移除 admin 只有 owner 能做；不提供「移除自己」。
fn can_remove_member(actor_role: Option<&str>, target_role: TeamRole, is_self: bool) -> bool {
    if target_role == TeamRole::Owner || is_self {
        return false;
    }
    match actor_role {
        Some("owner") => true,
        Some("admin") => target_role == TeamRole::Member,
        _ => false,
    }
}

/// 不带掩码的文本输入框（团队名 / 描述 / 邮箱）。
fn team_plain_input(
    window: &mut Window,
    cx: &mut App,
    placeholder: String,
    default_value: Option<String>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let input = InputState::new(window, cx).placeholder(placeholder);
        match default_value {
            Some(value) => input.default_value(value),
            None => input,
        }
    })
}

/// 角色单选胶囊：点一下把选择写回 `role_entity`，对话框会跟着重绘。
fn role_choice_chips(
    roles: &[TeamRole],
    selected: TeamRole,
    role_entity: &Entity<TeamRole>,
    cx: &App,
) -> AnyElement {
    let mut row = h_flex().w_full().flex_wrap().gap_2();
    for role in roles.iter().copied() {
        let is_selected = role == selected;
        let entity = role_entity.clone();
        row = row.child(
            div()
                .id(SharedString::from(format!(
                    "team-role-choice-{}",
                    role.as_str()
                )))
                .px_3()
                .py_1()
                .rounded(px(999.0))
                .border_1()
                .border_color(if is_selected {
                    cx.theme().accent
                } else {
                    cx.theme().border
                })
                .bg(if is_selected {
                    cx.theme().accent.opacity(0.15)
                } else {
                    cx.theme().background
                })
                .cursor_pointer()
                .text_sm()
                .child(team_role_label(Some(role.as_str())))
                .on_click(move |_, _window, cx| {
                    if entity.read(cx) == &role {
                        return;
                    }
                    entity.update(cx, |current, cx| {
                        *current = role;
                        cx.notify();
                    });
                }),
        );
    }
    row.into_any_element()
}

/// 写操作成功后的统一收尾：重载当前团队详情，并通知其它页面刷新缓存。
fn after_team_admin_change(tab: &Entity<TeamTab>, window: &mut Window, cx: &mut App) {
    tab.update(cx, |team_tab, cx| {
        team_tab.load_selected_team(window, cx);
        cx.notify();
    });
    team_key_change_completed(window, cx);
}

/// 建团队 / 改资料的字段上限，与网站团队表单保持一致。
const TEAM_NAME_MAX_CHARS: usize = 80;
const TEAM_DESCRIPTION_MAX_CHARS: usize = 500;

fn team_name_exceeds_limit(name: &str) -> bool {
    name.chars().count() > TEAM_NAME_MAX_CHARS
}

fn team_description_exceeds_limit(description: &str) -> bool {
    description.chars().count() > TEAM_DESCRIPTION_MAX_CHARS
}

fn show_team_create_dialog(tab: Entity<TeamTab>, window: &mut Window, cx: &mut App) {
    let name_input = team_plain_input(
        window,
        cx,
        t!("TeamManagement.team_name_placeholder").to_string(),
        None,
    );
    let description_input = team_plain_input(
        window,
        cx,
        t!("TeamManagement.team_description_placeholder").to_string(),
        None,
    );
    let error_message = cx.new(|_| Option::<String>::None);
    let name_for_ok = name_input.clone();
    let description_for_ok = description_input.clone();
    let name_for_render = name_input.clone();
    let description_for_render = description_input.clone();
    let error_for_ok = error_message.clone();
    let error_for_render = error_message.clone();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let name_ok = name_for_ok.clone();
        let description_ok = description_for_ok.clone();
        let error_ok = error_for_ok.clone();
        let tab_ok = tab.clone();
        dialog
            .title(t!("TeamManagement.create_team_title").to_string())
            .width(px(480.))
            .confirm()
            .on_ok(move |_, window, cx: &mut App| {
                let name = name_ok.read(cx).text().to_string().trim().to_string();
                if name.is_empty() {
                    set_team_key_dialog_error(
                        &error_ok,
                        t!("TeamManagement.name_required").to_string(),
                        cx,
                    );
                    return false;
                }
                let description = description_ok
                    .read(cx)
                    .text()
                    .to_string()
                    .trim()
                    .to_string();
                if team_name_exceeds_limit(&name) {
                    set_team_key_dialog_error(
                        &error_ok,
                        t!("TeamManagement.name_too_long", max = TEAM_NAME_MAX_CHARS).to_string(),
                        cx,
                    );
                    return false;
                }
                if team_description_exceeds_limit(&description) {
                    set_team_key_dialog_error(
                        &error_ok,
                        t!(
                            "TeamManagement.description_too_long",
                            max = TEAM_DESCRIPTION_MAX_CHARS
                        )
                        .to_string(),
                        cx,
                    );
                    return false;
                }
                let Some(engine) = team_key_engine(window, cx) else {
                    return false;
                };
                let tab = tab_ok.clone();
                window
                    .spawn(cx, async move |cx| {
                        let description = (!description.is_empty()).then_some(description);
                        let result = engine.create_team(&name, description.as_deref()).await;
                        if let Err(error) = cx.update(|window, cx: &mut App| match result {
                            Ok(team_id) => {
                                window.push_notification(
                                    t!("TeamManagement.team_created").to_string(),
                                    cx,
                                );
                                tab.update(cx, |team_tab, cx| {
                                    team_tab.selected_team_id = Some(team_id);
                                    team_tab.load_selected_team(window, cx);
                                    cx.notify();
                                });
                                team_key_change_completed(window, cx);
                            }
                            Err(error) => window.push_notification(error.to_string(), cx),
                        }) {
                            tracing::warn!("创建团队后更新窗口失败: {error}");
                        }
                    })
                    .detach();
                true
            })
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .child(Input::new(&name_for_render).w_full())
                    .child(Input::new(&description_for_render).w_full())
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("TeamManagement.team_description_hint").to_string()),
                    )
                    .when_some(error_for_render.read(cx).clone(), |this, message| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(message))
                    }),
            )
    });
}

fn show_team_profile_dialog(
    tab: Entity<TeamTab>,
    team_id: String,
    team_name: String,
    description: String,
    window: &mut Window,
    cx: &mut App,
) {
    let name_input = team_plain_input(
        window,
        cx,
        t!("TeamManagement.team_name_placeholder").to_string(),
        Some(team_name),
    );
    let description_input = team_plain_input(
        window,
        cx,
        t!("TeamManagement.team_description_placeholder").to_string(),
        Some(description),
    );
    let error_message = cx.new(|_| Option::<String>::None);
    let name_for_ok = name_input.clone();
    let description_for_ok = description_input.clone();
    let name_for_render = name_input.clone();
    let description_for_render = description_input.clone();
    let error_for_ok = error_message.clone();
    let error_for_render = error_message.clone();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let name_ok = name_for_ok.clone();
        let description_ok = description_for_ok.clone();
        let error_ok = error_for_ok.clone();
        let tab_ok = tab.clone();
        let team_id_ok = team_id.clone();
        dialog
            .title(t!("TeamManagement.edit_profile_title").to_string())
            .width(px(480.))
            .confirm()
            .on_ok(move |_, window, cx: &mut App| {
                let name = name_ok.read(cx).text().to_string().trim().to_string();
                if name.is_empty() {
                    set_team_key_dialog_error(
                        &error_ok,
                        t!("TeamManagement.name_required").to_string(),
                        cx,
                    );
                    return false;
                }
                let description = description_ok
                    .read(cx)
                    .text()
                    .to_string()
                    .trim()
                    .to_string();
                if team_name_exceeds_limit(&name) {
                    set_team_key_dialog_error(
                        &error_ok,
                        t!("TeamManagement.name_too_long", max = TEAM_NAME_MAX_CHARS).to_string(),
                        cx,
                    );
                    return false;
                }
                if team_description_exceeds_limit(&description) {
                    set_team_key_dialog_error(
                        &error_ok,
                        t!(
                            "TeamManagement.description_too_long",
                            max = TEAM_DESCRIPTION_MAX_CHARS
                        )
                        .to_string(),
                        cx,
                    );
                    return false;
                }
                let Some(engine) = team_key_engine(window, cx) else {
                    return false;
                };
                let tab = tab_ok.clone();
                let team_id = team_id_ok.clone();
                window
                    .spawn(cx, async move |cx| {
                        let description = (!description.is_empty()).then_some(description);
                        let result = engine
                            .update_team_profile(&team_id, &name, description.as_deref())
                            .await;
                        if let Err(error) = cx.update(|window, cx: &mut App| match result {
                            Ok(()) => {
                                window.push_notification(
                                    t!("TeamManagement.profile_saved").to_string(),
                                    cx,
                                );
                                after_team_admin_change(&tab, window, cx);
                            }
                            Err(error) => window.push_notification(error.to_string(), cx),
                        }) {
                            tracing::warn!("更新团队资料后更新窗口失败: {error}");
                        }
                    })
                    .detach();
                true
            })
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .child(Input::new(&name_for_render).w_full())
                    .child(Input::new(&description_for_render).w_full())
                    .when_some(error_for_render.read(cx).clone(), |this, message| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(message))
                    }),
            )
    });
}

fn show_add_member_dialog(
    tab: Entity<TeamTab>,
    team_id: String,
    actor_role: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let roles = assignable_team_roles(actor_role.as_deref());
    if roles.is_empty() {
        return;
    }
    let default_role = roles
        .iter()
        .copied()
        .find(|role| *role == TeamRole::Member)
        .unwrap_or(TeamRole::Member);
    let role_entity = cx.new(|_| default_role);
    let email_input = team_plain_input(
        window,
        cx,
        t!("TeamManagement.member_email_placeholder").to_string(),
        None,
    );
    let error_message = cx.new(|_| Option::<String>::None);
    let email_for_ok = email_input.clone();
    let email_for_render = email_input.clone();
    let error_for_ok = error_message.clone();
    let error_for_render = error_message.clone();
    let role_for_ok = role_entity.clone();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let email_ok = email_for_ok.clone();
        let error_ok = error_for_ok.clone();
        let role_ok = role_for_ok.clone();
        let tab_ok = tab.clone();
        let team_id_ok = team_id.clone();
        let roles = roles.clone();
        let selected = *role_entity.read(cx);
        dialog
            .title(t!("TeamManagement.add_member_title").to_string())
            .width(px(480.))
            .confirm()
            .on_ok(move |_, window, cx: &mut App| {
                let email = email_ok.read(cx).text().to_string().trim().to_string();
                if email.is_empty() {
                    set_team_key_dialog_error(
                        &error_ok,
                        t!("TeamManagement.email_required").to_string(),
                        cx,
                    );
                    return false;
                }
                if !email.contains('@') {
                    set_team_key_dialog_error(
                        &error_ok,
                        t!("TeamManagement.email_invalid").to_string(),
                        cx,
                    );
                    return false;
                }
                let Some(engine) = team_key_engine(window, cx) else {
                    return false;
                };
                let role = *role_ok.read(cx);
                let tab = tab_ok.clone();
                let team_id = team_id_ok.clone();
                window
                    .spawn(cx, async move |cx| {
                        let result = engine.add_team_member(&team_id, &email, role).await;
                        if let Err(error) = cx.update(|window, cx: &mut App| match result {
                            Ok(()) => {
                                window.push_notification(
                                    t!("TeamManagement.member_added").to_string(),
                                    cx,
                                );
                                after_team_admin_change(&tab, window, cx);
                            }
                            Err(error) => window.push_notification(error.to_string(), cx),
                        }) {
                            tracing::warn!("邀请团队成员后更新窗口失败: {error}");
                        }
                    })
                    .detach();
                true
            })
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .child(Input::new(&email_for_render).w_full())
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("TeamManagement.member_role_pick").to_string()),
                    )
                    .child(role_choice_chips(&roles, selected, &role_entity, cx))
                    .when_some(error_for_render.read(cx).clone(), |this, message| {
                        this.child(div().text_sm().text_color(cx.theme().danger).child(message))
                    }),
            )
    });
}

fn show_change_member_role_dialog(
    tab: Entity<TeamTab>,
    team_id: String,
    member: TeamMemberWithEmail,
    actor_role: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let roles = assignable_team_roles(actor_role.as_deref());
    if roles.is_empty() || !roles.contains(&member.role) {
        // 当前角色的权限不允许改这个成员，或者目标角色不在可授予集合里（防御性兜底）。
        if !can_change_member_role(actor_role.as_deref(), member.role) {
            return;
        }
    }
    let role_entity = cx.new(|_| member.role);
    let role_for_ok = role_entity.clone();
    let member_for_title = member.email.clone();
    let member_id = member.id.clone();

    window.open_dialog(cx, move |dialog, _window, cx| {
        let role_ok = role_for_ok.clone();
        let tab_ok = tab.clone();
        let team_id_ok = team_id.clone();
        let member_id_ok = member_id.clone();
        let roles = roles.clone();
        let selected = *role_entity.read(cx);
        dialog
            .title(format!(
                "{} - {}",
                t!("TeamManagement.change_role_title"),
                member_for_title
            ))
            .width(px(420.))
            .confirm()
            .on_ok(move |_, window, cx: &mut App| {
                let Some(engine) = team_key_engine(window, cx) else {
                    return false;
                };
                let role = *role_ok.read(cx);
                let tab = tab_ok.clone();
                let team_id = team_id_ok.clone();
                let member_id = member_id_ok.clone();
                window
                    .spawn(cx, async move |cx| {
                        let result = engine
                            .set_team_member_role(&team_id, &member_id, role)
                            .await;
                        if let Err(error) = cx.update(|window, cx: &mut App| match result {
                            Ok(()) => {
                                window.push_notification(
                                    t!("TeamManagement.role_changed").to_string(),
                                    cx,
                                );
                                after_team_admin_change(&tab, window, cx);
                            }
                            Err(error) => window.push_notification(error.to_string(), cx),
                        }) {
                            tracing::warn!("修改成员角色后更新窗口失败: {error}");
                        }
                    })
                    .detach();
                true
            })
            .child(v_flex().gap_3().p_4().child(role_choice_chips(
                &roles,
                selected,
                &role_entity,
                cx,
            )))
    });
}

fn show_remove_member_dialog(
    tab: Entity<TeamTab>,
    team_id: String,
    member: TeamMemberWithEmail,
    window: &mut Window,
    cx: &mut App,
) {
    let email = member.email.clone();
    let member_id = member.id.clone();

    window.open_dialog(cx, move |dialog, _window, _cx| {
        let tab_ok = tab.clone();
        let team_id_ok = team_id.clone();
        let member_id_ok = member_id.clone();
        dialog
            .title(t!("TeamManagement.remove_member_title").to_string())
            .width(px(420.))
            .confirm()
            .on_ok(move |_, window, cx: &mut App| {
                let Some(engine) = team_key_engine(window, cx) else {
                    return false;
                };
                let tab = tab_ok.clone();
                let team_id = team_id_ok.clone();
                let member_id = member_id_ok.clone();
                window
                    .spawn(cx, async move |cx| {
                        let result = engine.remove_team_member(&team_id, &member_id).await;
                        if let Err(error) = cx.update(|window, cx: &mut App| match result {
                            Ok(()) => {
                                window.push_notification(
                                    t!("TeamManagement.member_removed").to_string(),
                                    cx,
                                );
                                after_team_admin_change(&tab, window, cx);
                            }
                            Err(error) => window.push_notification(error.to_string(), cx),
                        }) {
                            tracing::warn!("移除团队成员后更新窗口失败: {error}");
                        }
                    })
                    .detach();
                true
            })
            .child(
                div().p_4().text_sm().child(
                    t!(
                        "TeamManagement.remove_member_confirm",
                        email = email.clone()
                    )
                    .to_string(),
                ),
            )
    });
}

fn team_key_role_can_rotate(role: Option<&str>) -> bool {
    matches!(role, Some("owner" | "admin"))
}

/// 团队总数与「缺本机密钥」的团队数（供侧栏状态行与团队页标题复用）。
pub(crate) fn team_key_summary(teams: &[TeamOption]) -> (usize, usize) {
    let missing = teams
        .iter()
        .filter(|team| !matches!(team.key_status, TeamKeyCacheStatus::Cached))
        .count();
    (teams.len(), missing)
}

/// 角色文案：服务端返回的是 `owner` / `admin` / `member` 原始串。
fn team_role_label(role: Option<&str>) -> String {
    match role {
        Some("owner") => t!("TeamManagement.role_owner").to_string(),
        Some("admin") => t!("TeamManagement.role_admin").to_string(),
        Some("member") => t!("TeamManagement.role_member").to_string(),
        Some(other) => other.to_string(),
        None => "-".to_string(),
    }
}

/// 毫秒时间戳 → 本机日期；非法值退化成占位符，不让脏数据显示成 1970。
fn unix_millis_to_date(millis: i64) -> String {
    DateTime::from_timestamp_millis(millis)
        .map(|time| time.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// UUID 只在资料卡里露出前 8 位，避免整块界面被 36 个字符撑开。
fn shorten_user_id(user_id: &str) -> String {
    let trimmed = user_id.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return "-".to_string();
    }
    trimmed.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;
    use one_core::cloud_sync::TeamRole;
    use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
    use std::sync::{Arc, Mutex};

    /// rust_i18n 的 locale 是进程级全局状态，并行测试会互相踩，必须串行。
    static LOCALE_LOCK: Mutex<()> = Mutex::new(());

    use super::{
        MemberDirectory, TeamKeyCacheStatus, TeamMembershipState, TeamOption,
        assignable_team_roles, can_change_member_role, can_edit_team_profile, can_manage_members,
        can_remove_member, emit_team_key_change_event, team_description_exceeds_limit,
        team_key_refresh_success_message, team_key_role_can_rotate, team_key_rotation_inputs_valid,
        team_key_summary, team_name_exceeds_limit, team_role_label, unix_millis_to_date,
    };

    /// `include_str!` 的源码契约必须先截掉测试模块本身，否则断言会命中测试自己写的字面量。
    fn implementation_source() -> &'static str {
        include_str!("team_tab.rs")
            .split_once("\n#[cfg(test)]\nmod tests {")
            .map(|(implementation, _)| implementation)
            .expect("team_tab.rs has a tests module")
    }

    fn team_option(id: &str, status: TeamKeyCacheStatus) -> TeamOption {
        TeamOption {
            id: id.to_string(),
            name: format!("Team {id}"),
            key_status: status,
            key_version: 1,
            key_verification: Some("verification".to_string()),
            last_verified_at: None,
            role: Some("owner".to_string()),
            membership_state: TeamMembershipState::Active,
        }
    }

    #[test]
    fn team_key_rotation_allows_same_passphrase() {
        assert!(team_key_rotation_inputs_valid(
            "same secure passphrase",
            "same secure passphrase"
        ));
        assert!(!team_key_rotation_inputs_valid("", "new secure passphrase"));
    }

    #[test]
    fn only_owners_and_admins_can_rotate_team_keys() {
        assert!(team_key_role_can_rotate(Some("owner")));
        assert!(team_key_role_can_rotate(Some("admin")));
        assert!(!team_key_role_can_rotate(Some("member")));
        assert!(!team_key_role_can_rotate(None));
    }

    #[test]
    fn team_key_refresh_success_message_reports_cached_count() {
        let _guard = LOCALE_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        rust_i18n::set_locale("en");

        assert_eq!(
            "Team list refreshed. 2 teams cached.",
            team_key_refresh_success_message(2)
        );
    }

    #[test]
    fn team_profile_fields_enforce_char_limits() {
        // 后端按字符计（不是字节），中文团队名不能因为 UTF-8 被多算。
        assert!(!team_name_exceeds_limit(&"团".repeat(80)));
        assert!(team_name_exceeds_limit(&"团".repeat(81)));
        assert!(!team_description_exceeds_limit(&"描".repeat(500)));
        assert!(team_description_exceeds_limit(&"描".repeat(501)));
        assert!(!team_name_exceeds_limit(""));
    }

    #[test]
    fn team_key_summary_counts_teams_that_miss_local_keys() {
        let teams = vec![
            team_option("a", TeamKeyCacheStatus::Cached),
            team_option("b", TeamKeyCacheStatus::Missing),
            team_option("c", TeamKeyCacheStatus::VersionMismatch),
        ];

        assert_eq!((3, 2), team_key_summary(&teams));
        assert_eq!((0, 0), team_key_summary(&[]));
    }

    #[test]
    fn team_key_cache_refresh_only_announces_update_after_success() {
        let source = implementation_source();
        let refresh = source
            .rsplit("fn refresh_team_key_cache(")
            .next()
            .expect("team key cache refresh exists")
            .split("struct TeamKeySaveRequest")
            .next()
            .expect("team key cache refresh has an end marker");

        let result_match = refresh
            .split("match result")
            .nth(1)
            .expect("refresh result is matched");
        let success = result_match
            .split("Ok(")
            .nth(1)
            .expect("refresh success branch exists")
            .split("Err(")
            .next()
            .expect("refresh success branch has an end marker");
        assert!(success.contains("ConnectionDataEvent::TeamCacheUpdated"));

        let failure = result_match
            .split("Err(")
            .nth(1)
            .expect("refresh failure branch exists")
            .split("};")
            .next()
            .expect("refresh failure branch has an end marker");
        assert!(!failure.contains("ConnectionDataEvent::TeamCacheUpdated"));
    }

    #[test]
    fn async_team_key_operations_notify_through_the_current_window() {
        let source = implementation_source();
        for (function, next_function) in [
            ("fn refresh_team_key_cache(", "struct TeamKeySaveRequest"),
            ("fn save_or_initialize_team_key(", "fn forget_team_key("),
            ("fn forget_team_key(", "fn team_key_engine("),
            ("fn rotate_team_key(", "fn team_key_role_can_rotate("),
        ] {
            let body = source
                .split(function)
                .nth(1)
                .expect("team key operation exists")
                .split(next_function)
                .next()
                .expect("team key operation has an end marker");

            assert!(!body.contains("update_window("), "{function}");
            assert!(
                body.contains("cx.update(|window, cx: &mut App|"),
                "{function}"
            );
            assert!(body.contains("window.push_notification(message, cx)"));
        }
    }

    #[test]
    fn successful_team_key_changes_request_runtime_reload() {
        let production = implementation_source();

        assert!(production.contains("fn team_key_change_completed("));
        assert!(production.contains("ConnectionDataEvent::CloudSyncRequested"));
        let entry_dialog = production
            .split("fn show_team_key_entry_dialog(")
            .nth(1)
            .expect("team key entry dialog exists")
            .split("fn show_team_key_rotation_dialog(")
            .next()
            .expect("team key entry dialog has an end marker");
        assert!(entry_dialog.contains("save_or_initialize_team_key("));
        for (function, next_function) in [
            ("fn save_or_initialize_team_key(", "fn forget_team_key("),
            ("fn forget_team_key(", "fn team_key_engine("),
            ("fn rotate_team_key(", "fn team_key_role_can_rotate("),
        ] {
            let body = production
                .split(function)
                .nth(1)
                .expect("team key change function exists")
                .split(next_function)
                .next()
                .expect("team key change function has an end marker");

            assert!(body.contains("team_key_change_completed("), "{function}");
        }
    }

    #[gpui::test]
    fn team_key_change_emits_cloud_sync_request(cx: &mut TestAppContext) {
        let events = Arc::new(Mutex::new(Vec::<ConnectionDataEvent>::new()));
        let events_for_subscription = events.clone();
        let _subscription = cx.update(|cx| {
            one_core::connection_notifier::init(cx);
            let notifier = get_notifier(cx).expect("connection notifier initialized");
            let subscription = cx.subscribe(&notifier, move |_, event, _| {
                events_for_subscription
                    .lock()
                    .expect("events lock")
                    .push(event.clone());
            });
            subscription
        });

        cx.update(|cx| emit_team_key_change_event(cx));

        assert!(
            events
                .lock()
                .expect("events lock")
                .iter()
                .any(|event| matches!(event, ConnectionDataEvent::CloudSyncRequested))
        );
    }

    #[test]
    fn team_tab_exposes_profile_key_and_member_sections() {
        let source = implementation_source();

        for section in [
            "fn render_profile_section(",
            "fn render_team_key_management_section(",
            "fn render_member_section(",
            "fn render_member_row(",
            "fn team_role_badge(",
        ] {
            assert!(source.contains(section), "团队成员/密钥视图缺失: {section}");
        }
        // 左栏列表要同时露出密钥状态和角色，否则用户得进详情才知道缺密钥。
        let list = source
            .split("fn render_team_list(")
            .nth(1)
            .expect("团队列表存在")
            .split("fn render_team_detail(")
            .next()
            .expect("团队列表有结尾");
        assert!(list.contains("team_key_status_badge("));
        assert!(list.contains("team_role_badge("));
        // 详情页先呈现资料、再是密钥、最后是成员，顺序决定用户视线。
        let detail = source
            .split("fn render_team_detail(")
            .nth(1)
            .expect("团队详情存在")
            .split("fn render_profile_section(")
            .next()
            .expect("团队详情有结尾");
        let profile_ix = detail.find("render_profile_section").expect("资料区");
        let key_ix = detail
            .find("render_team_key_management_section")
            .expect("密钥区");
        let member_ix = detail.find("render_member_section").expect("成员区");
        assert!(profile_ix < key_ix && key_ix < member_ix);
    }

    /// 左栏团队列表和右栏详情都必须能滚动：外层 flex 项只负责限高裁切，
    /// 滚动条挂在真正的滚动容器上，而且要用 overflow_y_scrollbar ——
    /// overflow_y_scroll 不画滚动条，内容超高时用户根本看不出还能滚。
    #[test]
    fn team_tab_scroll_regions_keep_visible_scrollbars() {
        let source = implementation_source();

        let left = source
            .split("let left = v_flex()")
            .nth(1)
            .expect("左栏存在")
            .split("let right = v_flex()")
            .next()
            .expect("左栏有结尾");
        assert!(left.contains(".id(\"team-tab-list\")"));
        assert!(left.contains(".min_h_0()"));
        assert!(left.contains(".overflow_y_scrollbar()"));
        assert!(!left.contains(".overflow_y_scroll()"));
        // 264px 的左栏塞不下「标题 + 统计 + 新建/刷新」单行排布，
        // 按钮行必须换行，否则会被右侧详情面板盖掉半截。
        assert!(left.contains(".flex_wrap()"));

        let right = source
            .split("let right = v_flex()")
            .nth(1)
            .expect("右栏存在")
            .split("h_flex().size_full().child(left).child(right)")
            .next()
            .expect("右栏有结尾");
        assert!(right.contains(".id(\"team-tab-detail\")"));
        assert!(right.contains(".min_h_0()"));
        assert!(right.contains(".overflow_y_scrollbar()"));
        assert!(!right.contains(".overflow_y_scroll()"));
    }

    /// accent 是表面色，当文字色在浅色主题下几乎看不见；owner 徽标必须用 primary。
    #[test]
    fn team_role_badge_uses_a_readable_owner_color() {
        let source = implementation_source();
        let badge = source
            .split("fn team_role_badge(")
            .nth(1)
            .expect("角色徽标存在")
            .split("fn membership_notice(")
            .next()
            .expect("角色徽标有结尾");
        assert!(badge.contains("Some(\"owner\") => cx.theme().primary"));
        assert!(!badge.contains("Some(\"owner\") => cx.theme().accent"));
    }

    #[test]
    fn team_tab_uses_the_shared_team_feature_gate_and_native_tab_entry() {
        let navigation = include_str!("home_tab/navigation.rs");
        assert!(
            navigation.contains("NavigationApplication::Team => self.add_team_tab(window, cx)")
        );

        let home_tabs = include_str!("home/home_tabs.rs");
        let entry = home_tabs
            .split("pub(crate) fn add_team_tab(")
            .nth(1)
            .expect("团队 tab 入口存在")
            .split("pub(crate) fn add_extensions_tab(")
            .next()
            .expect("团队 tab 入口有结尾");
        assert!(entry.contains("is_feature_enabled(Feature::TeamManagement, cx)"));
        assert!(!entry.contains("open_url"));

        // 团队管理不再把 token 拼进浏览器 URL。
        let encryption = include_str!("home_tab/encryption.rs");
        assert!(!encryption.contains("cx.open_url("));
    }

    #[test]
    fn team_role_labels_follow_the_current_locale() {
        let _guard = LOCALE_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        rust_i18n::set_locale("zh-CN");
        assert_eq!("拥有者", team_role_label(Some("owner")));
        assert_eq!("管理员", team_role_label(Some("admin")));
        assert_eq!("成员", team_role_label(Some("member")));
        // 服务端新增角色时原样显示，不要吞掉未知值。
        assert_eq!("billing", team_role_label(Some("billing")));
        assert_eq!("-", team_role_label(None));
    }

    #[test]
    fn unix_millis_to_date_renders_local_dates_and_rejects_invalid_values() {
        let date = unix_millis_to_date(1_700_000_000_000);
        assert_eq!(10, date.len(), "应渲染成 YYYY-MM-DD: {date}");
        assert!(date.starts_with("2023"));
        assert_eq!("-", unix_millis_to_date(i64::MAX));
    }

    #[test]
    fn member_directory_starts_idle_and_tracks_loading_failures() {
        assert!(matches!(MemberDirectory::Idle, MemberDirectory::Idle));
        assert!(!matches!(
            MemberDirectory::Loading,
            MemberDirectory::Failed(_)
        ));
        // 角色徽标与语言无关的部分：TeamRole 只有三档。
        assert_eq!("owner", TeamRole::Owner.as_str());
    }

    #[test]
    fn team_admin_permissions_follow_rpc_semantics() {
        // 团队资料只有 owner 能改（admin 写入会被 RLS 判成 0 行假成功）。
        assert!(can_edit_team_profile(Some("owner")));
        assert!(!can_edit_team_profile(Some("admin")));
        assert!(!can_edit_team_profile(Some("member")));
        assert!(!can_edit_team_profile(None));

        // owner 可以授任意角色；admin 不能授 owner，也不能邀请 owner。
        assert_eq!(
            assignable_team_roles(Some("owner")),
            vec![TeamRole::Owner, TeamRole::Admin, TeamRole::Member]
        );
        assert_eq!(
            assignable_team_roles(Some("admin")),
            vec![TeamRole::Admin, TeamRole::Member]
        );
        assert!(assignable_team_roles(Some("member")).is_empty());
        assert!(assignable_team_roles(None).is_empty());

        // 改角色：owner 可改任何人；admin 只能把 member 改成 admin/member。
        assert!(can_change_member_role(Some("owner"), TeamRole::Owner));
        assert!(can_change_member_role(Some("owner"), TeamRole::Admin));
        assert!(can_change_member_role(Some("admin"), TeamRole::Member));
        assert!(!can_change_member_role(Some("admin"), TeamRole::Admin));
        assert!(!can_change_member_role(Some("admin"), TeamRole::Owner));
        assert!(!can_change_member_role(Some("member"), TeamRole::Member));

        // 移除：owner 永远不可移除（包括 owner 自己），成员不能移除自己，
        // admin 只能移除 member。
        assert!(!can_remove_member(Some("owner"), TeamRole::Owner, false));
        assert!(!can_remove_member(Some("owner"), TeamRole::Owner, true));
        assert!(can_remove_member(Some("owner"), TeamRole::Admin, false));
        assert!(can_remove_member(Some("owner"), TeamRole::Member, false));
        assert!(can_remove_member(Some("admin"), TeamRole::Member, false));
        assert!(!can_remove_member(Some("admin"), TeamRole::Admin, false));
        assert!(!can_remove_member(Some("admin"), TeamRole::Member, true));
        assert!(!can_remove_member(Some("member"), TeamRole::Member, false));

        // 邀请入口只对 owner/admin 开放。
        assert!(can_manage_members(Some("owner")));
        assert!(can_manage_members(Some("admin")));
        assert!(!can_manage_members(Some("member")));
    }

    #[test]
    fn team_write_actions_call_the_engine_admin_api() {
        let production = implementation_source();
        for call in [
            ".create_team(",
            "\n                            .update_team_profile(",
            ".add_team_member(",
            "\n                            .set_team_member_role(",
            ".remove_team_member(",
        ] {
            assert!(production.contains(call), "缺少引擎调用: {call}");
        }
        // 成员操作与资料编辑都必须有入口，不能只留只读视图。
        for entry in [
            "fn show_team_create_dialog(",
            "fn show_team_profile_dialog(",
            "fn show_add_member_dialog(",
            "fn show_change_member_role_dialog(",
            "fn show_remove_member_dialog(",
            "fn after_team_admin_change(",
        ] {
            assert!(production.contains(entry), "缺少入口: {entry}");
        }
        // 角色变更 / 移除都只有拿到权限判断才渲染按钮。
        assert!(production.contains("can_change_member_role(actor_role, member.role)"));
        assert!(production.contains("can_remove_member(actor_role, member.role, is_current_user)"));
    }
}
