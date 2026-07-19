use super::*;

impl HomePage {
    pub(crate) fn show_connection_quick_open(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_master_key_ready_for_saved_connections(window, cx) {
            return;
        }

        let parent = cx.entity();
        let connections = self.connections.clone();
        let external_driver_registry = self.external_driver_registry.clone();
        let list = cx.new(|cx| {
            let mut delegate = ConnectionQuickOpenDelegate::new(parent, external_driver_registry);
            delegate.update_items(&connections);
            ListState::new(delegate, window, cx).searchable(true)
        });

        let list_for_focus = list.clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog
                .title(t!("Home.open_connection").to_string())
                .w(px(640.0))
                .margin_top(px(72.0))
                .close_button(false)
                .content({
                    let list = list.clone();
                    move |content, _window, _cx| {
                        content.p_0().child(
                            div().id("connection-quick-open-dialog").child(
                                List::new(&list)
                                    .search_placeholder(t!("Home.open_connection").to_string())
                                    .with_size(Size::Large)
                                    .max_h(px(420.0)),
                            ),
                        )
                    }
                })
        });
        // 将焦点设置到 List 搜索框，使上下键和 Enter 键可用
        list_for_focus.update(cx, |state, cx| {
            state.focus(window, cx);
        });
    }

    pub(crate) fn show_new_connection_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_connection_id = None;

        if !self.ensure_master_key_ready_for_new_connection(window, cx) {
            return;
        }

        let parent = cx.entity();
        let parent_window = window.window_handle();
        let external_driver_registry = self.external_driver_registry.clone();
        open_popup_window(
            PopupWindowOptions::new(t!("Home.new_connection").to_string()).size(1100.0, 700.0),
            move |window, cx| {
                cx.new(|cx| {
                    NewConnectionWindow::new(
                        parent,
                        parent_window,
                        external_driver_registry.clone(),
                        window,
                        cx,
                    )
                })
            },
            cx,
        );
    }

    pub(crate) fn open_connection_from_quick(
        &mut self,
        connection: &StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_connection_from_quick_with_mode(connection, TabOpenMode::Activate, window, cx);
    }

    pub(crate) fn open_connection_from_quick_with_mode(
        &mut self,
        connection: &StoredConnection,
        open_mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_master_key_ready_for_saved_connections(window, cx) {
            return;
        }

        let connection = connection.clone();
        self.touch_connection_last_used(connection.id, cx);
        let workspace = connection
            .workspace_id
            .and_then(|id| self.workspaces.iter().find(|w| w.id == Some(id)).cloned());
        let strategy = build_connection_open_strategy(connection, workspace);
        strategy.open(self, open_mode, window, cx);
        cx.notify();
    }

    pub(super) fn touch_connection_last_used(
        &mut self,
        connection_id: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let Some(connection_id) = connection_id else {
            return;
        };
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let result = storage
            .get::<ConnectionRepository>()
            .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))
            .and_then(|repo| repo.touch_last_used(connection_id));

        if let Err(err) = result {
            tracing::warn!("更新连接最近使用时间失败: {err}");
            return;
        }
        self.load_connections(cx);
    }
}
