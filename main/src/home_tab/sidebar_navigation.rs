use one_ui::IconSize;
use super::*;
use crate::navigation_applications::{NavigationApplication, home_applications};
use gpui_component::Icon;

/// 首页功能导航图标槽位尺寸（线性单色图标统一 16px）。
const NAV_ICON_SIZE: IconSize = IconSize::Default;
/// 导航行高 32px（小号密度档）。
const NAV_ROW_HEIGHT: gpui::Pixels = px(32.0);

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
        .h(NAV_ROW_HEIGHT)
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .rounded(radius)
        .text_sm()
        .cursor_pointer()
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
        let collapsed = self.sidebar_collapsed;
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let show_team = is_feature_enabled(Feature::TeamManagement, cx);
        let mut menu = v_flex().w_full().gap_1().child(self.render_nav_row(
            "home-app-home",
            Icon::default().path(NAVOP_HOME_LINE_ICON).mono(),
            t!("Home.connections_entry").to_string(),
            true, // Home is already visible. Do not reset search, groups, layout or scroll.
            None,
            cx,
        ));
        for application in home_applications(show_team) {
            if application == NavigationApplication::Extensions {
                menu = menu.child(div().my_2().border_t_1().border_color(cx.theme().border));
            }
            let label = application.label();
            menu = menu.child(self.render_nav_row(
                match application {
                    NavigationApplication::AiWorkbench => "home-app-ai-workbench",
                    NavigationApplication::Team => "home-app-team",
                    NavigationApplication::Notes => "home-app-notes",
                    NavigationApplication::JsonFormatter => "home-app-json",
                    NavigationApplication::Toolbox => "home-app-toolbox",
                    NavigationApplication::SessionLogs => "home-app-session-logs",
                    NavigationApplication::CredentialVault => "home-app-vault",
                    NavigationApplication::KnownHosts => "home-app-known-hosts",
                    NavigationApplication::Extensions => "home-app-extensions",
                },
                Icon::new(application.icon()).mono(),
                label,
                false,
                Some(Box::new(move |home, window, cx| {
                    home.activate_navigation_application(application, window, cx);
                })),
                cx,
            ));
        }
        menu.into_any_element()
    }

    /// 底部控制行：设置 + 侧栏折叠开关同行，折叠控制不再悬浮在侧栏边缘。
    pub(super) fn render_settings_entry(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self.sidebar_collapsed;
        let settings = self.render_nav_row(
            "home-app-settings",
            Icon::new(IconName::Settings).mono(),
            t!("Settings.title").to_string(),
            false,
            Some(Box::new(|home, window, cx| {
                home.add_settings_tab(window, cx)
            })),
            cx,
        );
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
