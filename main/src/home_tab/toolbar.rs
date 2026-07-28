use super::*;

impl HomePage {
    pub(super) fn render_toolbar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();

        let workspace_filter_open = self.workspace_filter_open;
        let workspace_filter =
            self.render_workspace_filter_popover(workspace_filter_open, window, cx);

        let is_syncing = self.syncing;
        let is_logged_in = self.current_user.is_some();
        let has_sync_license = is_feature_enabled(Feature::CloudSync, cx);
        let route = sync_route(cx);
        let personal_syncing = matches!(
            crate::personal_sync_runtime::runtime_status(cx),
            crate::personal_sync_status::PersonalSyncRuntimeStatus::Syncing
        );
        let personal_sync_ready = crate::personal_sync_runtime::actions_enabled(cx);
        let sync_disabled = match route {
            HomeSyncRoute::OnetCloud => (!is_logged_in && has_sync_license) || is_syncing,
            HomeSyncRoute::Personal => !personal_sync_ready || personal_syncing,
        };
        let has_master_key = crypto::has_master_key();
        let show_team_key_menu_item = is_feature_enabled(Feature::TeamManagement, cx)
            && should_show_team_key_menu_item(route, self.team_permissions.teams().len());
        let personal_conflict_count = if route == HomeSyncRoute::Personal {
            crate::personal_sync_conflicts::current_personal_conflict_count(cx)
        } else {
            0
        };
        let conflict_count = match route {
            HomeSyncRoute::OnetCloud => self.pending_conflicts.len(),
            HomeSyncRoute::Personal => personal_conflict_count,
        };
        let has_conflicts = conflict_count > 0;

        let legacy = self.home_page_style == HomePageStyle::Legacy;

        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .when(legacy, |toolbar| toolbar.gap_3().px_4().py_2())
            .when(!legacy, |toolbar| toolbar.gap_2().px_3().py_1())
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .items_center()
            .justify_between()
            // ===== 左侧功能区 =====
            .child(
                h_flex()
                    .min_w_0()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    // 新建连接按钮（主要操作）
                    .child(
                        Button::new("new-connect-button")
                            .icon(IconName::Plus)
                            .primary()
                            .label(t!("Home.new_connection"))
                            .tooltip(t!("Home.new_connection"))
                            .on_click(window.listener_for(&view, move |this, _, window, cx| {
                                this.show_new_connection_dialog(window, cx);
                            })),
                    )
                    .child(self.render_local_terminal_button(window, cx))
                    .child(
                        Button::new("import-connection-button")
                            .icon(IconName::Upload)
                            .label(t!("Home.other_app_import"))
                            .tooltip(t!("Home.other_app_import"))
                            .on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    show_connection_import_window(
                                        view.clone(),
                                        window.window_handle(),
                                        cx,
                                    );
                                }
                            }),
                    )
                    // 分隔线
                    .child(div().h(px(20.0)).w(px(1.0)).bg(cx.theme().border).mx_1())
                    // 同步按钮
                    .child(
                        Button::new("sync-button")
                            .icon(if has_sync_license {
                                IconName::Refresh
                            } else {
                                IconName::Key
                            })
                            .label(if is_syncing || personal_syncing {
                                t!("Home.syncing").to_string()
                            } else if route == HomeSyncRoute::OnetCloud && !has_sync_license {
                                t!("License.upgrade_to_pro").to_string()
                            } else {
                                t!("Home.sync").to_string()
                            })
                            .ghost()
                            .disabled(sync_disabled)
                            .tooltip(
                                if route == HomeSyncRoute::Personal && !personal_sync_ready {
                                    t!("Settings.Sync.Status.not_configured")
                                } else if route == HomeSyncRoute::OnetCloud
                                    && !is_logged_in
                                    && has_sync_license
                                {
                                    t!("Home.cloud_need_login")
                                } else if route == HomeSyncRoute::OnetCloud && !has_sync_license {
                                    t!("License.pro_required")
                                } else {
                                    t!("Home.sync_tooltip")
                                },
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if sync_route(cx) == HomeSyncRoute::OnetCloud && !has_sync_license {
                                    show_upgrade_dialog(window, cx);
                                } else {
                                    this.trigger_sync(cx);
                                }
                            })),
                    )
                    // 冲突指示器
                    .when(has_conflicts, |this| {
                        this.child(
                            Button::new("conflict-button")
                                .icon(IconName::TriangleAlert)
                                .label(format!("{}", conflict_count))
                                .ghost()
                                .text_color(cx.theme().warning)
                                .tooltip(if route == HomeSyncRoute::Personal {
                                    t!(
                                        "Home.personal_sync_conflict_tooltip",
                                        count = conflict_count
                                    )
                                    .to_string()
                                } else {
                                    t!("Home.conflict_tooltip", count = conflict_count).to_string()
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if sync_route(cx) == HomeSyncRoute::Personal {
                                        crate::personal_sync_conflicts::show_personal_conflict_dialog(
                                            window,
                                            cx,
                                        );
                                    } else {
                                        this.show_conflict_dialog(window, cx);
                                    }
                                })),
                        )
                    })
                    // 密钥菜单
                    .child(
                        Button::new("key-menu-button")
                            .icon(IconName::Key)
                            .label(t!("Encryption.keys").to_string())
                            .dropdown_caret(true)
                            .ghost()
                            .tooltip(t!("Encryption.keys_tooltip").to_string())
                            .dropdown_menu_with_anchor(Anchor::TopRight, {
                                let personal_view = view.clone();
                                let team_view = view.clone();
                                move |menu, _, _| {
                                    let personal_label = if has_master_key {
                                        t!("Encryption.personal_key_unlocked").to_string()
                                    } else {
                                        t!("Encryption.personal_key_locked").to_string()
                                    };
                                    let menu = menu.item(
                                        PopupMenuItem::new(personal_label)
                                            .icon(IconName::User)
                                            .on_click({
                                                let personal_view = personal_view.clone();
                                                move |_, window, cx| {
                                                    personal_view.update(cx, |home, cx| {
                                                        home.show_encryption_key_dialog(window, cx);
                                                    });
                                                }
                                            }),
                                    );

                                    if show_team_key_menu_item {
                                        menu.item(
                                            PopupMenuItem::new(
                                                t!("Encryption.team_key").to_string(),
                                            )
                                            .icon(IconName::Building2)
                                            .on_click({
                                                let team_view = team_view.clone();
                                                move |_, window, cx| {
                                                    team_view.update(cx, |home, cx| {
                                                        home.add_team_key_settings_tab(window, cx);
                                                    });
                                                }
                                            }),
                                        )
                                    } else {
                                        menu
                                    }
                                }
                            }),
                    ),
            )
            // ===== 右侧操作区 =====
            .child(
                h_flex()
                    .min_w_0()
                    .flex_wrap()
                    .gap_1()
                    .items_center()
                    .child(
                        Input::new(&self.search_input)
                            .cleanable(true)
                            .w(if legacy { px(240.0) } else { px(220.0) })
                            .bg(cx.theme().muted),
                    )
                    // 布局切换按钮
                    .child({
                        let is_card = self.connection_layout == ConnectionLayout::Card;
                        let tooltip = if is_card {
                            t!("Home.list_view").to_string()
                        } else {
                            t!("Home.card_view").to_string()
                        };
                        Button::new("layout-toggle")
                            .icon(if is_card {
                                IconName::LayoutDashboard
                            } else {
                                IconName::Menu
                            })
                            .ghost()
                            .tooltip(tooltip)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.connection_layout = this.connection_layout.toggle();
                                let layout = this.connection_layout.into();
                                AppSettings::update_and_save(cx, |settings| {
                                    settings.home_connection_layout = layout;
                                });
                                cx.notify();
                            }))
                    })
                    // 刷新按钮
                    .child(
                        Button::new("refresh-button")
                            .icon(IconName::Refresh)
                            .ghost()
                            .tooltip(t!("Home.refresh"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_local_home_data(cx);
                            })),
                    )
                    // 工作区筛选
                    .child(workspace_filter),
            )
    }
}
