use super::*;

impl HomePage {
    pub(super) fn render_sidebar(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 同步全局用户状态：如果设置页面执行了登出，同步清空本地状态
        let global_user = GlobalCurrentUser::get_user(cx);
        if global_user.is_none() && self.current_user.is_some() {
            self.current_user = None;
            self.team_options.clear();
        }

        let filter_types = ConnectionType::all();
        let collapsed = self.sidebar_collapsed;
        let show_team_management_entry =
            should_show_team_management_entry(is_feature_enabled(Feature::TeamManagement, cx));
        let sidebar_width = if collapsed {
            HOME_SIDEBAR_COLLAPSED_WIDTH
        } else {
            HOME_SIDEBAR_EXPANDED_WIDTH
        };

        v_flex()
            .relative()
            .w(sidebar_width)
            .h_full()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                // 侧边栏过滤选项
                v_flex()
                    .flex_1()
                    .w_full()
                    .p_2()
                    .gap_2()
                    .when(collapsed, |this| this.items_center())
                    .children(filter_types.into_iter().map(|filter_type| {
                        let is_selected = self.selected_filter == filter_type;
                        let filter_type_clone = filter_type;

                        div()
                            .id(filter_type.label())
                            .flex()
                            .items_center()
                            .gap_3()
                            .w_full()
                            .when(collapsed, |this| this.justify_center().px_0())
                            .when(!collapsed, |this| this.px_3())
                            .py_2()
                            .cursor_pointer()
                            .rounded_lg()
                            .overflow_hidden()
                            .when(is_selected, |this| {
                                this.bg(cx.theme().list_active)
                                    .border_l_3()
                                    .border_color(cx.theme().list_active_border)
                            })
                            .when(!is_selected, |this| {
                                this.bg(cx.theme().sidebar)
                                    .hover(|style| style.bg(cx.theme().sidebar_accent))
                            })
                            .on_click(cx.listener(move |this: &mut HomePage, _, _window, cx| {
                                this.selected_filter = filter_type_clone;
                                cx.notify();
                            }))
                            .child(Icon::new(filter_type.icon()).color().with_size(Size::Large))
                            .when(!collapsed, |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().foreground)
                                        .when(is_selected, |this| {
                                            this.font_weight(FontWeight::MEDIUM)
                                        })
                                        .child(filter_type.label()),
                                )
                            })
                    })),
            )
            .child(
                // 底部区域：主题切换、设置和用户头像
                v_flex()
                    .w_full()
                    .when(collapsed, |this| this.items_center().p_2())
                    .when(!collapsed, |this| this.p_4())
                    .gap_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .when(show_team_management_entry, |this| {
                        this.child(
                            Button::new("open_team_management")
                                .icon(IconName::TeamColor.color())
                                .tooltip(t!("TeamManagement.title").to_string())
                                .when(collapsed, |button| button.ghost().small())
                                .when(!collapsed, |button| {
                                    button
                                        .label(t!("TeamManagement.title").to_string())
                                        .w_full()
                                        .justify_start()
                                })
                                .on_click(cx.listener(|this: &mut HomePage, _, window, cx| {
                                    this.open_team_management(window, cx);
                                })),
                        )
                    })
                    .child(
                        Button::new("open_notes")
                            .icon(IconName::NotesColor.color())
                            .tooltip(t!("Home.notes").to_string())
                            .when(collapsed, |button| button.ghost().small())
                            .when(!collapsed, |button| {
                                button
                                    .label(t!("Home.notes").to_string())
                                    .w_full()
                                    .justify_start()
                            })
                            .on_click(cx.listener(|this: &mut HomePage, _, window, cx| {
                                this.add_notes_tab(window, cx);
                            })),
                    )
                    .child(
                        Button::new("open_extensions")
                            .icon(IconName::ExtensionsColor.color())
                            .tooltip(t!("Home.extensions").to_string())
                            .when(collapsed, |button| button.ghost().small())
                            .when(!collapsed, |button| {
                                button
                                    .label(t!("Home.extensions").to_string())
                                    .w_full()
                                    .justify_start()
                            })
                            .on_click(cx.listener(|this: &mut HomePage, _, window, cx| {
                                this.add_extensions_tab(window, cx);
                            })),
                    )
                    .child(
                        Button::new("open_settings")
                            .icon(IconName::SettingColor.color())
                            .tooltip(t!("Common.settings").to_string())
                            .when(collapsed, |button| button.ghost().small())
                            .when(!collapsed, |button| {
                                button.label(t!("Common.settings")).w_full().justify_start()
                            })
                            .on_click(cx.listener(|this: &mut HomePage, _, window, cx| {
                                this.add_settings_tab(window, cx);
                            })),
                    )
                    // 用户头像区域
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
                            .when(collapsed, |this| {
                                this.items_center().child(
                                    Button::new("home-sidebar-user-collapsed")
                                        .icon(IconName::User)
                                        .ghost()
                                        .small()
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
                            .when(!collapsed, |this| {
                                this.child(render_user_avatar(
                                    user,
                                    view.clone(),
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
                            .id("home-sidebar-toggle")
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
                            .hover(|this| this.bg(cx.theme().muted))
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
