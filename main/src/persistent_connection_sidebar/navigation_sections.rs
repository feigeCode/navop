use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, Entity, IntoElement, ParentElement as _, Styled as _};
use gpui_component::{
    Icon, IconName, IconSize, Selectable as _, Sizable as _, Size,
    button::{IconButton, IconButtonRole},
    v_flex,
};
use one_core::storage::ConnectionType;
use rust_i18n::t;

use super::rail::rail_button;
use super::{PersistentConnectionSidebar, PersistentConnectionSidebarEvent, SidebarPalette};
use crate::connection_visuals::connection_type_rail_icon;
use crate::home_tab::HomePage;
use crate::navigation_quick_open::{
    NavigationApplication, NavigationAvailability, NavigationQuickOpenRequest, NavigationTarget,
    is_overflow_connection_type, leading_navigation_applications, show_navigation_quick_open,
    trailing_navigation_applications, visible_connection_types,
};

#[derive(Clone, Copy)]
pub(super) struct FilterSectionVisuals {
    pub selected_filter: ConnectionType,
    pub palette: SidebarPalette,
    pub item_size: Size,
}

pub(super) struct ApplicationSectionConfig {
    pub availability: NavigationAvailability,
    pub user_tooltip: String,
    pub palette: SidebarPalette,
    pub item_size: Size,
}

#[derive(Clone, Copy)]
struct ApplicationButtonVisuals {
    palette: SidebarPalette,
    item_size: Size,
}

#[derive(Clone)]
struct PersistentFilterNavigation {
    home: Entity<HomePage>,
    sidebar: Entity<PersistentConnectionSidebar>,
}

impl PersistentFilterNavigation {
    fn activate(&self, filter: ConnectionType, cx: &mut gpui::App) {
        let selected = self.home.read(cx).selected_filter == filter;
        if !selected {
            self.home
                .update(cx, |home, cx| home.set_selected_filter(filter, cx));
        }
        self.sidebar.update(cx, |sidebar, cx| {
            let expanded = next_tree_expanded_after_filter_click(selected, sidebar.tree_expanded);
            sidebar.set_tree_expanded(expanded, cx);
            cx.emit(PersistentConnectionSidebarEvent::TreeVisibilityChanged { expanded });
        });
    }
}

pub(super) fn render_filter_buttons(
    home_page: &Entity<HomePage>,
    sidebar: Entity<PersistentConnectionSidebar>,
    visuals: FilterSectionVisuals,
) -> AnyElement {
    let navigation = PersistentFilterNavigation {
        home: home_page.clone(),
        sidebar,
    };
    let mut filters = v_flex().flex_1().w_full().items_center().gap_1().p_1();
    for filter in visible_connection_types() {
        filters = filters.child(render_filter_button(filter, &navigation, visuals));
    }
    filters
        .child(render_filter_overflow_button(&navigation, visuals))
        .into_any_element()
}

fn render_filter_button(
    filter: ConnectionType,
    navigation: &PersistentFilterNavigation,
    visuals: FilterSectionVisuals,
) -> impl IntoElement {
    let selected = visuals.selected_filter == filter;
    let navigation = navigation.clone();
    IconButton::new(
        format!("persistent-filter-{}", filter.label()),
        connection_type_rail_icon(filter).text_color(if selected {
            visuals.palette.foreground
        } else {
            visuals.palette.muted_foreground
        }),
    )
    .role(IconButtonRole::Navigation)
    .hit_size(visuals.item_size)
    .glyph_size(IconSize::Medium)
    .selected(selected)
    .text_color(visuals.palette.foreground)
    .when(selected, |button| button.bg(visuals.palette.selected))
    .tooltip(filter.label())
    .on_click(move |_, _, cx| navigation.activate(filter, cx))
}

fn render_filter_overflow_button(
    navigation: &PersistentFilterNavigation,
    visuals: FilterSectionVisuals,
) -> impl IntoElement {
    let selected = is_overflow_connection_type(visuals.selected_filter);
    let navigation = navigation.clone();
    IconButton::new(
        "persistent-more-connection-types",
        Icon::new(IconName::Ellipsis)
            .text_color(if selected {
                visuals.palette.foreground
            } else {
                visuals.palette.muted_foreground
            })
            .with_size(IconSize::Medium),
    )
    .role(IconButtonRole::Navigation)
    .hit_size(visuals.item_size)
    .glyph_size(IconSize::Medium)
    .selected(selected)
    .text_color(visuals.palette.foreground)
    .when(selected, |button| button.bg(visuals.palette.selected))
    .tooltip(t!("Home.more_connection_types").to_string())
    .on_click(move |_, window, cx| {
        let selected_filter = navigation.home.read(cx).selected_filter;
        let navigation_for_activate = navigation.clone();
        let on_activate = Rc::new(
            move |target, _window: &mut gpui::Window, cx: &mut gpui::App| {
                if let NavigationTarget::Connection(filter) = target {
                    navigation_for_activate.activate(filter, cx);
                }
            },
        );
        let request = NavigationQuickOpenRequest::connections(selected_filter, on_activate);
        show_navigation_quick_open(request, window, cx);
    })
}

pub(super) fn render_application_buttons(
    home_page: &Entity<HomePage>,
    config: ApplicationSectionConfig,
) -> AnyElement {
    let ApplicationSectionConfig {
        availability,
        user_tooltip,
        palette,
        item_size,
    } = config;
    let visuals = ApplicationButtonVisuals { palette, item_size };
    let mut applications = v_flex()
        .w_full()
        .items_center()
        .gap_1()
        .p_1()
        .border_t_1()
        .border_color(palette.border);
    for application in leading_navigation_applications(availability) {
        applications =
            applications.child(render_application_button(application, home_page, visuals));
    }
    applications = applications.child(render_application_overflow_button(home_page, visuals));
    for application in trailing_navigation_applications() {
        applications =
            applications.child(render_application_button(application, home_page, visuals));
    }
    applications
        .child(render_user_button(home_page, user_tooltip, visuals))
        .into_any_element()
}

fn render_application_button(
    application: NavigationApplication,
    home_page: &Entity<HomePage>,
    visuals: ApplicationButtonVisuals,
) -> impl IntoElement {
    rail_button(
        persistent_application_id(application),
        persistent_application_icon(application),
        application.label(),
        visuals.palette,
        home_page,
        visuals.item_size,
        move |home, window, cx| {
            home.activate_navigation_application(application, window, cx);
        },
    )
}

fn render_application_overflow_button(
    home_page: &Entity<HomePage>,
    visuals: ApplicationButtonVisuals,
) -> impl IntoElement {
    rail_button(
        "persistent-more-applications",
        IconName::Ellipsis,
        t!("Home.more_applications").to_string(),
        visuals.palette,
        home_page,
        visuals.item_size,
        |home, window, cx| home.show_application_navigation_quick_open(window, cx),
    )
}

fn render_user_button(
    home_page: &Entity<HomePage>,
    tooltip: String,
    visuals: ApplicationButtonVisuals,
) -> impl IntoElement {
    rail_button(
        "persistent-user",
        IconName::User,
        tooltip,
        visuals.palette,
        home_page,
        visuals.item_size,
        |home, window, cx| {
            if one_core::settings::GlobalCurrentUser::get_user(cx).is_none() {
                home.current_user = None;
                home.show_login_dialog(window, cx);
            }
        },
    )
}

fn persistent_application_id(application: NavigationApplication) -> &'static str {
    match application {
        NavigationApplication::AiWorkbench => "persistent-open-ai-workbench",
        NavigationApplication::Team => "persistent-open-team",
        NavigationApplication::Notes => "persistent-open-notes",
        NavigationApplication::ApiTesting => "persistent-open-api-testing",
        NavigationApplication::JsonFormatter => "persistent-open-json-formatter",
        NavigationApplication::SessionLogs => "persistent-open-session-logs",
        NavigationApplication::CredentialVault => "persistent-open-credential-vault",
        NavigationApplication::Extensions => "persistent-open-extensions",
        NavigationApplication::Settings => "persistent-open-settings",
    }
}

fn persistent_application_icon(application: NavigationApplication) -> IconName {
    match application {
        NavigationApplication::AiWorkbench => IconName::AILine,
        NavigationApplication::Team => IconName::TeamLine,
        NavigationApplication::Notes => IconName::NotesLine,
        NavigationApplication::ApiTesting => IconName::Network,
        NavigationApplication::JsonFormatter => IconName::Schema,
        NavigationApplication::SessionLogs => IconName::Terminal,
        NavigationApplication::CredentialVault => IconName::Key,
        NavigationApplication::Extensions => IconName::ExtensionsLine,
        NavigationApplication::Settings => IconName::Settings,
    }
}

fn next_tree_expanded_after_filter_click(selected: bool, tree_expanded: bool) -> bool {
    if selected { !tree_expanded } else { true }
}

#[cfg(test)]
mod tests {
    use super::next_tree_expanded_after_filter_click;

    #[test]
    fn selected_filter_button_toggles_the_connection_tree() {
        assert!(!next_tree_expanded_after_filter_click(true, true));
        assert!(next_tree_expanded_after_filter_click(true, false));
    }

    #[test]
    fn switching_filters_always_reveals_the_connection_tree() {
        assert!(next_tree_expanded_after_filter_click(false, true));
        assert!(next_tree_expanded_after_filter_click(false, false));
    }
}
