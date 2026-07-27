use std::collections::HashMap;

use gpui::{App, AppContext as _, Context, Entity, Subscription, Window};
use one_core::storage::models::StoredConnection;
use terminal::LocalConfig;
use terminal::terminal::TerminalConnectionKind;

use super::pane_tab_transfer::TerminalPaneTabMetadata;
use super::resize::WorkspaceSidebarResize;
use super::{TerminalPaneId, TerminalSplitTree};
use crate::view::{
    RecordingPlaybackViewConfig, TERMINAL_TOOLS_SIDEBAR_DEFAULT_WIDTH, TerminalView,
};

pub struct TerminalWorkspace {
    pub(super) active_pane_id: TerminalPaneId,
    pub(super) next_pane_id: u64,
    pub(super) panes: HashMap<TerminalPaneId, Entity<TerminalView>>,
    pub(super) split_tree: TerminalSplitTree,
    pub(super) sidebar_panel_size: gpui::Pixels,
    pub(super) sidebar_resizing: Option<WorkspaceSidebarResize>,
    pub(super) workspace_bounds: gpui::Bounds<gpui::Pixels>,
    pub(super) pane_tab_metadata: HashMap<TerminalPaneId, TerminalPaneTabMetadata>,
    pub(super) pane_subscriptions: HashMap<TerminalPaneId, Vec<Subscription>>,
}

impl TerminalWorkspace {
    pub fn new(config: LocalConfig, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_index(config, None, window, cx)
    }

    pub fn new_with_index(
        config: LocalConfig,
        tab_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let main = cx.new(|cx| {
            TerminalView::new_with_index(config, tab_index, window, cx).with_workspace_pane()
        });
        Self::from_pane(main, window, cx)
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
        let main = cx.new(|cx| {
            TerminalView::new_ssh_with_index(
                conn,
                tab_index,
                window,
                cx,
                working_dir,
                sync_path_with_terminal,
            )
            .with_workspace_pane()
        });
        Self::from_pane(main, window, cx)
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
        let main = cx.new(|cx| {
            TerminalView::new_serial_with_index(conn, tab_index, window, cx).with_workspace_pane()
        });
        Self::from_pane(main, window, cx)
    }

    pub fn new_recording_playback(
        config: RecordingPlaybackViewConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let main = cx.new(|cx| {
            TerminalView::new_recording_playback(config, window, cx).with_workspace_pane()
        });
        Self::from_pane(main, window, cx)
    }

    pub(super) fn from_pane(
        pane: Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_pane_id = TerminalPaneId::new(1);
        let mut panes = HashMap::new();
        panes.insert(initial_pane_id, pane.clone());
        let mut pane_tab_metadata = HashMap::new();
        pane_tab_metadata.insert(initial_pane_id, TerminalPaneTabMetadata::generated());
        let mut this = Self {
            active_pane_id: initial_pane_id,
            next_pane_id: 2,
            panes,
            split_tree: TerminalSplitTree::new(initial_pane_id),
            sidebar_panel_size: TERMINAL_TOOLS_SIDEBAR_DEFAULT_WIDTH,
            sidebar_resizing: None,
            workspace_bounds: gpui::Bounds::default(),
            pane_tab_metadata,
            pane_subscriptions: HashMap::new(),
        };
        this.subscribe_to_pane(initial_pane_id, pane, window, cx);
        this
    }

    pub fn connection_kind(&self, cx: &App) -> TerminalConnectionKind {
        self.active_pane().read(cx).connection_kind(cx)
    }

    pub fn connection_id(&self, cx: &App) -> Option<i64> {
        self.active_pane().read(cx).connection_id(cx)
    }

    pub(super) fn active_pane(&self) -> Entity<TerminalView> {
        self.panes
            .get(&self.active_pane_id)
            .cloned()
            .or_else(|| self.panes.values().next().cloned())
            .expect("terminal workspace must contain one pane")
    }
}
