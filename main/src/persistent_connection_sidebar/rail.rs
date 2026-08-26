#[cfg(target_os = "macos")]
use gpui::div;
use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, Entity, IntoElement, ParentElement, Styled};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, IconSize, Selectable, Sizable, Size, button::IconButton,
    v_flex,
};
use one_core::license::Feature;
use one_core::settings::{AppSettings, GlobalCurrentUser, StartupDefaultPage};
use rust_i18n::t;

use super::navigation_sections::{
    ApplicationSectionConfig, FilterSectionVisuals, render_application_buttons,
    render_filter_buttons,
};
use super::{PersistentConnectionSidebar, SidebarPalette};
use crate::home_tab::{HomePage, should_show_team_management_entry};
use crate::license::is_feature_enabled;
use crate::navigation_quick_open::NavigationAvailability;

pub(super) fn render_navigation_rail(
    home_page: &Entity<HomePage>,
    sidebar: Entity<PersistentConnectionSidebar>,
    palette: SidebarPalette,
    cx: &gpui::App,
) -> AnyElement {
    let layout = cx.theme().geometry.layout;
    let rail_width = layout.global_rail;
    let rail_item_size = Size::Size(layout.global_rail_item);
    let home = home_page.read(cx);
    let home_active = home.is_home_active();
    let selected_filter = home.selected_filter;
    let show_team =
        should_show_team_management_entry(is_feature_enabled(Feature::TeamManagement, cx));
    let show_ai_workbench_entry =
        AppSettings::current(cx).startup_default_page == StartupDefaultPage::Home;
    let global_user = GlobalCurrentUser::get_user(cx);
    let user_tooltip = global_user
        .as_ref()
        .map(|user| user.resolved_display_name())
        .unwrap_or_else(|| t!("Auth.login").to_string());

    v_flex()
        .w(rail_width)
        .h_full()
        .flex_shrink_0()
        .items_center()
        .bg(palette.rail_background)
        .text_color(palette.foreground)
        // The divider helps the narrower Windows/Linux title treatment, but
        // on macOS it cuts through the traffic-light strip and looks like a
        // stray window-frame line.
        .when(!cfg!(target_os = "macos"), |this| {
            this.border_r_1().border_color(palette.border)
        })
        .when(cfg!(target_os = "macos"), |this| {
            #[cfg(target_os = "macos")]
            {
                this.child(
                    div()
                        .w_full()
                        .h(layout.macos_rail_title_bar_height)
                        .flex_shrink_0()
                        .bg(palette.rail_background),
                )
            }
            #[cfg(not(target_os = "macos"))]
            {
                this
            }
        })
        .child(
            v_flex()
                .w_full()
                .flex_1()
                .min_h_0()
                .items_center()
                .bg(palette.rail_background)
                .child(
                    v_flex()
                        .w_full()
                        .items_center()
                        .gap_1()
                        .p_1()
                        .border_b_1()
                        .border_color(palette.border)
                        .child(selectable_rail_button(
                            "persistent-open-home",
                            IconName::Home,
                            t!("Home.title").to_string(),
                            RailButtonVisuals::new(palette, home_active),
                            home_page,
                            rail_item_size,
                            |home, window, cx| HomePage::show_home(home, window, cx),
                        )),
                )
                .child(render_filter_buttons(
                    home_page,
                    sidebar.clone(),
                    FilterSectionVisuals {
                        selected_filter,
                        palette,
                        item_size: rail_item_size,
                    },
                ))
                .child(render_application_buttons(
                    home_page,
                    ApplicationSectionConfig {
                        availability: NavigationAvailability {
                            show_ai_workbench: show_ai_workbench_entry,
                            show_team,
                        },
                        user_tooltip,
                        palette,
                        item_size: rail_item_size,
                        sidebar: sidebar.clone(),
                    },
                )),
        )
        .into_any_element()
}

pub(super) fn rail_button(
    id: &'static str,
    icon: IconName,
    tooltip: String,
    palette: SidebarPalette,
    home_page: &Entity<HomePage>,
    rail_item_size: Size,
    on_click: impl Fn(&mut HomePage, &mut gpui::Window, &mut gpui::Context<HomePage>) + 'static,
) -> impl IntoElement {
    selectable_rail_button(
        id,
        icon,
        tooltip,
        RailButtonVisuals::new(palette, false),
        home_page,
        rail_item_size,
        move |home_page, window, cx| {
            home_page.update(cx, |home, cx| on_click(home, window, cx));
        },
    )
}

pub(super) struct RailButtonVisuals {
    palette: SidebarPalette,
    selected: bool,
}

impl RailButtonVisuals {
    pub(super) fn new(palette: SidebarPalette, selected: bool) -> Self {
        Self { palette, selected }
    }
}

pub(super) fn selectable_rail_button(
    id: &'static str,
    icon: IconName,
    tooltip: String,
    visuals: RailButtonVisuals,
    home_page: &Entity<HomePage>,
    rail_item_size: Size,
    on_click: impl Fn(&Entity<HomePage>, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let RailButtonVisuals { palette, selected } = visuals;
    let home = home_page.clone();
    IconButton::new(
        id,
        Icon::new(icon)
            .mono()
            .text_color(if selected {
                palette.foreground
            } else {
                palette.muted_foreground
            })
            .with_size(IconSize::Medium),
    )
    .hit_size(rail_item_size)
    .glyph_size(IconSize::Medium)
    .selected(selected)
    .when(selected, |button| button.bg(palette.selected))
    .tooltip(tooltip)
    .on_click(move |_, window, cx| {
        on_click(&home, window, cx);
    })
}
