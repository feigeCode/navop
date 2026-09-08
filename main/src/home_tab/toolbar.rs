use super::*;
use gpui_component::Selectable as _;
use one_core::settings::ConnectionSortOrder;

impl HomePage {
    pub(super) fn render_toolbar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group_filter =
            self.render_workspace_filter_popover(self.workspace_filter_open, window, cx);
        h_flex()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .gap_2()
            // 与内容区 p_5 同一左边线（redesign：统一左右内缩）。
            .px_5()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .flex_1()
                    // 搜索框独占剩余空间；其余工具栏控件 flex_shrink_0，
                    // 防止窄窗口时按钮收缩把 dropdown caret 裁掉。
                    .min_w(gpui::rems(4.0))
                    .when(window.bounds().size.width > px(1400.0), |search| {
                        search.min_w(gpui::rems(46.0))
                    })
                    .child(
                        Input::new(&self.search_input)
                            .cleanable(true)
                            .w_full()
                            .bg(cx.theme().muted),
                    ),
            )
            .child(
                // 次要工具保持为一条连续操作带，只由按钮自身表达 hover/selected。
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .child(self.render_home_type_filter(window, cx))
                    .child(group_filter)
                    .child(self.render_sort_button(cx))
                    .child(self.render_layout_button(cx))
                    .child(
                        IconButton::new(
                            "refresh-button",
                            Icon::new(IconName::Refresh)
                                .mono()
                                .with_size(IconSize::Small),
                        )
                        .ghost()
                        .flex_shrink_0()
                        .tooltip(t!("Home.refresh"))
                        .on_click(cx.listener(|home, _, _, cx| home.refresh_local_home_data(cx))),
                    )
                    .child(self.render_batch_toggle(cx)),
            )
            .child(
                // 主操作区分隔线（demo：1×18px --border）
                div()
                    .flex_shrink_0()
                    .w(px(1.0))
                    .h(px(18.0))
                    .mx_1()
                    .bg(cx.theme().border),
            )
            .child(self.render_new_connection_button(window, cx))
            .child(self.render_local_terminal_button(window, cx))
            .into_any_element()
    }

    fn render_home_type_filter(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let selected = self.selected_filter;
        let view = cx.entity();
        Button::new("home-type-filter")
            .ghost()
            .flex_shrink_0()
            // 窄窗口隐藏 label 后退化为图标按钮，同样需要保住 caret 宽度。
            .min_w(px(52.0))
            // 「全部类型」用 Apps 网格图标；星号在工具栏里像装饰符，语义不清。
            .icon(if selected == ConnectionType::All {
                IconName::Apps.mono().with_size(IconSize::Small)
            } else {
                connection_type_navigation_icon(selected, ConnectionVisualSize::Tree)
                    .with_size(IconSize::Small)
            })
            .when(window.bounds().size.width > px(1100.0), |button| {
                button.label(connection_type_label(selected))
            })
            .selected(selected != ConnectionType::All)
            .dropdown_caret(true)
            .tooltip(t!("Home.connection_filter"))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let view = view.clone();
                crate::connection_type_menu::build_filter_menu(
                    menu,
                    selected,
                    std::rc::Rc::new(move |filter, _, cx| {
                        view.update(cx, |home, cx| home.set_selected_filter(filter, cx));
                    }),
                )
            })
            .into_any_element()
    }

    fn render_sort_button(&self, cx: &Context<Self>) -> AnyElement {
        let selected = AppSettings::global(cx).connection_sort_order;
        IconButton::new(
            "home-sort",
            Icon::new(match selected {
                ConnectionSortOrder::Natural => IconName::SortAscending,
                ConnectionSortOrder::Lru => IconName::SortDescending,
            })
            .mono()
            .with_size(IconSize::Small),
        )
        .ghost()
        .flex_shrink_0()
        .tooltip(t!("Settings.General.ConnectionDisplay.connection_sort"))
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            [
                (
                    ConnectionSortOrder::Natural,
                    t!("Settings.General.ConnectionDisplay.connection_sort_natural"),
                ),
                (
                    ConnectionSortOrder::Lru,
                    t!("Settings.General.ConnectionDisplay.connection_sort_lru"),
                ),
            ]
            .into_iter()
            .fold(menu, |menu, (order, label)| {
                menu.item(
                    PopupMenuItem::new(label.to_string())
                        .checked(selected == order)
                        .on_click(move |_, _, cx| {
                            AppSettings::update_and_save(cx, |settings| {
                                settings.connection_sort_order = order
                            });
                        }),
                )
            })
        })
        .into_any_element()
    }

    /// 视图切换：图标反映当前布局，菜单内三项带勾选态（redesign §7.2：当前视图可发现）。
    fn render_layout_button(&self, cx: &Context<Self>) -> AnyElement {
        let current = self.connection_layout;
        let view = cx.entity();
        let icon = match current {
            ConnectionLayout::Card => IconName::LayoutDashboard,
            ConnectionLayout::List => IconName::Menu,
            ConnectionLayout::Tree => IconName::Network,
        };
        IconButton::new(
            "layout-toggle",
            Icon::new(icon).mono().with_size(IconSize::Small),
        )
        .ghost()
        .flex_shrink_0()
        .tooltip(t!("Settings.General.ConnectionDisplay.connection_layout"))
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            [
                (ConnectionLayout::Card, t!("Home.card_view")),
                (ConnectionLayout::List, t!("Home.list_view")),
                (ConnectionLayout::Tree, t!("Home.tree_view")),
            ]
            .into_iter()
            .fold(menu, |menu, (layout, label)| {
                let view = view.clone();
                menu.item(
                    PopupMenuItem::new(label.to_string())
                        .checked(layout == current)
                        .on_click(move |_, _, cx| {
                            view.update(cx, |home, cx| {
                                home.set_connection_layout(layout.into(), cx);
                                AppSettings::update_and_save(cx, |settings| {
                                    settings.home_connection_layout = layout.into()
                                });
                            });
                        }),
                )
            })
        })
        .into_any_element()
    }

    /// 批量操作入口：选中态保持高亮；卡片/列表/树共享同一批量选择状态。
    fn render_batch_toggle(&self, cx: &Context<Self>) -> AnyElement {
        Button::new("home-batch-toggle")
            .ghost()
            .flex_shrink_0()
            .icon(
                Icon::new(IconName::ListChecks)
                    .mono()
                    .with_size(IconSize::Small),
            )
            .selected(self.batch_mode_active())
            .tooltip(t!("Connection.batch_operations"))
            .on_click(cx.listener(|home, _, _, cx| {
                let next = !home.batch_mode_active();
                home.set_batch_mode(next, cx);
            }))
            .into_any_element()
    }

    fn render_new_connection_button(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let home = cx.entity();
        DropdownButton::new("home-new-dropdown")
            .flex_shrink_0()
            .button(
                Button::new("new-connect-button")
                    .primary()
                    .icon(Icon::new(IconName::Plus).mono().with_size(IconSize::Small))
                    .when(window.bounds().size.width > px(1000.0), |button| {
                        button.label(t!("Home.new_connection"))
                    })
                    .tooltip(home_shortcuts::new_connection_tooltip(cx))
                    .on_click(cx.listener(|home, _, window, cx| {
                        home.show_new_connection_dialog(window, cx)
                    })),
            )
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let group_home = home.clone();
                let import_home = home.clone();
                menu.item(
                    PopupMenuItem::new(t!("Workspace.new").to_string())
                        .icon(IconName::FolderOpen)
                        .on_click(move |_, window, cx| {
                            let sort_order = group_home.read(cx).workspaces.len() as i32;
                            show_workspace_dialog(
                                group_home.clone(),
                                WorkspaceDialogConfig {
                                    initial_sort_order: Some(sort_order),
                                    ..Default::default()
                                },
                                window,
                                cx,
                            );
                        }),
                )
                .separator()
                .item(
                    PopupMenuItem::new(t!("Home.other_app_import").to_string())
                        .icon(IconName::Upload)
                        .on_click(move |_, window, cx| {
                            show_connection_import_window(import_home.clone(), window, cx)
                        }),
                )
            })
            .into_any_element()
    }
}
