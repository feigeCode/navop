use super::*;
use crate::navigation_applications::{NavigationApplication, home_applications};
use gpui_component::sidebar::{SidebarItem, SidebarMenuItem};

impl HomePage {
    pub(super) fn render_application_navigation(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let show_team = is_feature_enabled(Feature::TeamManagement, cx);
        let mut menu = v_flex().w_full().gap_1().child(
            SidebarMenuItem::new(t!("Home.connections_entry").to_string())
                // 导航图标统一线性单色体系，与其他入口一致（redesign §5.3）。
                .icon(Icon::new(IconName::Home).size(px(18.0)))
                .active(true)
                .collapsed(self.sidebar_collapsed)
                .render("home-app-home", window, cx), // Home is already visible. Do not reset search, groups, layout or scroll.
        );
        for application in home_applications(show_team) {
            if application == NavigationApplication::Extensions {
                menu = menu.child(div().my_2().border_t_1().border_color(cx.theme().border));
            }
            menu = menu.child(
                SidebarMenuItem::new(application.label())
                    .icon(Icon::new(application.icon()).size(px(18.0)))
                    .collapsed(self.sidebar_collapsed)
                    .on_click(cx.listener(move |home, _, window, cx| {
                        home.activate_navigation_application(application, window, cx);
                    }))
                    .render(application.label(), window, cx),
            );
        }
        menu.into_any_element()
    }

    /// 底部控制行：设置 + 侧栏折叠开关同行，折叠控制不再悬浮在侧栏边缘。
    pub(super) fn render_settings_entry(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self.sidebar_collapsed;
        let on_toggle = cx.listener(|this, _, _, cx| this.toggle_sidebar(cx));
        let settings = SidebarMenuItem::new(t!("Settings.title").to_string())
            .icon(Icon::new(IconName::Settings).size(px(18.0)))
            .collapsed(collapsed)
            .on_click(cx.listener(|home, _, window, cx| home.add_settings_tab(window, cx)))
            .render("home-app-settings", window, cx);
        let toggle = IconButton::new(
            "home-sidebar-toggle",
            if collapsed {
                IconName::ChevronRight
            } else {
                IconName::ChevronLeft
            },
        )
        .ghost()
        .tooltip(t!("Home.toggle_sidebar"))
        .on_click(on_toggle);
        if collapsed {
            // 折叠态宽度不足，开关单独成行居中。
            v_flex()
                .w_full()
                .gap_1()
                .child(settings)
                .child(h_flex().w_full().justify_center().child(toggle))
                .into_any_element()
        } else {
            h_flex()
                .w_full()
                .gap_1()
                .items_center()
                .child(div().flex_1().min_w_0().child(settings))
                .child(toggle)
                .into_any_element()
        }
    }
}
