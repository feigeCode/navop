use super::*;

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for TerminalView {}

impl TabContent for TerminalView {
    fn content_key(&self) -> &'static str {
        "Terminal"
    }

    fn title(&self, cx: &App) -> SharedString {
        let terminal = self.terminal.read(cx);
        let base_title = if let Some(name) = terminal.connection_name() {
            name.to_string()
        } else if !terminal.title().is_empty() {
            terminal.title().to_string()
        } else {
            "Terminal".to_string()
        };

        // 如果有序号，添加到标题后
        if let Some(index) = self.tab_index {
            SharedString::from(format!("{}({})", base_title, index))
        } else {
            SharedString::from(base_title)
        }
    }

    fn icon(&self, cx: &App) -> Option<Icon> {
        if self.connection_kind(cx) == TerminalConnectionKind::Serial {
            Some(IconName::SerialPort.color())
        } else {
            Some(IconName::TerminalColor.color())
        }
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn can_duplicate(&self, _cx: &App) -> bool {
        terminal_tab_duplicate_supported(&self.duplicate_source)
    }

    fn duplicate(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Arc<dyn TabContentView>> {
        let source = self.duplicate_source_snapshot(cx);
        let duplicate = cx.new(|cx| Self::new_from_duplicate_source(source, window, cx));
        Some(Arc::new(duplicate))
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if self.requires_close_confirmation(cx) {
            return self.confirm_local_terminal_close(window, cx);
        }

        self.close_terminal_now(cx);
        Task::ready(true)
    }

    fn sidebar_contributions(&self, _cx: &App) -> Vec<SidebarContribution> {
        Vec::new()
    }
}
