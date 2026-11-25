use super::*;
use crate::navigation_applications::{NavigationApplication, home_applications};
use gpui_component::Icon;
use one_ui::IconSize;

/// 首页功能导航图标槽位尺寸（线性单色图标统一 16px）。
const NAV_ICON_SIZE: IconSize = IconSize::Default;
/// 导航行高 34px（标准密度档）。
const NAV_ROW_HEIGHT: gpui::Pixels = px(34.0);
/// 图标导航按钮保留 38px 点击区，并与窗口红绿灯避让区对齐。
const NAV_COLLAPSED_ROW_HEIGHT: gpui::Pixels = px(38.0);

/// 首页导航行状态色：默认透明底+侧栏前景；hover 用浅中性底（侧栏前景 6% 叠加），
/// 选中用 sidebar_accent 底 + sidebar_accent_foreground 文字且不被 hover 覆盖。
#[derive(Clone, Copy)]
struct NavRowPalette {
    foreground: gpui::Hsla,
    hover_bg: gpui::Hsla,
    active_bg: gpui::Hsla,
    active_foreground: gpui::Hsla,
}

impl NavRowPalette {
    fn new(cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            foreground: theme.sidebar_foreground,
            hover_bg: theme.sidebar_foreground.opacity(0.06),
            active_bg: theme.sidebar_accent,
            active_foreground: theme.sidebar_accent_foreground,
        }
    }
}

/// 首页导航行（替代 SidebarMenuItem：其内部 hover/active 颜色不可从外层覆盖）。
/// 支持 hover / selected / focus-visible 区分与键盘 Enter/Space 触发。
#[allow(clippy::too_many_arguments)]
fn home_nav_row(
    id: SharedString,
    icon: Icon,
    label: SharedString,
    selected: bool,
    collapsed: bool,
    palette: NavRowPalette,
    radius: gpui::Pixels,
    on_click: Box<dyn Fn(&mut Window, &mut App) + 'static>,
) -> AnyElement {
    let label_for_tooltip = label.clone();
    let mut row = div()
        .id(id)
        .w_full()
        .h(if collapsed {
            NAV_COLLAPSED_ROW_HEIGHT
        } else {
            NAV_ROW_HEIGHT
        })
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .rounded(if collapsed { px(8.0) } else { radius })
        .text_sm()
        .focusable()
        .focus_visible(|style| {
            style
                .border_1()
                .border_color(palette.active_foreground.opacity(0.6))
        })
        .child(icon.with_size(NAV_ICON_SIZE).flex_shrink_0());
    if collapsed {
        row = row
            .justify_center()
            .tooltip(move |window, cx| Tooltip::new(label_for_tooltip.clone()).build(window, cx));
    } else {
        row = row.child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(label),
        );
    }
    if selected {
        row = row
            .bg(palette.active_bg)
            .text_color(palette.active_foreground)
            .font_weight(FontWeight::MEDIUM);
    } else {
        row = row
            .text_color(palette.foreground)
            .hover(move |style| style.bg(palette.hover_bg));
    }
    let on_key = std::rc::Rc::new(on_click);
    let on_pointer = on_key.clone();
    row.on_click(move |_, window, cx| on_pointer(window, cx))
        .on_key_down(move |event, window, cx| {
            let key = event.keystroke.key.as_str();
            if key == "enter" || key == " " {
                on_key(window, cx);
            }
        })
        .into_any_element()
}

impl HomePage {
    /// 渲染一条首页导航行。`action` 为 None 时纯展示（如已激活的“连接”入口）。
    fn render_nav_row(
        &self,
        id: &'static str,
        icon: Icon,
        label: String,
        selected: bool,
        action: Option<Box<dyn Fn(&mut HomePage, &mut Window, &mut Context<Self>) + 'static>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = NavRowPalette::new(cx);
        let radius = cx.theme().radius;
        let collapsed = self.sidebar_collapsed || HomePage::uses_global_navigation_layout(cx);
        match action {
            Some(action) => {
                let view = cx.entity();
                home_nav_row(
                    id.into(),
                    icon,
                    label.into(),
                    selected,
                    collapsed,
                    palette,
                    radius,
                    Box::new(move |window, cx| {
                        view.update(cx, |home, cx| action(home, window, cx));
                    }),
                )
            }
            None => home_nav_row(
                id.into(),
                icon,
                label.into(),
                selected,
                collapsed,
                palette,
                radius,
                Box::new(|_, _| {}),
            ),
        }
    }

    pub(super) fn render_application_navigation(
        &self,
        active_key: Option<&str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let show_team = is_feature_enabled(Feature::TeamManagement, cx);
        let active_key = active_key.unwrap_or("Home");
        let home_active = active_key == "Home";
        let mut menu = v_flex().w_full().gap_1().child(self.render_nav_row(
            "home-app-home",
            Icon::default().path(NAVOP_HOME_LINE_ICON).mono(),
            t!("Home.connections_entry").to_string(),
            home_active,
            Some(Box::new(|home, window, cx| {
                let tabs = home.tab_container.clone();
                let home_tab_id = cx
                    .try_global::<crate::navop_app::GlobalHomePage>()
                    .map(|home| home.home_tab_id.clone());
                window.defer(cx, move |window, cx| {
                    tabs.update(cx, |tabs, cx| {
                        if let Some(id) = home_tab_id {
                            tabs.activate_pinned_tab_by_id(&id, window, cx);
                        } else {
                            tabs.activate_pinned_tab(window, cx);
                        }
                    });
                });
            })),
            cx,
        ));
        let applications = home_applications(show_team);
        for application in applications.iter().copied() {
            if application == NavigationApplication::Team
                || (application == NavigationApplication::Extensions && !show_team)
            {
                menu = menu.child(div().my_2().border_t_1().border_color(cx.theme().border));
            }
            let label = application.label();
            let selected = application.content_key() == active_key;
            menu = menu.child(self.render_nav_row(
                match application {
                    NavigationApplication::AiWorkbench => "home-app-ai-workbench",
                    NavigationApplication::Monitor => "home-app-monitor",
                    NavigationApplication::Team => "home-app-team",
                    NavigationApplication::Notes => "home-app-notes",
                    NavigationApplication::JsonFormatter => "home-app-json",
                    NavigationApplication::Toolbox => "home-app-toolbox",
                    NavigationApplication::SessionLogs => "home-app-session-logs",
                    NavigationApplication::CredentialVault => "home-app-vault",
                    NavigationApplication::KnownHosts => "home-app-known-hosts",
                    NavigationApplication::Extensions => "home-app-extensions",
                    NavigationApplication::Settings => "home-app-settings",
                },
                Icon::new(application.icon()).mono(),
                label,
                selected,
                Some(Box::new(move |home, window, cx| {
                    home.activate_navigation_application(application, window, cx);
                })),
                cx,
            ));
        }
        // 常用与次级入口都直接平铺；短窗口由外层菜单区域滚动，不藏进「更多」。
        menu.into_any_element()
    }

    /// 底部控制行：设置 + 侧栏折叠开关同行，折叠控制不再悬浮在侧栏边缘。
    pub(super) fn render_settings_entry(
        &self,
        active_key: Option<&str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self.sidebar_collapsed || HomePage::uses_global_navigation_layout(cx);
        let settings_active = active_key == Some("Settings");
        let settings = self.render_nav_row(
            "home-app-settings",
            Icon::new(IconName::Settings).mono(),
            t!("Settings.title").to_string(),
            settings_active,
            Some(Box::new(|home, window, cx| {
                home.add_settings_tab(window, cx)
            })),
            cx,
        );
        if HomePage::uses_global_navigation_layout(cx) {
            return settings;
        }
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
        .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)));
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

#[cfg(test)]
mod layout_tests {
    #[test]
    fn secondary_navigation_stays_visible_and_uses_existing_page_activation() {
        let source = include_str!("sidebar_navigation.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("implementation");
        assert!(source.contains("for application in applications.iter().copied()"));
        assert!(!source.contains(".dropdown_menu_with_anchor(Anchor::TopRight"));
        assert!(source.contains("home.activate_navigation_application(application, window, cx)"));
    }
}
