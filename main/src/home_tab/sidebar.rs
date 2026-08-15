use super::sidebar_navigation::LegacyApplicationNavigationConfig;
use super::*;
use crate::navigation_quick_open::NavigationAvailability;
use one_core::settings::StartupDefaultPage;

impl HomePage {
    pub(super) fn render_sidebar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let collapsed = self.sidebar_collapsed;
        let sidebar_width = if collapsed {
            HOME_SIDEBAR_COLLAPSED_WIDTH
        } else {
            HOME_SIDEBAR_EXPANDED_WIDTH
        };
        let show_team =
            should_show_team_management_entry(is_feature_enabled(Feature::TeamManagement, cx));
        let show_ai_workbench =
            AppSettings::current(cx).startup_default_page == StartupDefaultPage::Home;
        let rail_item_size = Size::Size(cx.theme().geometry.layout.global_rail_item);
        let navigation = self.render_legacy_connection_navigation(collapsed, cx);
        let availability = NavigationAvailability {
            show_ai_workbench,
            show_team,
        };

        v_flex()
            .relative()
            .w(sidebar_width)
            .h_full()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(navigation)
            .child(self.render_legacy_application_navigation(
                LegacyApplicationNavigationConfig {
                    collapsed,
                    availability,
                    rail_item_size,
                },
                cx,
            ))
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
                            this.toggle_sidebar(cx);
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
    }
}
