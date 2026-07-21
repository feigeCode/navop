use super::*;

impl HomePage {
    pub fn new(
        tab_container: Entity<TabContainer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_query = cx.new(|_| String::new());
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("Home.search_placeholder"))
                .clean_on_escape()
        });

        // 订阅搜索输入变化
        let query_clone = search_query.clone();
        cx.subscribe_in(
            &search_input,
            window,
            move |_this, _input, event, _window, cx| {
                if let InputEvent::Change = event {
                    query_clone.update(cx, |q, cx| {
                        *q = _input.read(cx).text().to_string();
                        cx.notify();
                    });
                    cx.notify();
                }
            },
        )
        .detach();

        let mut page = Self {
            focus_handle: cx.focus_handle(),
            selected_filter: ConnectionType::All,
            connection_layout: AppSettings::current(cx).home_connection_layout.into(),
            home_page_style: AppSettings::current(cx).home_page_style,
            persistent_sidebar_expanded: AppSettings::current(cx).connection_sidebar_expanded,
            workspaces: Vec::new(),
            connections: Vec::new(),
            tab_container,
            search_input,
            search_query,
            editing_connection_id: None,
            selected_connection_id: None,
            connection_scroll_handle: UniformListScrollHandle::new(),
            filtered_workspace_ids: HashSet::new(),
            workspace_filter_open: false,
            workspace_filter_list: None,
            _subscriptions: Vec::new(),
            cloud_sync_service: Arc::new(std::sync::RwLock::new(CloudSyncService::new())),
            cloud_error: None,
            syncing: false,
            sync_requested: false,
            pending_conflicts: Vec::new(),
            auth_service: crate::auth::get_auth_service(cx),
            current_user: None,
            logging_in: false,
            auth_error: None,
            master_key_unlock_prompt_pending: false,
            master_key_dialog_open: false,
            sidebar_collapsed: false,
            team_options: Vec::new(),
            port_forwarding_runtime: Arc::new(
                tokio::sync::Mutex::new(PortForwardingRuntime::new()),
            ),
            external_driver_registry: IpcDriverRegistry::empty(),
        };

        // 异步加载工作区
        page.load_workspaces(cx);

        // 尝试从存储后端恢复主密钥
        let key_restored = crypto::try_restore_master_key();
        if key_restored {
            tracing::info!("已恢复主密钥");
        } else if crypto::has_repo_password_set() {
            // 有验证文件但恢复失败，提示用户需要重新输入密钥
            tracing::warn!("密钥恢复失败，需要用户重新输入主密钥");
            page.master_key_unlock_prompt_pending = true;
        } else {
            tracing::info!("首次使用，需要设置主密钥");
        }

        // 在恢复主密钥后再加载连接，避免解密阶段出现空密码
        page.load_connections(cx);

        // 尝试恢复登录会话
        page.try_restore_session(cx);

        // 订阅全局连接事件，当连接创建/更新时刷新列表并自动同步
        if let Some(notifier) = get_notifier(cx) {
            cx.subscribe(
                &notifier,
                |this, _, event: &ConnectionDataEvent, cx| match event {
                    ConnectionDataEvent::ConnectionCreated { connection } => {
                        // 立即将新连接添加到列表，避免异步加载的时序问题
                        this.connections.push(connection.clone());
                        cx.notify();
                        // 然后异步重新加载以确保数据一致性
                        this.load_connections(cx);
                        // 如果已登录且密钥已解锁，自动触发同步
                        if should_auto_onet_cloud_sync(cx, this.current_user.is_some()) {
                            tracing::info!("连接数据变化，自动触发云同步");
                            this.trigger_sync(cx);
                        }
                    }
                    ConnectionDataEvent::ConnectionUpdated { connection } => {
                        // 立即更新列表中的连接，避免异步加载的时序问题
                        if let Some(pos) =
                            this.connections.iter().position(|c| c.id == connection.id)
                        {
                            this.connections[pos] = connection.clone();
                        } else {
                            // 如果找不到，添加到列表
                            this.connections.push(connection.clone());
                        }
                        cx.notify();
                        // 然后异步重新加载以确保数据一致性
                        this.load_connections(cx);
                        // 如果已登录且密钥已解锁，自动触发同步
                        if should_auto_onet_cloud_sync(cx, this.current_user.is_some()) {
                            tracing::info!("连接数据变化，自动触发云同步");
                            this.trigger_sync(cx);
                        }
                    }
                    ConnectionDataEvent::ConnectionDeleted { connection_id, .. } => {
                        // 立即从列表中移除连接
                        this.connections.retain(|c| c.id != Some(*connection_id));
                        cx.notify();
                        // 然后异步重新加载以确保数据一致性
                        this.load_connections(cx);
                        // 如果已登录且密钥已解锁，自动触发同步
                        if should_auto_onet_cloud_sync(cx, this.current_user.is_some()) {
                            tracing::info!("连接数据变化，自动触发云同步");
                            this.trigger_sync(cx);
                        }
                    }
                    ConnectionDataEvent::WorkspaceCreated { .. }
                    | ConnectionDataEvent::WorkspaceUpdated { .. }
                    | ConnectionDataEvent::WorkspaceDeleted { .. } => {
                        this.load_workspaces(cx);
                        // 如果已登录且密钥已解锁，自动触发同步
                        if should_auto_onet_cloud_sync(cx, this.current_user.is_some()) {
                            tracing::info!("工作区数据变化，自动触发云同步");
                            this.trigger_sync(cx);
                        }
                    }
                    ConnectionDataEvent::SchemaChanged { .. } => {
                        // SchemaChanged 由 db_tree_view 处理，此处无需操作
                    }
                    ConnectionDataEvent::CloudSyncRequested => {
                        this.trigger_sync(cx);
                    }
                    ConnectionDataEvent::TeamCacheUpdated => {
                        this.load_team_options(cx);
                    }
                },
            )
            .detach();
        }

        page
    }
}
