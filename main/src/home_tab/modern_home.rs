use gpui::{
    Anchor, AnyElement, ColorExt as _, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, IconSize, Sizable, StyledExt,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use one_core::storage::StoredConnection;
use one_ui::{IconButton, IconButtonRole};
use rust_i18n::t;

use super::{
    HomePage, HomeSyncButtonContext, HomeSyncButtonState, card_connection_info,
    home_sync_button_state,
    modern_home_shortcuts::{new_connection_tooltip, quick_open_tooltip},
    should_show_team_management_entry, sync_route,
};
use crate::connection_visuals::ConnectionVisualSize;
use crate::home::connection_import_window::show_connection_import_window;
use crate::license::is_feature_enabled;
use crate::navigation_quick_open::{
    NavigationApplication, NavigationAvailability, all_navigation_applications,
};
use crate::onetcli_app::GlobalOnetCliApp;
use one_core::license::Feature;
use one_core::settings::{AppSettings, StartupDefaultPage};

const START_CENTER_MAX_WIDTH: gpui::Pixels = px(1200.0);
const START_CENTER_MAIN_COLUMN_WIDTH: gpui::Pixels = px(580.0);
const START_CENTER_SIDE_COLUMN_WIDTH: gpui::Pixels = px(300.0);
const START_CENTER_BRAND_WIDTH: gpui::Pixels = px(300.0);

struct SidePanelState<'a> {
    user: Option<&'a one_core::cloud_sync::UserInfo>,
    syncing: bool,
    sync_button_state: HomeSyncButtonState,
    view: gpui::Entity<HomePage>,
}

impl HomePage {
    pub(super) fn render_modern_home(
        &self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let view = cx.entity();
        let route = sync_route(cx);
        let personal_syncing = matches!(
            crate::personal_sync_runtime::runtime_status(cx),
            crate::personal_sync_status::PersonalSyncRuntimeStatus::Syncing
        );
        let sync_button_state = home_sync_button_state(HomeSyncButtonContext {
            route,
            sync_enabled: AppSettings::global(cx).sync_enabled,
            is_logged_in: self.current_user.is_some(),
            has_sync_license: is_feature_enabled(Feature::CloudSync, cx),
            onet_syncing: self.syncing,
            personal_sync_ready: crate::personal_sync_runtime::actions_enabled(cx),
            personal_syncing,
        });
        let syncing = self.syncing || personal_syncing;

        // 开始中心固定在窗口高度内：最近连接列表内部滚动，页面本身不出
        // 现整页滚动条（滚动容器需外层裁剪，见 AGENTS 布局经验）。
        div()
            .id("modern-home-start-center")
            .size_full()
            .overflow_hidden()
            .child(
                // 不能用 v_flex().items_center() 直接居中含 flex_1 行的内容列：
                // cross-axis center 会让该行经历一次未定义宽度 pass，其中 h_full
                // 百分比高度退化为内容自然高度，最近连接行被“撑爆”到只剩一条。
                // 居中改用主轴 justify_center + 内层每行 items_center，
                // 两列行的布局链上不再出现 cross-axis center。
                h_flex().size_full().justify_center().px_5().py_3().child(
                    v_flex()
                        .h_full()
                        .min_w_0()
                        .min_h_0()
                        .w_full()
                        .max_w(START_CENTER_MAX_WIDTH)
                        .gap_3()
                        .child(self.render_start_center_hero(view, window, cx))
                        .child(render_applications_panel(cx))
                        .child(
                            h_flex()
                                .w_full()
                                .min_h_0()
                                .flex_1()
                                .min_w_0()
                                .items_stretch()
                                .flex_wrap()
                                .gap_3()
                                .child(
                                    div()
                                        .id("modern-home-recent-column")
                                        .self_stretch()
                                        .min_w_0()
                                        .min_h_0()
                                        .flex_basis(START_CENTER_MAIN_COLUMN_WIDTH)
                                        .flex_grow_factor(2.0)
                                        .child(self.render_recent_connections_panel(window, cx)),
                                )
                                .child(
                                    v_flex()
                                        .self_stretch()
                                        .id("modern-home-side-column")
                                        .min_w_0()
                                        .min_h_0()
                                        .flex_basis(START_CENTER_SIDE_COLUMN_WIDTH)
                                        .flex_grow_1()
                                        .child(render_side_panel(
                                            SidePanelState {
                                                user: self.current_user.as_ref(),
                                                syncing,
                                                sync_button_state,
                                                view: cx.entity(),
                                            },
                                            window,
                                            cx,
                                        )),
                                ),
                        ),
                ),
            )
            .into_any_element()
    }

    fn render_start_center_hero(
        &self,
        view: gpui::Entity<HomePage>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .id("modern-home-hero")
            .w_full()
            .min_w_0()
            .items_center()
            .justify_between()
            .flex_wrap()
            .gap_3()
            .px_4()
            .py_3()
            .rounded_xl()
            .border_1()
            .border_color(cx.theme().primary.opacity(0.14))
            .bg(cx.theme().primary.opacity(0.04))
            .child(
                div()
                    .min_w_0()
                    .flex_basis(START_CENTER_BRAND_WIDTH)
                    .flex_grow_1()
                    .child(render_brand(cx)),
            )
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("modern-home-new-connection")
                            .icon(IconName::Plus)
                            .primary()
                            .large()
                            .label(t!("Home.new_connection"))
                            .tooltip(new_connection_tooltip(cx))
                            .on_click(window.listener_for(&view, |home, _, window, cx| {
                                home.show_new_connection_dialog(window, cx);
                            })),
                    )
                    .child(self.render_local_terminal_button(window, cx))
                    .child(
                        Button::new("modern-home-quick-open")
                            .icon(IconName::Search)
                            .outline()
                            .label(t!("Home.StartCenter.quick_open"))
                            .tooltip(quick_open_tooltip(cx))
                            .on_click(window.listener_for(&view, |home, _, window, cx| {
                                home.show_connection_quick_open(window, cx);
                            })),
                    )
                    .text_color(cx.theme().foreground),
            )
    }

    /// Recently opened connections, most recent first, so the home page works
    /// as a dashboard instead of a splash screen. The panel remains visible
    /// when empty to preserve the start center's task hierarchy.
    fn render_recent_connections_panel(
        &self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let mut recent: Vec<StoredConnection> = self
            .connections
            .iter()
            .filter(|conn| conn.last_used_at.is_some())
            .cloned()
            .collect();
        recent.sort_by_key(|conn| std::cmp::Reverse(conn.last_used_at));
        // badge 显示真实最近连接总数，而不是截断后可见的行数。
        let recent_total = recent.len();
        recent.truncate(8);

        // 最近列表在面板内部滚动：外层负责 flex/裁剪，内层承载滚动。
        surface_panel("modern-home-recent-panel", cx)
            .h_full()
            .min_h_0()
            .overflow_hidden()
            .child(
                panel_header(
                    t!("Home.StartCenter.recent"),
                    Some(recent_total.to_string()),
                    cx,
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Home.StartCenter.recent_description")),
                ),
            )
            .child(if recent.is_empty() {
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(render_empty_recent(cx))
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .id("modern-home-recent-list")
                            .w_full()
                            .overflow_y_scroll()
                            .children(
                                recent.into_iter().map(|conn| {
                                    self.render_recent_connection_row(conn, window, cx)
                                }),
                            ),
                    )
                    .into_any_element()
            })
    }

    fn render_recent_connection_row(
        &self,
        conn: StoredConnection,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let icon = self.connection_icon(&conn, ConnectionVisualSize::Inline);
        let name = conn.name.clone();
        let subtitle = card_connection_info(&conn)
            .map(|info| format!("{info} · {}", conn.connection_type.label()))
            .unwrap_or_else(|| conn.connection_type.label().to_string());
        let row_open_connection = conn.clone();
        let menu_open_connection = conn.clone();
        let edit_connection = conn.clone();
        let hover_border = cx.theme().list_active_border;
        let hover_background = cx.theme().muted;
        let view = cx.entity();

        h_flex()
            .id(SharedString::from(format!(
                "recent-conn-{}",
                conn.id.unwrap_or(0)
            )))
            .w_full()
            .min_w_0()
            .min_h(px(50.0))
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .cursor_pointer()
            .hover(move |style| style.bg(hover_background).border_color(hover_border))
            .on_click(window.listener_for(&view, move |home, _, window, cx| {
                home.open_connection_from_quick(&row_open_connection, window, cx);
            }))
            .child(
                div()
                    .flex_none()
                    .size(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .bg(cx.theme().secondary)
                    .text_color(cx.theme().secondary_foreground)
                    .child(icon),
            )
            .child(
                v_flex()
                    .min_w_0()
                    .flex_grow_1()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(subtitle),
                    ),
            )
            .child(
                IconButton::new(
                    SharedString::from(format!("recent-conn-menu-{}", conn.id.unwrap_or(0))),
                    IconName::ChevronRight,
                )
                .role(IconButtonRole::Compact)
                .text_color(cx.theme().muted_foreground)
                .tooltip(t!("Home.recent_actions_tooltip").to_string())
                .dropdown_menu_with_anchor(
                    Anchor::BottomRight,
                    move |menu, _, _| {
                        let open_view = view.clone();
                        let open_conn = menu_open_connection.clone();
                        let new_tab_view = view.clone();
                        let new_tab_conn = menu_open_connection.clone();
                        let edit_view = view.clone();
                        let edit_conn = edit_connection.clone();
                        let remove_view = view.clone();
                        let remove_conn_id = conn.id;
                        menu.item(
                            PopupMenuItem::new(t!("Home.open").to_string())
                                .icon(IconName::ExternalLink)
                                .on_click(move |_, window, cx| {
                                    open_view.update(cx, |home, cx| {
                                        home.open_connection_from_quick(&open_conn, window, cx);
                                    });
                                }),
                        )
                        .item(
                            PopupMenuItem::new(t!("Home.open_in_new_tab").to_string())
                                .icon(IconName::PanelRight)
                                .on_click(move |_, window, cx| {
                                    new_tab_view.update(cx, |home, cx| {
                                        home.open_connection_from_quick_with_mode(
                                            &new_tab_conn,
                                            one_core::tab_container::TabOpenMode::Background,
                                            window,
                                            cx,
                                        );
                                    });
                                }),
                        )
                        .item(
                            PopupMenuItem::new(t!("Common.edit").to_string())
                                .icon(IconName::Edit)
                                .on_click(move |_, window, cx| {
                                    edit_view.update(cx, |home, cx| {
                                        home.edit_connection(edit_conn.clone(), window, cx);
                                    });
                                }),
                        )
                        .item(
                            PopupMenuItem::new(t!("Home.remove_recent").to_string())
                                .icon(IconName::Remove)
                                .on_click(move |_, _window, cx| {
                                    remove_view.update(cx, |home, cx| {
                                        home.remove_recent_connection(remove_conn_id, cx);
                                    });
                                }),
                        )
                    },
                ),
            )
            .into_any_element()
    }
}

fn render_brand(cx: &gpui::App) -> impl IntoElement {
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_3()
        .child(
            div()
                .flex_none()
                .size(px(40.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(cx.theme().primary.opacity(0.1))
                .text_color(cx.theme().primary)
                .child(Icon::new(IconName::ServerLine).with_size(IconSize::Large)),
        )
        .child(
            v_flex()
                .min_w_0()
                .flex_grow_1()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().primary)
                        .child(t!("Home.StartCenter.get_started")),
                )
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child("Navop"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Home.StartCenter.subtitle")),
                ),
        )
}

fn render_side_panel(
    state: SidePanelState<'_>,
    window: &mut Window,
    cx: &mut gpui::Context<HomePage>,
) -> impl IntoElement {
    let SidePanelState {
        user,
        syncing,
        sync_button_state,
        view,
    } = state;

    // 侧栏拆成两张卡：「创建与导入」是行动区，「状态 + 账户」是当前环境状态区。
    v_flex()
        .w_full()
        .min_h_0()
        .h_full()
        .gap_3()
        .child(
            surface_panel("modern-home-create-panel", cx)
                .flex_shrink_0()
                .child(panel_header(
                    t!("Home.StartCenter.create_and_import"),
                    None,
                    cx,
                ))
                .child(utility_row(
                    "modern-home-import",
                    IconName::Upload,
                    t!("Home.other_app_import").to_string(),
                    t!("Home.StartCenter.import_description").to_string(),
                    view.clone(),
                    window,
                    |_, window, cx| {
                        show_connection_import_window(cx.entity(), window, cx);
                    },
                    cx,
                )),
        )
        .child(
            surface_panel("modern-home-side-panel", cx)
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(panel_header(t!("Home.StartCenter.status"), None, cx))
                .child(render_status_panel(
                    syncing,
                    sync_button_state,
                    view.clone(),
                    window,
                    cx,
                ))
                .child(div().h(px(1.0)).w_full().bg(cx.theme().border))
                .child(render_account_panel(user, view, window, cx)),
        )
}

/// Every application entry that used to sit in the persistent navigation rail
/// now lives on the start center as an icon tile grid. Visibility gating
/// (workbench/team availability) mirrors the old rail rules.
fn render_applications_panel(cx: &mut gpui::Context<HomePage>) -> impl IntoElement {
    let availability = NavigationAvailability {
        show_ai_workbench: AppSettings::current(cx).startup_default_page
            == StartupDefaultPage::Home,
        show_team: should_show_team_management_entry(is_feature_enabled(
            Feature::TeamManagement,
            cx,
        )),
    };
    let mut tiles = Vec::new();
    for application in all_navigation_applications(availability) {
        tiles.push(application_tile(application, cx));
    }

    surface_panel("modern-home-applications-panel", cx)
        .child(
            panel_header(t!("Home.StartCenter.applications"), None, cx).child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Home.StartCenter.applications_description")),
            ),
        )
        .child(
            div()
                .w_full()
                .flex()
                .flex_wrap()
                .justify_between()
                .gap_1()
                .children(tiles),
        )
}

fn application_tile(
    application: NavigationApplication,
    cx: &mut gpui::Context<HomePage>,
) -> impl IntoElement + use<> {
    let hover_background = cx.theme().muted;

    v_flex()
        .id(home_application_id(application))
        .min_w(px(88.0))
        .max_w(px(112.0))
        .flex_basis(px(88.0))
        .flex_grow_1()
        .items_center()
        .gap_1()
        .p_2()
        .rounded_md()
        .cursor_pointer()
        .hover(move |style| style.bg(hover_background))
        .on_click(cx.listener(move |home, _, window, cx| {
            home.activate_navigation_application(application, window, cx);
            collapse_connection_sidebar_if_auto_hide(cx);
        }))
        .child(
            div()
                .size(px(36.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(cx.theme().primary.opacity(0.06))
                .text_color(cx.theme().primary)
                .child(
                    Icon::new(application.icon())
                        .mono()
                        .text_color(cx.theme().primary)
                        .with_size(IconSize::Medium),
                ),
        )
        .child(
            div()
                .w_full()
                .text_xs()
                .text_center()
                .text_color(cx.theme().foreground)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(application.label()),
        )
}

fn home_application_id(application: NavigationApplication) -> &'static str {
    match application {
        NavigationApplication::AiWorkbench => "home-app-ai-workbench",
        NavigationApplication::Team => "home-app-team",
        NavigationApplication::Notes => "home-app-notes",
        NavigationApplication::JsonFormatter => "home-app-json-formatter",
        NavigationApplication::SessionLogs => "home-app-session-logs",
        NavigationApplication::CredentialVault => "home-app-credential-vault",
        NavigationApplication::Extensions => "home-app-extensions",
    }
}

/// The floating connection tree overlays the home content, so opening any
/// application from a home tile should fold it back when auto-hide is on.
/// HomePage cannot reach the sidebar entity directly, hence the global hop.
fn collapse_connection_sidebar_if_auto_hide(cx: &mut gpui::Context<HomePage>) {
    if let Some(app) = cx
        .try_global::<GlobalOnetCliApp>()
        .map(|global| global.app.clone())
    {
        app.update(cx, |app, cx| {
            app.collapse_connection_sidebar_if_auto_hide(cx);
        });
    }
}

fn render_account_panel(
    user: Option<&one_core::cloud_sync::UserInfo>,
    view: gpui::Entity<HomePage>,
    window: &mut Window,
    cx: &mut gpui::Context<HomePage>,
) -> impl IntoElement {
    // 账户区只占自然高度，撑满会把状态行挤出首屏。
    let panel = v_flex()
        .id("modern-home-account-panel")
        .w_full()
        .flex_shrink_0()
        .gap_2()
        .p_3()
        .child(panel_header(t!("Home.StartCenter.account"), None, cx));

    let panel = if user.is_none() {
        // 未登录时补一行说明，让账户卡承担同步转化入口，而不是只有一个孤立按钮。
        panel
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Home.StartCenter.account_login_description")),
            )
            .child(crate::user_avatar::render_user_avatar(
                None,
                view,
                |home, window, cx| {
                    home.show_login_dialog(window, cx);
                },
                cx,
            ))
    } else {
        // 已登录时头像占一行，右侧留登出入口，避免账户卡只有展示没有操作。
        panel.child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(crate::user_avatar::render_user_avatar(
                            user,
                            view.clone(),
                            |_, _, _| {},
                            cx,
                        )),
                )
                .child(
                    IconButton::new("account-logout-button", IconName::Close)
                        .role(IconButtonRole::Compact)
                        .text_color(cx.theme().muted_foreground)
                        .tooltip(t!("Auth.logout").to_string())
                        .on_click(window.listener_for(&view, |home, _, _window, cx| {
                            home.sign_out(cx);
                        })),
                ),
        )
    };

    panel
}

fn render_status_panel(
    syncing: bool,
    sync_button_state: HomeSyncButtonState,
    view: gpui::Entity<HomePage>,
    window: &mut Window,
    cx: &gpui::App,
) -> impl IntoElement {
    let has_key = one_core::crypto::has_master_key();
    let sync_view = view.clone();
    let key_view = view;

    // 状态区只占内容自然高度，不吸收侧栏剩余空间，账户面板因此紧跟其上。
    v_flex()
        .id("modern-home-status-panel")
        .w_full()
        .flex_shrink_0()
        .gap_2()
        .child(
            v_flex()
                .w_full()
                .gap_1()
                .child(
                    status_row(
                        "modern-home-sync",
                        if syncing {
                            IconName::LoaderCircle
                        } else {
                            IconName::Refresh
                        },
                        if syncing {
                            t!("Home.syncing").to_string()
                        } else {
                            t!("Home.sync").to_string()
                        },
                        if syncing {
                            t!("Home.StartCenter.sync_description_syncing").to_string()
                        } else {
                            t!("Home.StartCenter.sync_description").to_string()
                        },
                        !sync_button_state.is_disabled(),
                        cx,
                    )
                    .when(!sync_button_state.is_disabled(), |this| {
                        this.on_click(window.listener_for(&sync_view, |home, _, window, cx| {
                            home.handle_sync_click(window, cx);
                        }))
                    }),
                )
                .child(
                    status_row(
                        "modern-home-keys",
                        if has_key {
                            IconName::CircleCheck
                        } else {
                            IconName::Key
                        },
                        if has_key {
                            t!("Encryption.personal_key_unlocked").to_string()
                        } else {
                            t!("Encryption.personal_key_locked").to_string()
                        },
                        if has_key {
                            t!("Home.StartCenter.key_description_unlocked").to_string()
                        } else {
                            t!("Home.StartCenter.key_description_locked").to_string()
                        },
                        true,
                        cx,
                    )
                    .on_click(window.listener_for(
                        &key_view,
                        |home, _, window, cx| {
                            home.show_encryption_key_dialog(window, cx);
                        },
                    )),
                ),
        )
}

fn surface_panel(id: &'static str, cx: &gpui::App) -> gpui::Stateful<gpui::Div> {
    v_flex()
        .id(id)
        .w_full()
        .min_w_0()
        .gap_2()
        .p_3()
        .rounded_xl()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
}

fn panel_header(title: impl IntoElement, badge: Option<String>, cx: &gpui::App) -> gpui::Div {
    v_flex().w_full().gap_1().child(
        h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .justify_between()
            .child(
                div()
                    .min_w_0()
                    .flex_grow_1()
                    .text_sm()
                    .font_semibold()
                    .text_color(cx.theme().foreground)
                    .whitespace_nowrap()
                    .child(title),
            )
            .when_some(badge, |this, badge| {
                this.child(
                    div()
                        .flex_none()
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .bg(cx.theme().secondary)
                        .text_xs()
                        .text_color(cx.theme().secondary_foreground)
                        .child(badge),
                )
            }),
    )
}

fn render_empty_recent(cx: &gpui::App) -> impl IntoElement {
    v_flex()
        .w_full()
        .min_h(px(140.0))
        .items_center()
        .justify_center()
        .gap_3()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .child(
            div()
                .size(px(40.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_lg()
                .bg(cx.theme().background)
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::LayoutDashboard).with_size(IconSize::Medium)),
        )
        .child(
            v_flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(t!("Home.StartCenter.no_recent")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Home.StartCenter.no_recent_description")),
                ),
        )
}

#[allow(clippy::too_many_arguments)]
fn utility_row(
    id: &'static str,
    icon: IconName,
    title: String,
    description: String,
    view: gpui::Entity<HomePage>,
    window: &mut Window,
    on_click: impl Fn(&mut HomePage, &mut Window, &mut gpui::Context<HomePage>) + 'static,
    cx: &gpui::App,
) -> impl IntoElement {
    let hover_background = cx.theme().muted;

    h_flex()
        .id(id)
        .w_full()
        .min_w_0()
        .min_h(px(46.0))
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_lg()
        .cursor_pointer()
        .hover(move |style| style.bg(hover_background))
        .on_click(window.listener_for(&view, move |home, _, window, cx| {
            on_click(home, window, cx);
        }))
        .child(
            div()
                .flex_none()
                .size(px(30.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .bg(cx.theme().secondary)
                .text_color(cx.theme().secondary_foreground)
                .child(Icon::new(icon).with_size(IconSize::Small)),
        )
        .child(
            v_flex()
                .min_w_0()
                .flex_grow_1()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(description),
                ),
        )
        .child(
            Icon::new(IconName::ChevronRight)
                .with_size(IconSize::Small)
                .text_color(cx.theme().muted_foreground),
        )
}

fn status_row(
    id: &'static str,
    icon: IconName,
    title: String,
    description: String,
    interactive: bool,
    cx: &gpui::App,
) -> gpui::Stateful<gpui::Div> {
    let hover_background = cx.theme().muted;

    h_flex()
        .id(id)
        .w_full()
        .min_w_0()
        .min_h(px(44.0))
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_lg()
        .when(interactive, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(hover_background))
        })
        .child(
            Icon::new(icon)
                .with_size(IconSize::Small)
                .text_color(cx.theme().muted_foreground),
        )
        .child(
            v_flex()
                .min_w_0()
                .flex_grow_1()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(description),
                ),
        )
        .when(interactive, |this| {
            this.child(
                Icon::new(IconName::ChevronRight)
                    .with_size(IconSize::Small)
                    .text_color(cx.theme().muted_foreground),
            )
        })
}
