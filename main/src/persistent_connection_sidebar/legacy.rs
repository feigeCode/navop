use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ElementId, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, FunctionalIcon, Icon, IconName, IconSize, Sizable, Size,
    button::{Button, IconButton},
    v_flex,
};
use one_core::cloud_sync::UserInfo;
use one_core::license::Feature;
use one_core::settings::{AppSettings, GlobalCurrentUser, StartupDefaultPage};
use one_core::storage::ConnectionType;
use rust_i18n::t;

use super::PersistentConnectionSidebar;
use crate::connection_visuals::{
    ConnectionVisualSize, connection_type_navigation_icon, connection_type_rail_icon,
};
use crate::home_tab::{HomePage, should_show_team_management_entry};
use crate::license::is_feature_enabled;
use crate::user_avatar::render_user_avatar;

const LEGACY_SIDEBAR_EXPANDED_WIDTH: gpui::Pixels = px(220.0);
const LEGACY_SIDEBAR_COLLAPSED_WIDTH: gpui::Pixels = px(68.0);

impl PersistentConnectionSidebar {
    pub(super) fn render_legacy_sidebar(
        &mut self,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let collapsed = self.legacy_collapsed;
        let sidebar_width = if collapsed {
            LEGACY_SIDEBAR_COLLAPSED_WIDTH
        } else {
            LEGACY_SIDEBAR_EXPANDED_WIDTH
        };
        let show_team =
            should_show_team_management_entry(is_feature_enabled(Feature::TeamManagement, cx));
        let show_ai_workbench =
            AppSettings::current(cx).startup_default_page == StartupDefaultPage::Home;
        let rail_item_size = Size::Size(cx.theme().geometry.layout.global_rail_item);
        let (home_active, selected_filter, current_user) = {
            let home = self.home_page.read(cx);
            (
                home.is_home_active(),
                home.selected_filter,
                home.current_user.clone(),
            )
        };

        let home_page = self.home_page.clone();
        let home_entry = legacy_navigation_row(
            "legacy-open-home",
            Icon::new(IconName::Home)
                .with_size(Size::Small)
                .into_any_element(),
            t!("Home.title").to_string().into(),
            home_active,
            collapsed,
            cx,
            move |window, cx| HomePage::show_home(&home_page, window, cx),
        );

        let mut navigation = v_flex()
            .flex_1()
            .w_full()
            .p_2()
            .gap_2()
            .when(collapsed, |sidebar| sidebar.items_center())
            .child(home_entry);

        for filter in ConnectionType::all() {
            let selected = selected_filter == filter;
            let icon = if collapsed {
                connection_type_rail_icon(filter).into_any_element()
            } else {
                connection_type_navigation_icon(filter, ConnectionVisualSize::List)
                    .into_any_element()
            };
            let home_page = self.home_page.clone();
            navigation = navigation.child(legacy_navigation_row(
                format!("legacy-filter-{}", filter.label()),
                icon,
                filter.label().into(),
                selected,
                collapsed,
                cx,
                move |window, cx| {
                    home_page.update(cx, |home, cx| {
                        home.set_selected_filter(filter, cx);
                    });
                    HomePage::show_home(&home_page, window, cx);
                },
            ));
        }

        v_flex()
            .relative()
            .w(sidebar_width)
            .h_full()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .when(cfg!(target_os = "macos"), |sidebar| {
                sidebar.child(
                    div()
                        .w_full()
                        .h(cx.theme().geometry.layout.macos_rail_title_bar_height)
                        .flex_shrink_0(),
                )
            })
            .child(navigation)
            .child(
                v_flex()
                    .w_full()
                    .when(collapsed, |footer| footer.items_center().p_2())
                    .when(!collapsed, |footer| footer.p_4())
                    .gap_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .when(show_ai_workbench, |footer| {
                        footer.child(legacy_sidebar_button(
                            "legacy-open-ai-workbench",
                            IconName::AILine,
                            t!("Settings.General.Startup.default_page_ai_workbench").to_string(),
                            collapsed,
                            &self.home_page,
                            cx,
                            |home, window, cx| home.add_ai_workbench_tab(window, cx),
                        ))
                    })
                    .when(show_team, |footer| {
                        footer.child(legacy_sidebar_button(
                            "legacy-open-team",
                            IconName::TeamColor,
                            t!("TeamManagement.title").to_string(),
                            collapsed,
                            &self.home_page,
                            cx,
                            |home, window, cx| home.open_team_management(window, cx),
                        ))
                    })
                    .child(legacy_sidebar_button(
                        "legacy-open-notes",
                        IconName::NotesColor,
                        t!("Home.notes").to_string(),
                        collapsed,
                        &self.home_page,
                        cx,
                        |home, window, cx| home.add_notes_tab(window, cx),
                    ))
                    .child(legacy_sidebar_button(
                        "legacy-open-extensions",
                        IconName::ExtensionsColor,
                        t!("Home.extensions").to_string(),
                        collapsed,
                        &self.home_page,
                        cx,
                        |home, window, cx| home.add_extensions_tab(window, cx),
                    ))
                    .child(legacy_sidebar_button(
                        "legacy-open-settings",
                        IconName::SettingColor,
                        t!("Common.settings").to_string(),
                        collapsed,
                        &self.home_page,
                        cx,
                        |home, window, cx| home.add_settings_tab(window, cx),
                    ))
                    .child(render_legacy_user(
                        &self.home_page,
                        current_user.as_ref(),
                        collapsed,
                        rail_item_size,
                        cx,
                    )),
            )
            .child(
                div()
                    .absolute()
                    .right_0()
                    .top_0()
                    .bottom_0()
                    .w(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .occlude()
                    .cursor_pointer()
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.toggle_legacy_collapsed(cx);
                        }),
                    )
                    .child(
                        div()
                            .id("legacy-home-sidebar-toggle")
                            .w(px(18.0))
                            .h(px(52.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(9.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .shadow_sm()
                            .hover(|handle| handle.bg(cx.theme().muted))
                            .child(
                                Icon::new(if collapsed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronLeft
                                })
                                .with_size(Size::Small)
                                .text_color(cx.theme().muted_foreground),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn legacy_navigation_row(
    id: impl Into<ElementId>,
    icon: AnyElement,
    label: SharedString,
    selected: bool,
    collapsed: bool,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_3()
        .w_full()
        .py_2()
        .when(collapsed, |row| {
            row.justify_center()
                .px_0()
                .py_0()
                .h(cx.theme().geometry.layout.global_rail_item)
        })
        .when(!collapsed, |row| row.px_3())
        .cursor_pointer()
        .rounded_lg()
        .overflow_hidden()
        .when(selected, |row| {
            row.bg(cx.theme().list_active)
                .border_l_3()
                .border_color(cx.theme().list_active_border)
        })
        .when(!selected, |row| {
            row.hover(|style| style.bg(cx.theme().sidebar_accent))
        })
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(icon)
        .when(!collapsed, |row| {
            row.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .when(selected, |label| label.font_weight(FontWeight::MEDIUM))
                    .child(label),
            )
        })
        .into_any_element()
}

fn legacy_sidebar_button(
    id: &'static str,
    icon: IconName,
    label: String,
    collapsed: bool,
    home_page: &Entity<HomePage>,
    cx: &App,
    on_click: impl Fn(&mut HomePage, &mut Window, &mut gpui::Context<HomePage>) + 'static,
) -> AnyElement {
    let home_page = home_page.clone();
    let listener = move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
        home_page.update(cx, |home, cx| on_click(home, window, cx));
    };

    if collapsed {
        IconButton::new(id, Icon::new(icon).color())
            .hit_size(Size::Size(cx.theme().geometry.layout.global_rail_item))
            .glyph_size(IconSize::Medium)
            .tooltip(label)
            .on_click(listener)
            .into_any_element()
    } else {
        Button::new(id)
            .icon(Icon::new(icon).color())
            .label(label)
            .w_full()
            .justify_start()
            .on_click(listener)
            .into_any_element()
    }
}

fn render_legacy_user(
    home_page: &Entity<HomePage>,
    user: Option<&UserInfo>,
    collapsed: bool,
    rail_item_size: Size,
    cx: &App,
) -> AnyElement {
    let home_for_click = home_page.clone();
    v_flex()
        .relative()
        .w_full()
        .mt_2()
        .pt_2()
        .border_t_1()
        .border_color(cx.theme().border)
        .when(collapsed, |footer| {
            footer.items_center().child(
                IconButton::new("legacy-home-user", FunctionalIcon::new(IconName::User))
                    .hit_size(rail_item_size)
                    .glyph_size(IconSize::Medium)
                    .tooltip(
                        user.map(UserInfo::resolved_display_name)
                            .unwrap_or_else(|| t!("Auth.login").to_string()),
                    )
                    .on_click(move |_, window, cx| {
                        home_for_click.update(cx, |home, cx| {
                            if GlobalCurrentUser::get_user(cx).is_none() {
                                home.current_user = None;
                                home.show_login_dialog(window, cx);
                            }
                        });
                    }),
            )
        })
        .when(!collapsed, |footer| {
            footer.child(render_user_avatar(
                user,
                home_page.clone(),
                |home: &mut HomePage, window, cx| {
                    if home.current_user.is_none() {
                        home.show_login_dialog(window, cx);
                    }
                },
                cx,
            ))
        })
        .into_any_element()
}
