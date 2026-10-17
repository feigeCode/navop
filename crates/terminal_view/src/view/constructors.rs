use super::*;

impl TerminalView {
    pub fn new(config: LocalConfig, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_index(config, None, window, cx)
    }

    pub fn new_with_index(
        config: LocalConfig,
        tab_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_local_view(config, tab_index, None, window, cx)
    }

    /// 扩展声明的终端型连接使用的本地终端:带固定标签标题,
    /// 子进程退出后可按 Enter 重启,复制标签会按同样的参数重新拉起。
    pub fn new_launched(
        config: LocalConfig,
        title: String,
        tab_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_local_view(config, tab_index, Some(title), window, cx)
    }

    /// `title` 为 `Some` 表示扩展声明的终端型连接:固定标题且可重启。
    fn new_local_view(
        config: LocalConfig,
        tab_index: Option<usize>,
        title: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // 创建 Terminal Entity
        let duplicate_source = match &title {
            Some(title) => TerminalDuplicateSource::Launched {
                config: config.clone(),
                title: title.clone(),
            },
            None => TerminalDuplicateSource::Local(config.clone()),
        };
        let local_working_dir = resolve_local_working_dir(config.working_dir.clone());
        // WSL 会话没有 Windows 工作目录，文件树需要改指发行版文件系统
        let workspace_source = resolve_local_workspace_source(&config);
        let init_error = Rc::new(RefCell::new(None));
        let init_error_clone = init_error.clone();
        let terminal = cx.new(move |cx| {
            let (mut terminal, error) = if title.is_some() {
                Terminal::new_local_restartable_or_disconnected(config, cx)
            } else {
                Terminal::new_local_or_disconnected(config, cx)
            };
            *init_error_clone.borrow_mut() = error;
            terminal.set_connection_name(title);
            terminal
        });
        let view = Self::new_with_terminal(
            TerminalViewInit {
                terminal,
                connection_id: None,
                stored_connection: None,
                sync_path_enabled: true,
                local_working_dir,
                workspace_source,
                tab_index,
                duplicate_source: Some(duplicate_source),
                recording_playback_name: None,
                session_log_name: None,
            },
            window,
            cx,
        );

        if let Some(error) = init_error.borrow_mut().take() {
            window.push_notification(
                Notification::error(
                    t!("TerminalView.local_terminal_create_failed", error = error).to_string(),
                )
                .autohide(true),
                cx,
            );
        }

        view
    }

    pub fn new_ssh(conn: StoredConnection, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_ssh_with_index(conn, None, window, cx, None, true)
    }

    pub fn new_ssh_with_index(
        conn: StoredConnection,
        tab_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
        working_dir: Option<&str>,
        sync_path_with_terminal: bool,
    ) -> Self {
        // 创建 SSH Terminal Entity
        let connection_id = conn.id;
        let stored_conn = conn.clone();
        let duplicate_source = TerminalDuplicateSource::Ssh {
            connection: stored_conn.clone(),
            working_dir: working_dir.map(str::to_string),
            sync_path_with_terminal,
        };
        let terminal =
            cx.new(|cx| Terminal::new_ssh(conn, cx, working_dir, sync_path_with_terminal));
        Self::new_with_terminal(
            TerminalViewInit {
                terminal,
                connection_id,
                stored_connection: Some(stored_conn),
                sync_path_enabled: sync_path_with_terminal,
                local_working_dir: None,
                workspace_source: None,
                tab_index,
                duplicate_source: Some(duplicate_source),
                recording_playback_name: None,
                session_log_name: None,
            },
            window,
            cx,
        )
    }

    pub fn new_recording_playback(
        config: RecordingPlaybackViewConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (playback, display_name) = config.into_parts();
        let display_name = recording_playback_display_name(display_name.as_ref());
        let terminal = cx.new(|cx| Terminal::new_recording_playback(playback, cx));

        Self::new_with_terminal(
            TerminalViewInit {
                terminal,
                connection_id: None,
                stored_connection: None,
                sync_path_enabled: false,
                local_working_dir: None,
                workspace_source: None,
                tab_index: None,
                duplicate_source: None,
                recording_playback_name: Some(display_name),
                session_log_name: None,
            },
            window,
            cx,
        )
    }

    pub fn new_session_log(
        config: SessionLogViewConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (playback, display_name) = config.into_parts();
        let display_name = recording_playback_display_name(display_name.as_ref());
        let terminal = cx.new(|cx| Terminal::new_session_log(playback, cx));

        Self::new_with_terminal(
            TerminalViewInit {
                terminal,
                connection_id: None,
                stored_connection: None,
                sync_path_enabled: false,
                local_working_dir: None,
                workspace_source: None,
                tab_index: None,
                duplicate_source: None,
                recording_playback_name: None,
                session_log_name: Some(display_name),
            },
            window,
            cx,
        )
    }

    pub fn new_serial(conn: StoredConnection, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_serial_with_index(conn, None, window, cx)
    }

    pub fn new_serial_with_index(
        conn: StoredConnection,
        tab_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let connection_id = conn.id;
        let duplicate_source = TerminalDuplicateSource::Serial(conn.clone());
        let terminal = cx.new(|cx| Terminal::new_serial(conn, cx));
        // 串口不传 stored_connection，避免创建文件管理器面板
        Self::new_with_terminal(
            TerminalViewInit {
                terminal,
                connection_id,
                stored_connection: None,
                sync_path_enabled: true,
                local_working_dir: None,
                workspace_source: None,
                tab_index,
                duplicate_source: Some(duplicate_source),
                recording_playback_name: None,
                session_log_name: None,
            },
            window,
            cx,
        )
    }

    pub fn new_telnet(conn: StoredConnection, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_telnet_with_index(conn, None, window, cx)
    }

    pub fn new_telnet_with_index(
        conn: StoredConnection,
        tab_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let connection_id = conn.id;
        let duplicate_source = TerminalDuplicateSource::Telnet(conn.clone());
        let terminal = cx.new(|cx| Terminal::new_telnet(conn, cx));
        // Telnet 不传 stored_connection，避免创建文件管理器面板
        Self::new_with_terminal(
            TerminalViewInit {
                terminal,
                connection_id,
                stored_connection: None,
                sync_path_enabled: true,
                local_working_dir: None,
                workspace_source: None,
                tab_index,
                duplicate_source: Some(duplicate_source),
                recording_playback_name: None,
                session_log_name: None,
            },
            window,
            cx,
        )
    }
}
