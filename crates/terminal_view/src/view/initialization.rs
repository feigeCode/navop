use super::*;

impl TerminalView {
    pub(super) fn new_with_terminal(
        terminal: Entity<Terminal>,
        connection_id: Option<i64>,
        stored_connection: Option<StoredConnection>,
        sync_path_enabled: bool,
        local_working_dir: Option<PathBuf>,
        tab_index: Option<usize>,
        duplicate_source: TerminalDuplicateSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let blink_manager = cx.new(|_| BlinkCursor::new());

        // 获取初始颜色
        let colors = terminal.read(cx).term().lock().colors().clone();
        let connection_kind = terminal.read(cx).connection_kind();
        let is_local_terminal = connection_kind == TerminalConnectionKind::Local;

        // 终端默认跟随应用主题（需要在创建侧边栏之前）。
        let default_theme = TerminalTheme::from_application_theme(cx.theme());
        let default_font_size = px(TERMINAL_RESET_FONT_SIZE);
        let default_font_family: SharedString = default_monospace_font().into();
        let default_font_fallbacks = default_font_fallbacks();
        let default_line_height_scale = DEFAULT_LINE_HEIGHT_SCALE;
        let ssh_config = terminal.read(cx).ssh_config().cloned();
        let ssh_session_manager = terminal.read(cx).ssh_session_manager().cloned();
        let history_scope = terminal_history_scope(connection_kind, connection_id);
        let public_mcp_registration = {
            let terminal = terminal.read(cx);
            crate::public_mcp::register_terminal(terminal, cx)
        };
        let terminal_ai_resource = public_mcp_registration
            .as_ref()
            .and_then(TerminalPublicMcpRegistration::agent_resource);

        let command_bar = cx.new(|cx| {
            TerminalCommandBar::new(
                TerminalCommandBarConfig {
                    terminal: terminal.clone(),
                    connection_id,
                    colors: default_theme.colors(),
                },
                window,
                cx,
            )
        });

        let workspace_editor = is_local_terminal.then(|| {
            let theme =
                crate::sidebar::workspace_theme_from_terminal_colors(&default_theme.colors());
            cx.new(|_| WorkspaceEditor::new(theme))
        });
        let local_workspace = local_working_dir
            .clone()
            .zip(workspace_editor.clone())
            .map(|(root, editor)| LocalWorkspaceSidebar { root, editor });

        // 创建侧边栏（传递 StoredConnection 用于文件管理器）
        let sidebar = cx.new(|cx| {
            TerminalSidebar::new(
                connection_id,
                connection_kind,
                stored_connection,
                terminal_ai_resource,
                ssh_config,
                ssh_session_manager,
                local_workspace,
                &default_theme,
                default_font_size,
                default_font_family.clone(),
                sync_path_enabled,
                history_scope,
                window,
                cx,
            )
        });
        let sidebar_toolbar = cx.new(|_| TerminalSidebarToolbar::new(sidebar.clone()));
        let sidebar_tool_panels = SidebarPanel::all()
            .iter()
            .copied()
            .map(|panel| {
                let sidebar = sidebar.clone();
                (
                    panel,
                    cx.new(move |_| TerminalSidebarToolPanel::new(sidebar.clone(), panel)),
                )
            })
            .collect::<HashMap<_, _>>();

        // 订阅侧边栏事件（需要 window 以便弹确认对话框）
        let sidebar_subscription = cx.subscribe_in(&sidebar, window, Self::handle_sidebar_event);

        // 订阅 Terminal 事件
        let terminal_subscription = cx.subscribe_in(&terminal, window, Self::handle_terminal_event);
        let command_bar_subscription =
            cx.subscribe_in(&command_bar, window, Self::handle_command_bar_event);
        let workspace_editor_subscription = workspace_editor
            .as_ref()
            .map(|editor| cx.subscribe_in(editor, window, Self::handle_workspace_editor_event));

        // 订阅 BlinkCursor 变化
        let blink_subscription = cx.observe(&blink_manager, |this, _, cx| {
            cx.notify();
            let _ = this;
        });

        let focus_handle = cx.focus_handle();

        // 焦点获得/失去订阅
        let focus_subscription = cx.on_focus(&focus_handle, window, |this, _window, cx| {
            if this.cursor_blink_enabled {
                this.blink_manager.update(cx, BlinkCursor::start);
            }
            cx.emit(TerminalPaneEvent::Focused);
        });
        let blur_subscription = cx.on_blur(&focus_handle, window, |this, _window, cx| {
            if this.cursor_blink_enabled {
                this.blink_manager.update(cx, BlinkCursor::stop);
            }
        });

        let mut subscriptions = Vec::new();
        subscriptions.push(sidebar_subscription);
        subscriptions.push(terminal_subscription);
        subscriptions.push(command_bar_subscription);
        if let Some(subscription) = workspace_editor_subscription {
            subscriptions.push(subscription);
        }
        subscriptions.push(blink_subscription);
        subscriptions.push(focus_subscription);
        subscriptions.push(blur_subscription);
        if let Some(global_settings) = cx.try_global::<GlobalTerminalLocalSettings>().cloned() {
            let settings_subscription = cx.subscribe_in(
                &global_settings.0,
                window,
                Self::handle_terminal_settings_event,
            );
            subscriptions.push(settings_subscription);
        }
        subscriptions
            .push(cx.observe_global_in::<AppSettings>(window, Self::handle_app_settings_changed));
        subscriptions.push(
            cx.observe_global_in::<gpui_component::Theme>(window, Self::handle_app_theme_changed),
        );

        let scrollbar_metrics = Rc::new(RefCell::new(TerminalScrollbarMetrics::default()));
        let scrollbar_handle = TerminalScrollbarHandle::new(
            terminal.read(cx).scroll_proxy(),
            scrollbar_metrics.clone(),
        );

        let mut this = Self {
            terminal,
            duplicate_source,
            local_working_dir: if is_local_terminal {
                local_working_dir
            } else {
                None
            },
            blink_manager,
            sidebar,
            workspace_editor,
            command_bar,
            sidebar_toolbar,
            sidebar_tool_panels,
            font_size: default_font_size,
            line_height: default_font_size * default_line_height_scale,
            font_family: default_font_family,
            font_fallbacks: default_font_fallbacks,
            line_height_scale: default_line_height_scale,
            cell_width: DEFAULT_CELL_WIDTH,
            font_metrics: None,
            // 初始化为 None，确保首次渲染时会触发 resize，
            // 将正确的终端尺寸发送给 PTY
            last_size: None,
            last_alt_screen: false,
            scroll_lines_accumulated: 0.0,
            mouse_state: MouseState::default(),
            block_selection: None,
            addon_manager: Self::create_addon_manager(),
            _subscriptions: subscriptions,
            mouse_position: None,
            render_cache: RenderCache::new(DEFAULT_ROWS, DEFAULT_COLS, colors),
            focus_handle,
            terminal_bounds: Bounds::default(),
            ime_state: None,
            history_prompt: HistoryPromptState::default(),
            shell_prompt_input_active: false,
            local_command_running: false,
            suggestion_debounce: None,
            recording_path_prompt_pending: false,
            recording_control_error: None,
            recording_ticker: None,
            cd_completion_client: None,
            cd_completion_cache: HashMap::new(),
            cd_completion_loading_parent: None,
            ssh_mfa_inputs: Vec::new(),
            focus_terminal_after_connect: false,
            current_theme: default_theme,
            tab_index,
            cursor_blink_enabled: false,
            confirm_multiline_paste: true,
            confirm_high_risk_command: true,
            auto_copy_on_select: true,
            autocomplete_enabled: true,
            middle_click_paste: true,
            right_click_paste: false,
            paste_image_upload: true,
            vim_scroll_to_arrow_keys: true,
            broadcast_client_id: None,
            sidebar_panel_size: TERMINAL_TOOLS_SIDEBAR_DEFAULT_WIDTH,
            resizing: None,
            view_bounds: Bounds::default(),
            scrollbar_metrics,
            scrollbar_handle,
            public_mcp_registration,
            render_mode: TerminalRenderMode::Embedded,
        };
        let initial_settings = current_settings(cx);
        this.apply_settings_snapshot(&initial_settings, window, cx);
        this.register_broadcast_input(cx);
        this
    }

    pub(super) fn register_broadcast_input(&mut self, cx: &mut Context<Self>) {
        if self.broadcast_client_id.is_some() {
            return;
        }

        let label = {
            let terminal = self.terminal.read(cx);
            if terminal.connection_kind() != TerminalConnectionKind::Ssh {
                return;
            }
            let base = terminal
                .connection_name()
                .filter(|name| !name.is_empty())
                .or_else(|| (!terminal.title().is_empty()).then(|| terminal.title()))
                .unwrap_or("SSH Terminal");
            self.tab_index
                .map(|index| format!("{base}({index})"))
                .unwrap_or_else(|| base.to_string())
        };

        init_broadcast_input_registry(cx);
        let view = cx.entity().downgrade();
        let Some(registry) = broadcast_input_registry(cx) else {
            return;
        };
        let client_id = registry.update(cx, |registry, cx| registry.register(label, view, cx));
        self.broadcast_client_id = Some(client_id);
    }

    pub(super) fn unregister_broadcast_input(&mut self, cx: &mut Context<Self>) {
        let Some(client_id) = self.broadcast_client_id.take() else {
            return;
        };
        if let Some(registry) = broadcast_input_registry(cx) {
            registry.update(cx, |registry, cx| registry.unregister(client_id, cx));
        }
    }

    pub(super) fn broadcast_user_input(&self, data: &[u8], cx: &mut Context<Self>) {
        let Some(client_id) = self.broadcast_client_id else {
            return;
        };
        let Some(registry) = broadcast_input_registry(cx) else {
            return;
        };
        let deliveries = registry.read(cx).deliveries_from(client_id, data);
        for (view, data) in deliveries {
            let _ = view.update(cx, |view, cx| {
                view.write_broadcast_input(data, cx);
            });
        }
    }

    pub(super) fn refresh_public_mcp_session(&self, cx: &mut Context<Self>) {
        let Some(registration) = &self.public_mcp_registration else {
            return;
        };
        registration.refresh(self.terminal.read(cx));
    }

    pub(super) fn unregister_public_mcp_session(&mut self, cx: &mut Context<Self>) {
        if let Some(registration) = self.public_mcp_registration.take() {
            registration.unregister(cx);
        }
    }
}
