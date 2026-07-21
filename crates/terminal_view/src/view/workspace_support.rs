use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalPaneEvent {
    Focused,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TerminalRenderMode {
    #[default]
    Embedded,
    WorkspacePane,
}

#[derive(Clone)]
pub(crate) struct TerminalWorkspaceSidebarSnapshot {
    pub(crate) layout: TerminalToolDockLayout,
    pub(crate) sidebar: Entity<TerminalSidebar>,
    pub(crate) toolbar: Entity<TerminalSidebarToolbar>,
    pub(crate) panels: HashMap<SidebarPanel, Entity<TerminalSidebarToolPanel>>,
    pub(crate) colors: crate::theme::TerminalColors,
}

impl TerminalView {
    pub(crate) fn with_workspace_pane(mut self) -> Self {
        self.render_mode = TerminalRenderMode::WorkspacePane;
        self
    }

    pub(crate) fn duplicate_source_snapshot(&self, cx: &App) -> TerminalDuplicateSource {
        let current_working_dir = self
            .terminal
            .read(cx)
            .current_working_dir()
            .map(str::to_string);
        terminal_duplicate_source_with_cwd(
            self.duplicate_source.clone(),
            current_working_dir.as_deref(),
        )
    }

    pub(crate) fn duplicate_supported(&self) -> bool {
        terminal_tab_duplicate_supported(&self.duplicate_source)
    }

    pub(crate) fn new_from_duplicate_source(
        source: TerminalDuplicateSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        match source {
            TerminalDuplicateSource::Local(config) => {
                Self::new_with_index(config, None, window, cx)
            }
            TerminalDuplicateSource::Serial(connection) => {
                Self::new_serial_with_index(connection, None, window, cx)
            }
            TerminalDuplicateSource::Ssh {
                connection,
                working_dir,
                sync_path_with_terminal,
            } => Self::new_ssh_with_index(
                connection,
                None,
                window,
                cx,
                working_dir.as_deref(),
                sync_path_with_terminal,
            ),
        }
    }

    pub(crate) fn requires_close_confirmation(&self, cx: &App) -> bool {
        if self
            .workspace_editor
            .as_ref()
            .is_some_and(|editor| editor.read(cx).has_dirty_tabs(cx))
        {
            return true;
        }
        let terminal = self.terminal.read(cx);
        should_confirm_local_terminal_close(
            terminal.connection_kind(),
            self.local_command_running,
            terminal.mode(),
            terminal.child_exited(),
        )
    }

    pub(crate) fn close_now(&mut self, cx: &mut Context<Self>) {
        self.close_terminal_now(cx);
    }

    pub(crate) fn workspace_sidebar_snapshot(&self, cx: &App) -> TerminalWorkspaceSidebarSnapshot {
        TerminalWorkspaceSidebarSnapshot {
            layout: self.terminal_tool_layout(cx),
            sidebar: self.sidebar.clone(),
            toolbar: self.sidebar_toolbar.clone(),
            panels: self.sidebar_tool_panels.clone(),
            colors: self.sidebar.read(cx).colors(),
        }
    }
}

impl EventEmitter<TerminalPaneEvent> for TerminalView {}
