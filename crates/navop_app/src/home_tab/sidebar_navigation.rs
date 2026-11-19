use super::*;
use crate::navigation_applications::{NavigationApplication, home_applications};
use gpui_component::Icon;
use one_ui::IconSize;

/// Application-owned ribbon row. The ribbon is icon-only, so the label is
/// always exposed through a tooltip.
fn ribbon_row(
    id: SharedString,
    icon: Icon,
    label: SharedString,
    selected: bool,
    palette: NavPalette,
    on_click: Box<dyn Fn(&mut Window, &mut App) + 'static>,
) -> AnyElement {
    let label_for_tooltip = label.clone();
    let geometry = one_ui::theme_geometry();
    let row = div()
        .id(id)
        .w_full()
        .h(geometry.layout.global_rail_item)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(geometry.radius.md)
        .cursor_pointer()
        .focusable()
        .focus_visible(move |style| style.border_1().border_color(palette.active_foreground))
        .tooltip(move |window, cx| Tooltip::new(label_for_tooltip.clone()).build(window, cx))
        .child(icon.with_size(IconSize::Default).flex_shrink_0());
    let row = if selected {
        row.bg(palette.active_background)
            .text_color(palette.active_foreground)
    } else {
        row.text_color(palette.foreground)
            .hover(move |style| style.bg(palette.hover_background))
    };
    let on_key = std::rc::Rc::new(on_click);
    let on_pointer = on_key.clone();
    row.on_click(move |_, window, cx| on_pointer(window, cx))
        .on_key_down(move |event, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | " ") {
                on_key(window, cx);
            }
        })
        .into_any_element()
}

impl HomePage {
    /// Persistent application ribbon shared by every tab.
    pub(crate) fn render_ribbon(
        &self,
        active_content_key: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let geometry = one_ui::theme_geometry();
        let show_team = is_feature_enabled(Feature::TeamManagement, cx);
        let mut menu = v_flex()
            .id("application-ribbon-menu")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .gap_1()
            .px_2()
            .child(self.render_ribbon_connection(active_content_key, cx));
        for application in home_applications(show_team) {
            if application == NavigationApplication::Extensions {
                menu = menu.child(div().my_1().border_t_1().border_color(cx.theme().border));
            }
            menu = menu.child(self.render_ribbon_application(application, active_content_key, cx));
        }

        v_flex()
            .id("application-ribbon")
            .w(geometry.layout.global_rail)
            .min_w(geometry.layout.global_rail)
            .max_w(geometry.layout.global_rail)
            .h_full()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            // Keep the native traffic-light strip clear while the rail
            // background still extends to the window edge.
            .pt(geometry.layout.tab_bar)
            .child(menu)
            .child(
                v_flex()
                    .flex_shrink_0()
                    .gap_1()
                    .p_2()
                    .border_t_1()
                    .border_color(cx.theme().sidebar_border)
                    .child(self.render_ribbon_account_entry(window, cx))
                    .child(self.render_ribbon_settings(active_content_key, cx)),
            )
            .into_any_element()
    }

    fn render_ribbon_connection(
        &self,
        active_content_key: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = active_content_key == Some("Home");
        let tab_container = cx
            .try_global::<one_core::tab_container::GlobalTabContainer>()
            .map(|global| global.primary_pane())
            .unwrap_or_else(|| self.tab_container.clone());
        let palette = NavPalette::new(cx);
        ribbon_row(
            "ribbon-home".into(),
            Icon::default().path(NAVOP_HOME_LINE_ICON).mono(),
            t!("Home.connections_entry").into(),
            selected,
            palette,
            Box::new(move |window, cx| {
                tab_container.update(cx, |tabs, cx| {
                    tabs.activate_pinned_tab_by_id("home", window, cx);
                });
            }),
        )
    }

    fn render_ribbon_application(
        &self,
        application: NavigationApplication,
        active_content_key: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = active_content_key == Some(application_content_key(application));
        let view = cx.entity();
        ribbon_row(
            SharedString::from(format!("ribbon-app-{:?}", application)),
            Icon::new(application.icon()).mono(),
            application.label().into(),
            selected,
            NavPalette::new(cx),
            Box::new(move |window, cx| {
                view.update(cx, |home, cx| {
                    home.activate_navigation_application(application, window, cx);
                });
            }),
        )
    }

    fn render_ribbon_settings(
        &self,
        active_content_key: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = active_content_key == Some("Settings");
        let view = cx.entity();
        ribbon_row(
            "ribbon-settings".into(),
            Icon::new(IconName::Settings).mono(),
            t!("Settings.title").into(),
            selected,
            NavPalette::new(cx),
            Box::new(move |window, cx| {
                view.update(cx, |home, cx| home.add_settings_tab(window, cx));
            }),
        )
    }
}

#[derive(Clone, Copy)]
struct NavPalette {
    foreground: gpui::Hsla,
    hover_background: gpui::Hsla,
    active_background: gpui::Hsla,
    active_foreground: gpui::Hsla,
}

impl NavPalette {
    fn new(cx: &App) -> Self {
        Self {
            foreground: cx.theme().sidebar_foreground,
            hover_background: cx.theme().list_hover,
            active_background: cx.theme().sidebar_accent,
            active_foreground: cx.theme().sidebar_accent_foreground,
        }
    }
}

fn application_content_key(application: NavigationApplication) -> &'static str {
    match application {
        NavigationApplication::AiWorkbench => "AiWorkbench",
        NavigationApplication::Team => "Team",
        NavigationApplication::Notes => "Notes",
        NavigationApplication::JsonFormatter => "JsonFormatter",
        NavigationApplication::Toolbox => "Toolbox",
        NavigationApplication::SessionLogs => "SessionLogs",
        NavigationApplication::CredentialVault => "CredentialVault",
        NavigationApplication::KnownHosts => "KnownHosts",
        NavigationApplication::Extensions => "Extensions",
        NavigationApplication::Settings => "Settings",
    }
}
