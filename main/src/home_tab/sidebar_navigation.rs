use super::*;
use crate::navigation_quick_open::{
    NavigationApplication, NavigationAvailability, is_overflow_connection_type,
    leading_navigation_applications, visible_connection_types,
};
use gpui_component::Selectable as _;

pub(super) struct LegacyApplicationNavigationConfig {
    pub collapsed: bool,
    pub availability: NavigationAvailability,
    pub rail_item_size: Size,
}

struct LegacySidebarButton {
    id: &'static str,
    icon: IconName,
    label: String,
    show_label: bool,
    collapsed: bool,
    selected: bool,
}

impl HomePage {
    pub(super) fn render_legacy_connection_navigation(
        &self,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut navigation = v_flex()
            .flex_1()
            .w_full()
            .p_2()
            .gap_2()
            .when(collapsed, |sidebar| sidebar.items_center());
        for filter in visible_connection_types() {
            navigation = navigation.child(self.render_legacy_filter(filter, collapsed, cx));
        }
        navigation
            .child(self.render_legacy_sidebar_button(
                LegacySidebarButton {
                    id: "legacy-more-connection-types",
                    icon: IconName::Ellipsis,
                    label: t!("Home.more_connection_types").to_string(),
                    show_label: false,
                    collapsed,
                    selected: is_overflow_connection_type(self.selected_filter),
                },
                |home, window, cx| {
                    home.show_legacy_connection_navigation_quick_open(window, cx);
                },
                cx,
            ))
            .into_any_element()
    }

    pub(super) fn render_legacy_application_navigation(
        &self,
        config: LegacyApplicationNavigationConfig,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let LegacyApplicationNavigationConfig {
            collapsed,
            availability,
            rail_item_size,
        } = config;
        let mut footer = v_flex()
            .w_full()
            .when(collapsed, |footer| footer.items_center().p_2())
            .when(!collapsed, |footer| footer.p_4())
            .gap_3()
            .border_t_1()
            .border_color(cx.theme().border);
        for application in leading_navigation_applications(availability) {
            footer =
                footer.child(self.render_legacy_application_button(application, collapsed, cx));
        }
        footer = footer.child(self.render_legacy_sidebar_button(
            LegacySidebarButton {
                id: "legacy-more-applications",
                icon: IconName::Ellipsis,
                label: t!("Home.more_applications").to_string(),
                show_label: false,
                collapsed,
                selected: false,
            },
            |home, window, cx| home.show_application_navigation_quick_open(window, cx),
            cx,
        ));
        footer
            .child(self.render_legacy_user(collapsed, rail_item_size, cx))
            .into_any_element()
    }

    fn render_legacy_filter(
        &self,
        filter: ConnectionType,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected_filter == filter;
        let icon = if collapsed {
            connection_type_rail_icon(filter)
        } else {
            connection_type_navigation_icon(filter, ConnectionVisualSize::List)
        };
        div()
            .id(filter.label())
            .flex()
            .items_center()
            .gap_3()
            .w_full()
            .py_2()
            .when(collapsed, |row| {
                row.justify_center()
                    .px_0()
                    .py_0()
                    .h(cx.theme().geometry.layout.global_rail_item)
            })
            .when(!collapsed, |row| row.px_3())
            .cursor_pointer()
            .rounded_lg()
            .overflow_hidden()
            .when(selected, |row| {
                row.bg(cx.theme().list_active)
                    .border_l_3()
                    .border_color(cx.theme().list_active_border)
            })
            .when(!selected, |row| {
                row.hover(|style| style.bg(cx.theme().sidebar_accent))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_selected_filter(filter, cx);
            }))
            .child(icon)
            .when(!collapsed, |row| {
                row.child(render_legacy_filter_label(filter, selected, cx))
            })
            .into_any_element()
    }

    fn render_legacy_application_button(
        &self,
        application: NavigationApplication,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_legacy_sidebar_button(
            LegacySidebarButton {
                id: legacy_application_id(application),
                icon: legacy_application_icon(application),
                label: application.label(),
                show_label: true,
                collapsed,
                selected: false,
            },
            move |home, window, cx| {
                home.activate_navigation_application(application, window, cx);
            },
            cx,
        )
    }

    fn render_legacy_sidebar_button(
        &self,
        button: LegacySidebarButton,
        on_click: impl Fn(&mut HomePage, &mut Window, &mut Context<HomePage>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let LegacySidebarButton {
            id,
            icon,
            label,
            show_label,
            collapsed,
            selected,
        } = button;
        let listener = cx.listener(move |home, _, window, cx| on_click(home, window, cx));
        if collapsed {
            IconButton::new(id, Icon::new(icon).mono())
                .hit_size(Size::Size(cx.theme().geometry.layout.global_rail_item))
                .glyph_size(one_ui::IconSize::Small)
                .selected(selected)
                .when(selected, |button| button.bg(cx.theme().list_active))
                .tooltip(label)
                .on_click(listener)
                .into_any_element()
        } else {
            let tooltip = label.clone();
            Button::new(id)
                .icon(Icon::new(icon).mono())
                .w_full()
                .when(show_label, |button| button.label(label).justify_start())
                .when(!show_label, |button| {
                    button.justify_center().tooltip(tooltip)
                })
                .selected(selected)
                .when(selected, |button| button.bg(cx.theme().list_active))
                .on_click(listener)
                .into_any_element()
        }
    }

    fn render_legacy_user(
        &self,
        collapsed: bool,
        rail_item_size: Size,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let user = self.current_user.as_ref();
        let view = cx.entity();
        v_flex()
            .relative()
            .w_full()
            .mt_2()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .when(collapsed, |footer| {
                footer.items_center().child(
                    IconButton::new("legacy-home-user", Icon::new(IconName::User))
                        .hit_size(rail_item_size)
                        .glyph_size(one_ui::IconSize::Small)
                        .tooltip(
                            user.map(UserInfo::resolved_display_name)
                                .unwrap_or_else(|| t!("Auth.login").to_string()),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.current_user.is_none() {
                                this.show_login_dialog(window, cx);
                            }
                        })),
                )
            })
            .when(!collapsed, |footer| {
                footer.child(render_user_avatar(
                    user,
                    view,
                    |this: &mut HomePage, window, cx| {
                        if this.current_user.is_none() {
                            this.show_login_dialog(window, cx);
                        }
                    },
                    cx,
                ))
            })
            .into_any_element()
    }
}

fn render_legacy_filter_label(
    filter: ConnectionType,
    selected: bool,
    cx: &App,
) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().foreground)
        .when(selected, |label| label.font_weight(FontWeight::MEDIUM))
        .child(filter.label())
}

fn legacy_application_id(application: NavigationApplication) -> &'static str {
    match application {
        NavigationApplication::AiWorkbench => "legacy-open-ai-workbench",
        NavigationApplication::Team => "legacy-open-team",
        NavigationApplication::Notes => "legacy-open-notes",
        NavigationApplication::JsonFormatter => "legacy-open-json-formatter",
        NavigationApplication::SessionLogs => "legacy-open-session-logs",
        NavigationApplication::CredentialVault => "legacy-open-credential-vault",
        NavigationApplication::Extensions => "legacy-open-extensions",
    }
}

fn legacy_application_icon(application: NavigationApplication) -> IconName {
    match application {
        NavigationApplication::AiWorkbench => IconName::AILine,
        NavigationApplication::Team => IconName::TeamLine,
        NavigationApplication::Notes => IconName::NotesLine,
        NavigationApplication::JsonFormatter => IconName::Json,
        NavigationApplication::SessionLogs => IconName::Terminal,
        NavigationApplication::CredentialVault => IconName::Key,
        NavigationApplication::Extensions => IconName::ExtensionsLine,
    }
}
