use super::*;

/// 正在调整大小的面板
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResizingPanel {
    LeftSidebar,
    RightSidebar,
    BottomSidebar,
}

/// IME composition state.
pub(super) struct ImeState {
    pub(super) marked_range: Option<std::ops::Range<usize>>,
}

pub(super) struct SshMfaInput {
    pub(super) prompt: String,
    pub(super) echo: bool,
    pub(super) input: Entity<InputState>,
}

#[derive(Clone)]
pub(crate) enum TerminalDuplicateSource {
    Local(LocalConfig),
    Ssh {
        connection: StoredConnection,
        working_dir: Option<String>,
        sync_path_with_terminal: bool,
    },
    Serial(StoredConnection),
}

#[derive(Clone)]
pub(super) struct SshReconnectSource {
    pub(super) connection: StoredConnection,
    pub(super) working_dir: Option<String>,
    pub(super) sync_path_with_terminal: bool,
}

pub(super) fn resolve_ssh_reconnect_source(
    source: &TerminalDuplicateSource,
    load_connection: impl FnOnce(i64) -> anyhow::Result<Option<StoredConnection>>,
) -> anyhow::Result<Option<SshReconnectSource>> {
    let TerminalDuplicateSource::Ssh {
        connection,
        working_dir,
        sync_path_with_terminal,
    } = source
    else {
        return Ok(None);
    };

    let latest = match connection.id {
        Some(id) => load_connection(id)?
            .ok_or_else(|| anyhow::anyhow!("SSH connection {id} no longer exists"))?,
        None => connection.clone(),
    };
    Ok(Some(SshReconnectSource {
        connection: latest,
        working_dir: working_dir.clone(),
        sync_path_with_terminal: *sync_path_with_terminal,
    }))
}

pub(super) fn live_terminal_input_supported(
    live_connection_kind: Option<TerminalConnectionKind>,
) -> bool {
    live_connection_kind.is_some()
}

pub(super) fn live_ssh_feature_supported(
    live_connection_kind: Option<TerminalConnectionKind>,
) -> bool {
    live_connection_kind == Some(TerminalConnectionKind::Ssh)
}

pub(super) fn terminal_tab_duplicate_supported(
    source: Option<&TerminalDuplicateSource>,
    live_connection_kind: Option<TerminalConnectionKind>,
) -> bool {
    matches!(
        (source, live_connection_kind),
        (
            Some(TerminalDuplicateSource::Local(_)),
            Some(TerminalConnectionKind::Local),
        ) | (
            Some(TerminalDuplicateSource::Ssh { .. }),
            Some(TerminalConnectionKind::Ssh),
        ) | (
            Some(TerminalDuplicateSource::Serial(_)),
            Some(TerminalConnectionKind::Serial),
        )
    )
}

impl TerminalView {
    pub(super) fn accepts_live_terminal_input(&self, cx: &App) -> bool {
        live_terminal_input_supported(self.terminal.read(cx).live_connection_kind())
    }

    pub(super) fn is_live_ssh_terminal(&self, cx: &App) -> bool {
        live_ssh_feature_supported(self.terminal.read(cx).live_connection_kind())
    }
}

pub(super) fn terminal_duplicate_source_with_cwd(
    source: TerminalDuplicateSource,
    current_working_dir: Option<&str>,
) -> TerminalDuplicateSource {
    let Some(cwd) = current_working_dir.filter(|cwd| !cwd.trim().is_empty()) else {
        return source;
    };
    let cwd = cwd.to_string();

    match source {
        TerminalDuplicateSource::Local(mut config) => {
            config.working_dir = Some(cwd);
            TerminalDuplicateSource::Local(config)
        }
        TerminalDuplicateSource::Ssh {
            connection,
            sync_path_with_terminal,
            ..
        } => TerminalDuplicateSource::Ssh {
            connection,
            working_dir: Some(cwd),
            sync_path_with_terminal,
        },
        TerminalDuplicateSource::Serial(connection) => TerminalDuplicateSource::Serial(connection),
    }
}

/// Mouse interaction state.
#[derive(Default)]
pub(super) struct MouseState {
    pub(super) selecting: bool,
    pub(super) block_selecting: bool,
    pub(super) pending_sgr_left_press: Option<PendingSgrMousePress>,
    pub(super) last_click_point: Option<AlacPoint>,
    pub(super) click_count: u32,
    pub(super) last_click_time: Option<std::time::Instant>,
}

pub(super) struct PendingSgrMousePress {
    pub(super) point: AlacPoint,
    pub(super) position: Point<Pixels>,
    pub(super) modifiers: Modifiers,
}

#[derive(Debug, Clone)]
pub(super) struct TerminalScrollbarMetrics {
    pub(super) viewport_size: Size<Pixels>,
    pub(super) line_height: Pixels,
    pub(super) cell_width: Pixels,
}

impl Default for TerminalScrollbarMetrics {
    fn default() -> Self {
        Self {
            viewport_size: size(px(0.0), px(0.0)),
            line_height: px(1.0),
            cell_width: px(1.0),
        }
    }
}

#[derive(Clone)]
pub(super) struct TerminalFontMetrics {
    pub(super) requested_family: SharedString,
    pub(super) fallbacks: Vec<SharedString>,
    pub(super) font_size: Pixels,
    pub(super) effective_family: SharedString,
    pub(super) cell_width: Pixels,
}

impl TerminalFontMetrics {
    pub(super) fn matches(
        &self,
        requested_family: &SharedString,
        fallbacks: &[SharedString],
        font_size: Pixels,
    ) -> bool {
        &self.requested_family == requested_family
            && self.fallbacks == fallbacks
            && self.font_size == font_size
    }
}

#[derive(Clone)]
pub(super) struct TerminalScrollbarHandle {
    pub(super) proxy: TerminalScrollProxy,
    pub(super) metrics: Rc<RefCell<TerminalScrollbarMetrics>>,
    pub(super) future_display_offset: Rc<StdCell<Option<usize>>>,
}

impl TerminalScrollbarHandle {
    pub(super) fn new(
        proxy: TerminalScrollProxy,
        metrics: Rc<RefCell<TerminalScrollbarMetrics>>,
    ) -> Self {
        Self {
            proxy,
            metrics,
            future_display_offset: Rc::new(StdCell::new(None)),
        }
    }

    pub(super) fn take_future_display_offset(&self) -> Option<usize> {
        self.future_display_offset.take()
    }
}

impl ScrollbarHandle for TerminalScrollbarHandle {
    fn offset(&self) -> Point<Pixels> {
        let metrics = self.metrics.borrow();
        let line_height = metrics.line_height.max(px(1.0));
        // Snapshot terminal state in a single lock to avoid inconsistency
        let snapshot = self.proxy.snapshot();
        let max_offset = snapshot.history_size;
        let scroll_offset = max_offset.saturating_sub(snapshot.display_offset);
        Point::new(px(0.0), -(scroll_offset as f32 * line_height))
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        let metrics = self.metrics.borrow();
        let line_height = metrics.line_height.max(px(1.0));
        let snapshot = self.proxy.snapshot();
        let max_offset = snapshot.history_size as i32;
        if max_offset == 0 {
            return;
        }
        let offset_delta = (offset.y / line_height).round() as i32;
        let display_offset = (max_offset + offset_delta).clamp(0, max_offset) as usize;
        self.future_display_offset.set(Some(display_offset));
    }

    fn content_size(&self) -> Size<Pixels> {
        let metrics = self.metrics.borrow();
        let line_height = metrics.line_height.max(px(1.0));
        let snapshot = self.proxy.snapshot();
        let total_lines = snapshot.history_size + snapshot.screen_lines;
        let height = line_height * total_lines as f32;
        let width = metrics
            .viewport_size
            .width
            .max(metrics.cell_width * snapshot.columns as f32);
        size(width, height)
    }
}
