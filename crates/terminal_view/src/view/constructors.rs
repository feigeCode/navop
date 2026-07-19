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
        // 创建 Terminal Entity
        let duplicate_source = TerminalDuplicateSource::Local(config.clone());
        let local_working_dir = resolve_local_working_dir(config.working_dir.clone());
        let init_error = Rc::new(RefCell::new(None));
        let init_error_clone = init_error.clone();
        let terminal = cx.new(move |cx| {
            let (terminal, error) = Terminal::new_local_or_disconnected(config, cx);
            *init_error_clone.borrow_mut() = error;
            terminal
        });
        let view = Self::new_with_terminal(
            terminal,
            None,
            None,
            true,
            local_working_dir,
            tab_index,
            duplicate_source,
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
            terminal,
            connection_id,
            Some(stored_conn),
            sync_path_with_terminal,
            None,
            tab_index,
            duplicate_source,
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
            terminal,
            connection_id,
            None,
            true,
            None,
            tab_index,
            duplicate_source,
            window,
            cx,
        )
    }
}
