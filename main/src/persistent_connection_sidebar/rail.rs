#[cfg(target_os = "macos")]
use gpui::div;
use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, Entity, Hsla, IntoElement, ParentElement, Styled, px, rgb};
use gpui_component::{
    Icon, IconName, Selectable, Sizable, Size,
    button::{Button, ButtonVariants as _},
    v_flex,
};
use one_core::license::Feature;
use one_core::settings::{AppSettings, GlobalCurrentUser, StartupDefaultPage};
use one_core::storage::ConnectionType;
use rust_i18n::t;

use super::{PersistentConnectionSidebar, PersistentConnectionSidebarEvent, SidebarPalette};
use crate::home_tab::{HomePage, should_show_team_management_entry};
use crate::license::is_feature_enabled;

#[cfg(target_os = "macos")]
const NAVIGATION_RAIL_WIDTH: gpui::Pixels = px(44.0);
#[cfg(not(target_os = "macos"))]
const NAVIGATION_RAIL_WIDTH: gpui::Pixels = px(48.0);
#[cfg(target_os = "macos")]
const MACOS_TITLE_BAR_HEIGHT: gpui::Pixels = px(40.0);

pub(super) fn render_navigation_rail(
    home_page: &Entity<HomePage>,
    sidebar: Entity<PersistentConnectionSidebar>,
    palette: SidebarPalette,
    cx: &gpui::App,
) -> AnyElement {
    let home = home_page.read(cx);
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
        .w(NAVIGATION_RAIL_WIDTH)
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
                        .h(MACOS_TITLE_BAR_HEIGHT)
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
                .child(render_filter_buttons(
                    home_page,
                    sidebar,
                    selected_filter,
                    palette,
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
                                |home, window, cx| home.open_team_management(window, cx),
                            ))
                        })
                        .child(rail_button(
                            "persistent-open-notes",
                            IconName::NotesLine,
                            t!("Home.notes").to_string(),
                            palette,
                            home_page,
                            |home, window, cx| home.add_notes_tab(window, cx),
                        ))
                        .child(rail_button(
                            "persistent-open-extensions",
                            IconName::ExtensionsLine,
                            t!("Home.extensions").to_string(),
                            palette,
                            home_page,
                            |home, window, cx| home.add_extensions_tab(window, cx),
                        ))
                        .child(rail_button(
                            "persistent-open-settings",
                            IconName::Settings,
                            t!("Common.settings").to_string(),
                            palette,
                            home_page,
                            |home, window, cx| home.add_settings_tab(window, cx),
                        ))
                        .child(rail_button(
                            "persistent-user",
                            IconName::User,
                            user_tooltip,
                            palette,
                            home_page,
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

fn render_filter_buttons(
    home_page: &Entity<HomePage>,
    sidebar: Entity<PersistentConnectionSidebar>,
    selected_filter: ConnectionType,
    palette: SidebarPalette,
) -> AnyElement {
    let mut filters = v_flex().flex_1().w_full().items_center().gap_1().p_1();
    for filter in ConnectionType::all() {
        let home = home_page.clone();
        let sidebar = sidebar.clone();
        let selected = selected_filter == filter;
        filters = filters.child(
            Button::new(format!("persistent-filter-{}", filter.label()))
                .icon(
                    Icon::new(filter_line_icon(filter))
                        .text_color(filter_icon_color(filter, selected))
                        .with_size(Size::Large),
                )
                .ghost()
                .large()
                .selected(selected)
                .text_color(palette.foreground)
                // Keep the protocol identity color visible. A full primary
                // fill made the blue database icon disappear on Windows and
                // looked disproportionately bright in dark themes.
                .when(selected, |button| button.bg(palette.muted))
                .tooltip(filter.label())
                .on_click(move |_, _, cx| {
                    if !selected {
                        home.update(cx, |home, cx| home.set_selected_filter(filter, cx));
                    }
                    sidebar.update(cx, |sidebar, cx| {
                        let expanded =
                            next_tree_expanded_after_filter_click(selected, sidebar.tree_expanded);
                        sidebar.set_tree_expanded(expanded, cx);
                        cx.emit(PersistentConnectionSidebarEvent::TreeVisibilityChanged {
                            expanded,
                        });
                    });
                }),
        );
    }
    filters.into_any_element()
}

fn next_tree_expanded_after_filter_click(selected: bool, tree_expanded: bool) -> bool {
    if selected { !tree_expanded } else { true }
}

/// Unified line-style icon for each connection filter, replacing the previous
/// mix of brand SVGs and filled backplate icons.
fn filter_line_icon(filter: ConnectionType) -> IconName {
    match filter {
        ConnectionType::All => IconName::ServerLine,
        ConnectionType::Database => IconName::DatabaseLine,
        ConnectionType::SshSftp => IconName::TerminalLine,
        ConnectionType::Redis => IconName::RedisLine,
        ConnectionType::MongoDB => IconName::MongoDBLine,
        ConnectionType::Serial => IconName::SerialLine,
        ConnectionType::PortForwarding => IconName::PortForwardingLine,
        ConnectionType::Rdp => IconName::RdpLine,
        ConnectionType::Vnc => IconName::VncLine,
    }
}

/// Harmonized identity colors for the filter icons (consistent saturation and
/// lightness). Unselected icons are dimmed; the selected one keeps full color.
fn filter_icon_color(filter: ConnectionType, selected: bool) -> Hsla {
    let base = match filter {
        ConnectionType::All => 0x64748B,
        ConnectionType::SshSftp => 0xF97316,
        ConnectionType::Database => 0x3B82F6,
        ConnectionType::Redis => 0xEF4444,
        ConnectionType::MongoDB => 0x22C55E,
        ConnectionType::Serial => 0x8B5CF6,
        ConnectionType::PortForwarding => 0x0EA5E9,
        ConnectionType::Rdp => 0x6366F1,
        ConnectionType::Vnc => 0x10B981,
    };
    let color: Hsla = rgb(base).into();
    if selected { color } else { color.opacity(0.72) }
}

fn rail_button(
    id: &'static str,
    icon: IconName,
    tooltip: String,
    palette: SidebarPalette,
    home_page: &Entity<HomePage>,
    on_click: impl Fn(&mut HomePage, &mut gpui::Window, &mut gpui::Context<HomePage>) + 'static,
) -> impl IntoElement {
    let home = home_page.clone();
    Button::new(id)
        .icon(Icon::new(icon).text_color(palette.muted_foreground))
        .ghost()
        .large()
        .tooltip(tooltip)
        .on_click(move |_, window, cx| {
            home.update(cx, |home, cx| on_click(home, window, cx));
        })
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
