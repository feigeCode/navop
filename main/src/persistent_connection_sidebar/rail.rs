use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, Entity, IntoElement, ParentElement, Styled, px};
use gpui_component::{
    ActiveTheme, Icon, Selectable, Sizable, Size,
    button::{Button, ButtonVariants as _},
    v_flex,
};
use one_core::license::Feature;
use one_core::settings::GlobalCurrentUser;
use one_core::storage::ConnectionType;
use rust_i18n::t;

use crate::home_tab::{HomePage, should_show_team_management_entry};
use crate::license::is_feature_enabled;

const NAVIGATION_RAIL_WIDTH: gpui::Pixels = px(68.0);
#[cfg(target_os = "macos")]
const MACOS_NAVIGATION_RAIL_WIDTH: gpui::Pixels = px(80.0);
#[cfg(target_os = "macos")]
const MACOS_TITLE_BAR_HEIGHT: gpui::Pixels = px(40.0);

pub(super) fn render_navigation_rail(home_page: &Entity<HomePage>, cx: &gpui::App) -> AnyElement {
    let home = home_page.read(cx);
    let selected_filter = home.selected_filter;
    let show_team =
        should_show_team_management_entry(is_feature_enabled(Feature::TeamManagement, cx));
    let global_user = GlobalCurrentUser::get_user(cx);
    let user_tooltip = global_user
        .as_ref()
        .map(|user| user.resolved_display_name())
        .unwrap_or_else(|| t!("Auth.login").to_string());

    let rail_width = if cfg!(target_os = "macos") {
        #[cfg(target_os = "macos")]
        {
            MACOS_NAVIGATION_RAIL_WIDTH
        }
        #[cfg(not(target_os = "macos"))]
        {
            NAVIGATION_RAIL_WIDTH
        }
    } else {
        NAVIGATION_RAIL_WIDTH
    };

    v_flex()
        .w(rail_width)
        .h_full()
        .flex_shrink_0()
        .items_center()
        .bg(cx.theme().sidebar)
        .border_r_1()
        .border_color(cx.theme().border)
        .when(cfg!(target_os = "macos"), |this| {
            #[cfg(target_os = "macos")]
            {
                this.pt(MACOS_TITLE_BAR_HEIGHT)
            }
            #[cfg(not(target_os = "macos"))]
            {
                this
            }
        })
        .child(render_filter_buttons(home_page, selected_filter))
        .child(
            v_flex()
                .w_full()
                .items_center()
                .gap_2()
                .p_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .when(show_team, |this| {
                    this.child(rail_button(
                        "persistent-open-team",
                        gpui_component::IconName::TeamColor,
                        t!("TeamManagement.title").to_string(),
                        home_page,
                        |home, window, cx| home.open_team_management(window, cx),
                    ))
                })
                .child(rail_button(
                    "persistent-open-notes",
                    gpui_component::IconName::NotesColor,
                    t!("Home.notes").to_string(),
                    home_page,
                    |home, window, cx| home.add_notes_tab(window, cx),
                ))
                .child(rail_button(
                    "persistent-open-extensions",
                    gpui_component::IconName::ExtensionsColor,
                    t!("Home.extensions").to_string(),
                    home_page,
                    |home, window, cx| home.add_extensions_tab(window, cx),
                ))
                .child(rail_button(
                    "persistent-open-settings",
                    gpui_component::IconName::SettingColor,
                    t!("Common.settings").to_string(),
                    home_page,
                    |home, window, cx| home.add_settings_tab(window, cx),
                ))
                .child(rail_button(
                    "persistent-user",
                    gpui_component::IconName::User,
                    user_tooltip,
                    home_page,
                    |home, window, cx| {
                        if GlobalCurrentUser::get_user(cx).is_none() {
                            home.current_user = None;
                            home.show_login_dialog(window, cx);
                        }
                    },
                )),
        )
        .into_any_element()
}

fn render_filter_buttons(
    home_page: &Entity<HomePage>,
    selected_filter: ConnectionType,
) -> AnyElement {
    let mut filters = v_flex().flex_1().w_full().items_center().gap_2().p_2();
    for filter in ConnectionType::all() {
        let home = home_page.clone();
        let selected = selected_filter == filter;
        filters = filters.child(
            Button::new(format!("persistent-filter-{}", filter.label()))
                .icon(Icon::new(filter.icon()).color().with_size(Size::Large))
                .ghost()
                .small()
                .selected(selected)
                .tooltip(filter.label())
                .on_click(move |_, _, cx| {
                    home.update(cx, |home, cx| home.set_selected_filter(filter, cx));
                }),
        );
    }
    filters.into_any_element()
}

fn rail_button(
    id: &'static str,
    icon: gpui_component::IconName,
    tooltip: String,
    home_page: &Entity<HomePage>,
    on_click: impl Fn(&mut HomePage, &mut gpui::Window, &mut gpui::Context<HomePage>) + 'static,
) -> impl IntoElement {
    let home = home_page.clone();
    Button::new(id)
        .icon(icon)
        .ghost()
        .small()
        .tooltip(tooltip)
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| on_click(home, window, cx));
        })
}
