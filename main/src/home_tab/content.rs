use super::*;

/// 非卡片布局下最近区固定容量（历史行为：最多 4 条）。
const RECENT_ROW_FALLBACK: usize = 4;

impl HomePage {
    pub(crate) fn set_connection_layout(
        &mut self,
        layout: HomeConnectionLayout,
        cx: &mut Context<Self>,
    ) {
        self.connection_layout = layout.into();
        self.sync_sidebar_home_embedded(cx);
        cx.notify();
    }

    /// 让常驻侧栏知道是否作为主页 Tree 布局嵌入渲染。
    fn sync_sidebar_home_embedded(&self, cx: &mut Context<Self>) {
        let embedded = self.connection_layout == ConnectionLayout::Tree;
        if let Some(sidebar) = &self.connection_sidebar {
            sidebar.update(cx, |sidebar, cx| sidebar.set_home_embedded(embedded, cx));
        }
    }

    pub(super) fn render_content_area(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.connection_layout == ConnectionLayout::Tree {
            if let Some(sidebar) = self.connection_sidebar.clone() {
                // 作为子视图渲染侧栏实体：树在 Render 阶段自己 read(HomePage)，
                // 不在本页租约内重入读自身（修复布局切换瞬间 panic）。
                sidebar.update(cx, |sidebar, cx| sidebar.set_home_embedded(true, cx));
                return sidebar.into_any_element();
            }
        }
        let query = self.search_query.read(cx).to_lowercase();
        self.render_workspace_view(
            &query,
            self.selected_connection_id,
            self.connection_layout,
            window,
            cx,
        )
    }

    fn home_groups(
        &self,
        query: &str,
        cx: &App,
    ) -> Vec<(Option<i64>, String, Vec<StoredConnection>)> {
        let mut groups: Vec<_> = self
            .workspaces
            .iter()
            .filter(|ws| {
                self.filtered_workspace_ids.is_empty()
                    || ws
                        .id
                        .is_some_and(|id| self.filtered_workspace_ids.contains(&id))
            })
            .map(|ws| (ws.id, ws.name.clone(), Vec::new()))
            .collect();
        if self.filtered_workspace_ids.is_empty() {
            groups.push((
                None,
                t!("Home.unassigned_workspace").to_string(),
                Vec::new(),
            ));
        }
        for (id, _, connections) in &mut groups {
            *connections = self
                .connections
                .iter()
                .filter(|conn| conn.workspace_id == *id)
                .filter(|conn| {
                    self.match_connection_type(conn) && self.match_connection(conn, query)
                })
                .cloned()
                .collect();
            crate::connection_sort::sort_connections(
                connections,
                AppSettings::global(cx).connection_sort_order,
            );
        }
        groups.retain(|(_, _, connections)| !connections.is_empty());
        groups
    }

    pub(super) fn render_workspace_view(
        &self,
        query: &str,
        selected: Option<i64>,
        layout: ConnectionLayout,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // 共享网格几何由内容区统一计算，所有分组使用同一组列边界（redesign §4.1）。
        let (columns, card_width) = match layout {
            ConnectionLayout::Card => grid::card_grid_metrics(
                window.bounds().size.width - self.home_sidebar_width(),
                window.rem_size(),
            ),
            _ => (RECENT_ROW_FALLBACK, px(0.0)),
        };
        let groups = self.home_groups(query, cx);
        let recent =
            recent::recent_connections(&self.connections, self.selected_filter, query, columns);
        let visible_count = groups
            .iter()
            .flat_map(|(_, _, connections)| connections.iter())
            .chain(recent.iter())
            .filter_map(|conn| conn.id)
            .collect::<HashSet<_>>()
            .len();
        // 分组间距 lg：小于页面边距、大于组头到内容的距离（redesign §9.1）。
        let mut body = v_flex()
            .w_full()
            .min_w_0()
            .gap_4()
            .child(self.render_content_heading(visible_count, cx));
        if visible_count == 0 {
            body = body.child(self.render_empty_home(cx));
        }
        if !recent.is_empty() {
            body = body.child(self.render_recent_group(recent, selected, layout, card_width, cx));
        }
        for (id, title, connections) in groups {
            body = body.child(self.render_home_group(
                id,
                title,
                connections,
                selected,
                layout,
                card_width,
                cx,
            ));
        }
        div()
            .id("home-content")
            .size_full()
            .min_w_0()
            .overflow_y_scroll()
            .p_5()
            .child(body)
            .into_any_element()
    }

    fn render_content_heading(&self, count: usize, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .gap_2()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("Connection.title").to_string()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Home.connection_count", count = count).to_string()),
            )
            .child(div().flex_1())
            // 展开/折叠命令归类进「分组」菜单，不再各占一个工具栏按钮（redesign §2.3/§4.3）。
            .child(self.render_group_menu(cx))
            .into_any_element()
    }

    fn render_group_menu(&self, cx: &Context<Self>) -> AnyElement {
        let view = cx.entity();
        Button::new("home-group-menu")
            .ghost()
            .small()
            .label(t!("Home.group_menu"))
            .dropdown_caret(true)
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let expand_view = view.clone();
                menu.item(
                    PopupMenuItem::new(t!("Home.expand_all").to_string())
                        .icon(IconName::ChevronDown)
                        .on_click(move |_, _, cx| {
                            expand_view.update(cx, |home, cx| {
                                home.collapsed_groups.clear();
                                home.recent_collapsed = false;
                                cx.notify();
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new(t!("Connection.collapse_all").to_string())
                        .icon(IconName::ChevronRight)
                        .on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |home, cx| {
                                    home.collapsed_groups = home
                                        .workspaces
                                        .iter()
                                        .map(|ws| ws.id)
                                        .chain([None])
                                        .collect();
                                    home.recent_collapsed = true;
                                    cx.notify();
                                });
                            }
                        }),
                )
            })
            .into_any_element()
    }

    fn render_empty_home(&self, cx: &mut Context<Self>) -> AnyElement {
        let initial = self.connections.is_empty();
        v_flex()
            .w_full()
            .py_8()
            .gap_3()
            .items_center()
            .child(Icon::new(IconName::Search).text_color(cx.theme().muted_foreground))
            .child(
                if initial {
                    t!("Home.empty_connections")
                } else {
                    t!("Home.no_filter_results")
                }
                .to_string(),
            )
            .child(
                Button::new("home-empty-action")
                    .primary()
                    .label(if initial {
                        t!("Home.new_connection")
                    } else {
                        t!("Home.clear_filters")
                    })
                    .on_click(cx.listener(move |home, _, window, cx| {
                        if initial {
                            home.show_new_connection_dialog(window, cx);
                        } else {
                            home.selected_filter = ConnectionType::All;
                            home.clear_workspace_filter(cx);
                            home.search_input
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            home.search_query.update(cx, |query, cx| {
                                query.clear();
                                cx.notify();
                            });
                            cx.notify();
                        }
                    })),
            )
            .into_any_element()
    }

    fn render_recent_group(
        &self,
        connections: Vec<StoredConnection>,
        selected: Option<i64>,
        layout: ConnectionLayout,
        card_width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let count = connections.len();
        v_flex()
            .id("home-recent-group")
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("home-recent-toggle")
                            .ghost()
                            .small()
                            .icon(if self.recent_collapsed {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            })
                            .on_click(cx.listener(|home, _, _, cx| {
                                home.recent_collapsed = !home.recent_collapsed;
                                cx.notify();
                            })),
                    )
                    .child(
                        // 历史语义图标（redesign §6.1：避免与收藏星标混淆），
                        // 资源经应用 AssetSource 内嵌。
                        Icon::default()
                            .path(NAVOP_HISTORY_ICON)
                            .with_size(IconSize::Small)
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .child(t!("Home.recent_connections").to_string()),
                    )
                    .child(render_group_count_badge(count, cx))
                    // 分组筛选不作用于最近区；仅在筛选生效时就近提示（redesign §6.2）。
                    .when(!self.filtered_workspace_ids.is_empty(), |header| {
                        header.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Home.recent_hint").to_string()),
                        )
                    }),
            )
            .when(!self.recent_collapsed, |group| {
                group.child(self.render_connections_grid(
                    connections,
                    selected,
                    layout,
                    true,
                    card_width,
                    cx,
                ))
            })
            .into_any_element()
    }

    fn render_home_group(
        &self,
        id: Option<i64>,
        title: String,
        connections: Vec<StoredConnection>,
        selected: Option<i64>,
        layout: ConnectionLayout,
        card_width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self.collapsed_groups.contains(&id);
        v_flex()
            .id(SharedString::from(format!("home-group-{id:?}")))
            .w_full()
            .min_w_0()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("group-toggle-{id:?}")))
                            .ghost()
                            .small()
                            .icon(if collapsed {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            })
                            .label(title)
                            .on_click(cx.listener(move |home, _, _, cx| {
                                if !home.collapsed_groups.remove(&id) {
                                    home.collapsed_groups.insert(id);
                                }
                                cx.notify();
                            })),
                    )
                    .child(render_group_count_badge(connections.len(), cx)),
            )
            .when(!collapsed, |group| {
                group.child(self.render_connections_grid(
                    connections,
                    selected,
                    layout,
                    false,
                    card_width,
                    cx,
                ))
            })
            .into_any_element()
    }

    pub(super) fn render_connections_grid(
        &self,
        connections: Vec<StoredConnection>,
        selected: Option<i64>,
        layout: ConnectionLayout,
        recent: bool,
        card_width: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut grid = div()
            .flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .when(layout != ConnectionLayout::Card, |grid| grid.flex_col());
        if layout == ConnectionLayout::Card {
            grid = grid.flex_wrap();
        }
        for (index, conn) in connections.into_iter().enumerate() {
            grid = grid.child(match layout {
                // Tree 布局在 render_content_area 拦截；这里兜底按列表渲染。
                ConnectionLayout::List | ConnectionLayout::Tree => {
                    self.render_connection_list_item(conn, selected, index, recent, cx)
                }
                // 固定共享列宽：单项组与末行不拉宽（redesign §4.1）。
                ConnectionLayout::Card => div()
                    .w(card_width)
                    .flex_shrink_0()
                    .child(self.render_connection_card(conn, selected, index, recent, cx))
                    .into_any_element(),
            });
        }
        grid.into_any_element()
    }
}

/// 分组头数量徽标（demo：11px、muted、--hover 底、圆角 8px、tabular-nums）。
fn render_group_count_badge(count: usize, cx: &App) -> AnyElement {
    div()
        .px_1p5()
        .py_0p5()
        .rounded(px(8.0))
        .bg(cx.theme().muted)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(count.to_string())
        .into_any_element()
}
