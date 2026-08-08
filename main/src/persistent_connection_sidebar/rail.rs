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
use one_core::storage::ConnectionType;
use rust_i18n::t;

use super::{PersistentConnectionSidebar, PersistentConnectionSidebarEvent, SidebarPalette};
use crate::connection_visuals::connection_type_rail_icon;
use crate::home_tab::{HomePage, should_show_team_management_entry};
use crate::license::is_feature_enabled;

pub(super) fn render_navigation_rail(
    home_page: &Entity<HomePage>,
    sidebar: Entity<PersistentConnectionSidebar>,
    tree_expanded: bool,
    palette: SidebarPalette,
    cx: &gpui::App,
) -> AnyElement {
    let layout = cx.theme().geometry.layout;
    let rail_width = layout.global_rail;
    let rail_item_size = Size::Size(layout.global_rail_item);
    let home = home_page.read(cx);
    let home_active = home.is_home_active();
    let selected_filter = home.selected_filter;
    let has_tabs = {
        let tabs = home.tab_container.read(cx);
        tabs.has_pinned_tab() || !tabs.tabs().is_empty()
    };
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
                        .when(home_active && !has_tabs, |this| {
                            this.child(connection_tree_toggle_button(
                                sidebar.clone(),
                                tree_expanded,
                                palette,
                                rail_item_size,
                            ))
                        })
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
                    sidebar,
                    selected_filter,
                    palette,
                    rail_item_size,
                ))
                .child(
                    v_flex()
                        .w_full()
                        .items_center()
                        .gap_1()
                        .p_1()
                        .border_t_1()
                        .border_color(palette.border)
                        .when(show_ai_workbench_entry, |this| {
                            this.child(rail_button(
                                "persistent-open-ai-workbench",
                                IconName::AILine,
                                t!("Settings.General.Startup.default_page_ai_workbench")
                                    .to_string(),
                                palette,
                                home_page,
                                rail_item_size,
                                |home, window, cx| home.add_ai_workbench_tab(window, cx),
                            ))
                        })
                        .when(show_team, |this| {
                            this.child(rail_button(
                                "persistent-open-team",
                                IconName::TeamLine,
                                t!("TeamManagement.title").to_string(),
                                palette,
                                home_page,
                                rail_item_size,
                                |home, window, cx| home.open_team_management(window, cx),
                            ))
                        })
                        .child(rail_button(
                            "persistent-open-notes",
                            IconName::NotesLine,
                            t!("Home.notes").to_string(),
                            palette,
                            home_page,
                            rail_item_size,
                            |home, window, cx| home.add_notes_tab(window, cx),
                        ))
                        .child(rail_button(
                            "persistent-open-extensions",
                            IconName::ExtensionsLine,
                            t!("Home.extensions").to_string(),
                            palette,
                            home_page,
                            rail_item_size,
                            |home, window, cx| home.add_extensions_tab(window, cx),
                        ))
                        .child(rail_button(
                            "persistent-open-settings",
                            IconName::Settings,
                            t!("Common.settings").to_string(),
                            palette,
                            home_page,
                            rail_item_size,
                            |home, window, cx| home.add_settings_tab(window, cx),
                        ))
                        .child(rail_button(
                            "persistent-user",
                            IconName::User,
                            user_tooltip,
                            palette,
                            home_page,
                            rail_item_size,
                            |home, window, cx| {
                                if GlobalCurrentUser::get_user(cx).is_none() {
                                    home.current_user = None;
                                    home.show_login_dialog(window, cx);
                                }
                            },
                        )),
                ),
        )
        .into_any_element()
}

fn connection_tree_toggle_button(
    sidebar: Entity<PersistentConnectionSidebar>,
    expanded: bool,
    palette: SidebarPalette,
    rail_item_size: Size,
) -> impl IntoElement {
    IconButton::new(
        "persistent-toggle-connections",
        if expanded {
            IconName::PanelLeftClose
        } else {
            IconName::PanelLeftOpen
        },
    )
    .hit_size(rail_item_size)
    .glyph_size(IconSize::Medium)
    .text_color(palette.muted_foreground)
    .tooltip(if expanded {
        t!("Sidebar.hide_navigation").to_string()
    } else {
        t!("Sidebar.show_navigation").to_string()
    })
    .on_click(move |_, _, cx| {
        sidebar.update(cx, |sidebar, cx| {
            let expanded = !sidebar.is_expanded();
            sidebar.set_tree_expanded(expanded, cx);
            cx.emit(PersistentConnectionSidebarEvent::TreeVisibilityChanged { expanded });
        });
    })
}

fn render_filter_buttons(
    home_page: &Entity<HomePage>,
    sidebar: Entity<PersistentConnectionSidebar>,
    selected_filter: ConnectionType,
    palette: SidebarPalette,
    rail_item_size: Size,
) -> AnyElement {
    let mut filters = v_flex().flex_1().w_full().items_center().gap_1().p_1();
    for filter in ConnectionType::all() {
        let home = home_page.clone();
        let sidebar = sidebar.clone();
        let selected = selected_filter == filter;
        filters = filters.child(
            IconButton::new(
                format!("persistent-filter-{}", filter.label()),
                connection_type_rail_icon(filter).text_color(if selected {
                    palette.foreground
                } else {
                    palette.muted_foreground
                }),
            )
            .hit_size(rail_item_size)
            .glyph_size(IconSize::Medium)
            .selected(selected)
            .text_color(palette.foreground)
            // Selection is expressed by the container and foreground
            // hierarchy; protocol colors are reserved for content identity.
            .when(selected, |button| button.bg(palette.selected))
            .tooltip(filter.label())
            .on_click(move |_, _, cx| {
                if !selected {
                    home.update(cx, |home, cx| home.set_selected_filter(filter, cx));
                }
                sidebar.update(cx, |sidebar, cx| {
                    let expanded =
                        next_tree_expanded_after_filter_click(selected, sidebar.tree_expanded);
                    sidebar.set_tree_expanded(expanded, cx);
                    cx.emit(PersistentConnectionSidebarEvent::TreeVisibilityChanged { expanded });
                });
            }),
        );
    }
    filters.into_any_element()
}

fn next_tree_expanded_after_filter_click(selected: bool, tree_expanded: bool) -> bool {
    if selected { !tree_expanded } else { true }
}

fn rail_button(
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

struct RailButtonVisuals {
    palette: SidebarPalette,
    selected: bool,
}

impl RailButtonVisuals {
    fn new(palette: SidebarPalette, selected: bool) -> Self {
        Self { palette, selected }
    }
}

fn selectable_rail_button(
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

#[cfg(test)]
mod tests {
    use super::next_tree_expanded_after_filter_click;

    #[test]
    fn home_rail_keeps_connection_tree_toggle_when_tab_bar_is_hidden() {
        let source = include_str!("rail.rs");
        let renderer_start = source
            .find("pub(super) fn render_navigation_rail")
            .expect("navigation rail renderer");
        let renderer_end = source[renderer_start..]
            .find("fn render_filter_buttons")
            .map(|offset| renderer_start + offset)
            .expect("filter button renderer");
        let renderer = &source[renderer_start..renderer_end];

        assert!(renderer.contains("tree_expanded: bool"));
        assert!(!renderer.contains("sidebar.read(cx)"));
        assert!(renderer.contains(".when(home_active && !has_tabs"));
        assert!(renderer.contains("connection_tree_toggle_button("));
        assert!(renderer.contains("\"persistent-toggle-connections\""));
        assert!(renderer.contains("IconName::PanelLeftClose"));
        assert!(renderer.contains("IconName::PanelLeftOpen"));
        assert!(
            renderer
                .contains("PersistentConnectionSidebarEvent::TreeVisibilityChanged { expanded }")
        );
    }

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
