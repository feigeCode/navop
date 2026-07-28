use super::*;
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

        v_flex()
            .relative()
            .w(sidebar_width)
            .h_full()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .flex_1()
                    .w_full()
                    .p_2()
                    .gap_2()
                    .when(collapsed, |sidebar| sidebar.items_center())
                    .children(ConnectionType::all().into_iter().map(|filter| {
                        let selected = self.selected_filter == filter;
                        div()
                            .id(filter.label())
                            .flex()
                            .items_center()
                            .gap_3()
                            .w_full()
                            .when(collapsed, |row| row.justify_center().px_0())
                            .when(!collapsed, |row| row.px_3())
                            .py_2()
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
                            .child(Icon::new(filter.icon()).color().with_size(Size::Large))
                            .when(!collapsed, |row| {
                                row.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().foreground)
                                        .when(selected, |label| {
                                            label.font_weight(FontWeight::MEDIUM)
                                        })
                                        .child(filter.label()),
                                )
                            })
                    })),
            )
            .child(
                v_flex()
                    .w_full()
                    .when(collapsed, |footer| footer.items_center().p_2())
                    .when(!collapsed, |footer| footer.p_4())
                    .gap_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .when(show_ai_workbench, |footer| {
                        footer.child(self.render_legacy_sidebar_button(
                            "legacy-open-ai-workbench",
                            IconName::AI,
                            t!("Settings.General.Startup.default_page_ai_workbench").to_string(),
                            collapsed,
                            |home, window, cx| home.add_ai_workbench_tab(window, cx),
                            cx,
                        ))
                    })
                    .when(show_team, |footer| {
                        footer.child(self.render_legacy_sidebar_button(
                            "legacy-open-team",
                            IconName::TeamColor,
                            t!("TeamManagement.title").to_string(),
                            collapsed,
                            |home, window, cx| home.open_team_management(window, cx),
                            cx,
                        ))
                    })
                    .child(self.render_legacy_sidebar_button(
                        "legacy-open-notes",
                        IconName::NotesColor,
                        t!("Home.notes").to_string(),
                        collapsed,
                        |home, window, cx| home.add_notes_tab(window, cx),
                        cx,
                    ))
                    .child(self.render_legacy_sidebar_button(
                        "legacy-open-extensions",
                        IconName::ExtensionsColor,
                        t!("Home.extensions").to_string(),
                        collapsed,
                        |home, window, cx| home.add_extensions_tab(window, cx),
                        cx,
                    ))
                    .child(self.render_legacy_sidebar_button(
                        "legacy-open-settings",
                        IconName::SettingColor,
                        t!("Common.settings").to_string(),
                        collapsed,
                        |home, window, cx| home.add_settings_tab(window, cx),
                        cx,
                    ))
                    .child({
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
                                    Button::new("legacy-home-user")
                                        .icon(Icon::new(IconName::UserColor).color())
                                        .ghost()
                                        .large()
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
                    }),
            )
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

    fn render_legacy_sidebar_button(
        &self,
        id: &'static str,
        icon: IconName,
        label: String,
        collapsed: bool,
        on_click: impl Fn(&mut HomePage, &mut Window, &mut Context<HomePage>) + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new(id)
            .icon(Icon::new(icon).color())
            .tooltip(label.clone())
            .when(collapsed, |button| button.ghost().large())
            .when(!collapsed, |button| {
                button.label(label).w_full().justify_start()
            })
            .on_click(cx.listener(move |home, _, window, cx| on_click(home, window, cx)))
    }
}
