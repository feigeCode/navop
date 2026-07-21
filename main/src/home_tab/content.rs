use super::*;

impl HomePage {
    pub(crate) fn set_connection_layout(
        &mut self,
        layout: HomeConnectionLayout,
        cx: &mut Context<Self>,
    ) {
        self.connection_layout = layout.into();
        cx.notify();
    }

    pub(super) fn render_content_area(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let search_query = self.search_query.read(cx).to_lowercase();
        let selected_id = self.selected_connection_id;
        let layout = self.connection_layout;
        self.render_workspace_view(&search_query, selected_id, layout, cx)
            .into_any_element()
    }

    pub(super) fn render_workspace_view(
        &self,
        search_query: &str,
        selected_id: Option<i64>,
        layout: ConnectionLayout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let workspaces_with_connections: Vec<_> = self
            .workspaces
            .iter()
            .filter(|ws| {
                if self.filtered_workspace_ids.is_empty() {
                    return true;
                }
                match ws.id {
                    Some(id) => self.filtered_workspace_ids.contains(&id),
                    None => true,
                }
            })
            .map(|ws| {
                let conn_list: Vec<_> = self
                    .connections
                    .iter()
                    .filter(|conn| conn.workspace_id == ws.id)
                    .filter(|conn| self.match_connection(conn, search_query))
                    .filter(|conn| self.match_connection_type(conn))
                    .cloned()
                    .collect();
                (ws.clone(), conn_list)
            })
            .collect();

        let unassigned_connections: Vec<_> = self
            .connections
            .iter()
            .filter(|conn| conn.workspace_id.is_none())
            .filter(|conn| self.match_connection(conn, search_query))
            .filter(|conn| self.match_connection_type(conn))
            .cloned()
            .collect();
        let has_workspaces = self.workspaces.iter().any(|ws| ws.id.is_some());

        if layout == ConnectionLayout::List && !has_workspaces {
            return div()
                .id("home-content")
                .size_full()
                .min_w_0()
                .overflow_y_scroll()
                .px_4()
                .py_3()
                .child(div().w_full().max_w(px(1160.0)).mx_auto().child(
                    self.render_connection_uniform_list(unassigned_connections, selected_id, cx),
                ))
                .into_any_element();
        }

        div()
            .id("home-content")
            .size_full()
            .min_w_0()
            .overflow_y_scroll()
            .px_4()
            .py_3()
            .child(div().w_full().max_w(px(1160.0)).mx_auto().child({
                let mut container = v_flex().gap_5().w_full().min_w_0();

                // 过滤掉空的工作区
                for (workspace, connections) in workspaces_with_connections {
                    if connections.is_empty() {
                        continue;
                    }
                    container = container.child(self.render_workspace_section(
                        workspace,
                        connections,
                        selected_id,
                        layout,
                        cx,
                    ));
                }

                // 如果用户没有设置工作区，直接显示连接列表；否则显示未分配工作区
                if !unassigned_connections.is_empty() {
                    if has_workspaces {
                        container = container.child(self.render_unassigned_section(
                            unassigned_connections,
                            selected_id,
                            layout,
                            cx,
                        ));
                    } else {
                        // 没有工作区时，直接显示连接卡片
                        container = container.child(self.render_connections_grid(
                            unassigned_connections,
                            selected_id,
                            layout,
                            cx,
                        ));
                    }
                }

                container
            }))
            .into_any_element()
    }

    pub(super) fn render_workspace_section(
        &self,
        workspace: Workspace,
        connections: Vec<StoredConnection>,
        selected_id: Option<i64>,
        layout: ConnectionLayout,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let workspace_id = workspace.id;
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .py_0p5()
                    .child(
                        Icon::new(IconName::AppsColor)
                            .color()
                            .with_size(Size::Medium),
                    )
                    .child(
                        div()
                            .id(ElementId::Name(SharedString::from(format!(
                                "workspace-name-{}",
                                workspace_id.unwrap_or(0)
                            ))))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().foreground)
                            .child(workspace.name.clone()),
                    )
                    .child(
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded_full()
                            .bg(cx.theme().muted)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                t!("Home.connection_count", count = connections.len()).to_string(),
                            ),
                    )
                    .child(div().flex_1()),
            )
            .when(!connections.is_empty(), |this| {
                let mut container = match layout {
                    ConnectionLayout::List => v_flex().w_full().min_w_0().gap_1(),
                    ConnectionLayout::Card => div().flex().flex_wrap().w_full().min_w_0().gap_3(),
                };

                for (idx, conn) in connections.iter().enumerate() {
                    container = match layout {
                        ConnectionLayout::List => container.child(
                            self.render_connection_list_item(conn.clone(), selected_id, idx, cx),
                        ),
                        ConnectionLayout::Card => {
                            container.child(div().w(px(280.0)).flex_shrink_0().child(
                                self.render_connection_card(conn.clone(), selected_id, idx, cx),
                            ))
                        }
                    };
                }

                this.child(container)
            })
    }

    pub(super) fn render_connections_grid(
        &self,
        connections: Vec<StoredConnection>,
        selected_id: Option<i64>,
        layout: ConnectionLayout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if layout == ConnectionLayout::List {
            return self.render_connection_uniform_list(connections, selected_id, cx);
        }

        let mut container = div().flex().flex_wrap().w_full().min_w_0().gap_3();
        for (idx, conn) in connections.into_iter().enumerate() {
            container = container.child(
                div()
                    .w(px(280.0))
                    .flex_shrink_0()
                    .child(self.render_connection_card(conn, selected_id, idx, cx)),
            );
        }
        container.into_any_element()
    }

    pub(super) fn render_connection_uniform_list(
        &self,
        connections: Vec<StoredConnection>,
        selected_id: Option<i64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let item_count = connections.len();
        uniform_list("home-connection-list", item_count, {
            cx.processor(move |this: &mut Self, range: Range<usize>, _window, cx| {
                range
                    .filter_map(|idx| {
                        let conn = connections.get(idx).cloned()?;
                        Some(this.render_connection_list_item(conn, selected_id, idx, cx))
                    })
                    .collect()
            })
        })
        .size_full()
        .track_scroll(&self.connection_scroll_handle)
        .with_sizing_behavior(ListSizingBehavior::Auto)
        .into_any_element()
    }

    pub(super) fn render_unassigned_section(
        &self,
        connections: Vec<StoredConnection>,
        selected_id: Option<i64>,
        layout: ConnectionLayout,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .py_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().foreground)
                            .child(
                                t!("Home.unassigned_workspace")
                                    .to_string()
                                    .into_any_element(),
                            ),
                    )
                    .child(
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded_full()
                            .bg(cx.theme().muted)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                t!("Home.connection_count", count = connections.len()).to_string(),
                            ),
                    ),
            )
            .child({
                let mut container = match layout {
                    ConnectionLayout::List => v_flex().w_full().min_w_0().gap_1(),
                    ConnectionLayout::Card => div().flex().flex_wrap().w_full().min_w_0().gap_3(),
                };

                for (idx, conn) in connections.into_iter().enumerate() {
                    container = match layout {
                        ConnectionLayout::List => container
                            .child(self.render_connection_list_item(conn, selected_id, idx, cx)),
                        ConnectionLayout::Card => container.child(
                            div()
                                .w(px(280.0))
                                .flex_shrink_0()
                                .child(self.render_connection_card(conn, selected_id, idx, cx)),
                        ),
                    };
                }
                container
            })
    }
}
