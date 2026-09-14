//! Low-frequency lifecycle counters for the remote file editor.
//!
//! Background: `reload_tab` / `save_tab` spawn detached `window.spawn` tasks
//! that used to capture a **strong** `Entity` of the editor window. A slow SFTP
//! round-trip therefore kept the whole popup window — and every tab it owned,
//! including their `EditorState` entities — alive after the user closed it.
//! Those tasks now hold a `WeakEntity`, and these counters make the remaining
//! in-flight work observable so a memory-growth report can tell
//! "the window closed" apart from "the work is gone".
//!
//! Counted gauges:
//!
//! * `live_editor_views` — `RemoteFileEditorWindow` entities alive
//! * `live_editor_tabs` — open tabs across all editor windows
//! * `pending_load_tasks` — in-flight remote reads
//! * `pending_save_tasks` — in-flight remote writes
//! * `active_parse_tasks` — in-flight language/parser loads
//!
//! Privacy: these logs intentionally record only `tab_id`, `size_bytes` and the
//! resolved language name. Remote paths can carry query-string credentials
//! (e.g. `…?token=…`), and file contents must never be logged.

use std::sync::atomic::{AtomicI64, Ordering};

/// Point-in-time copy of the editor lifecycle gauges.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct EditorLifecycleSnapshot {
    pub(crate) live_editor_views: i64,
    pub(crate) live_editor_tabs: i64,
    pub(crate) pending_load_tasks: i64,
    pub(crate) pending_save_tasks: i64,
    pub(crate) active_parse_tasks: i64,
}

impl EditorLifecycleSnapshot {
    /// Whether any in-flight task outlives its own window.
    pub(crate) const fn has_pending_work(&self) -> bool {
        self.pending_load_tasks != 0
            || self.pending_save_tasks != 0
            || self.active_parse_tasks != 0
    }

    /// Whether the editor is fully torn down.
    pub(crate) const fn is_torn_down(&self) -> bool {
        self.live_editor_views == 0 && self.live_editor_tabs == 0 && !self.has_pending_work()
    }
}

static LIVE_EDITOR_VIEWS: AtomicI64 = AtomicI64::new(0);
static LIVE_EDITOR_TABS: AtomicI64 = AtomicI64::new(0);
static PENDING_LOAD_TASKS: AtomicI64 = AtomicI64::new(0);
static PENDING_SAVE_TASKS: AtomicI64 = AtomicI64::new(0);
static ACTIVE_PARSE_TASKS: AtomicI64 = AtomicI64::new(0);

/// Kind of background editor task, used as the log `task` field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditorTaskKind {
    Load,
    Save,
    Parse,
}

impl EditorTaskKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Save => "save",
            Self::Parse => "parse",
        }
    }

    fn gauge(self) -> &'static AtomicI64 {
        match self {
            Self::Load => &PENDING_LOAD_TASKS,
            Self::Save => &PENDING_SAVE_TASKS,
            Self::Parse => &ACTIVE_PARSE_TASKS,
        }
    }
}

/// Returns a consistent-enough snapshot for diagnostics.
pub(crate) fn snapshot() -> EditorLifecycleSnapshot {
    EditorLifecycleSnapshot {
        live_editor_views: LIVE_EDITOR_VIEWS.load(Ordering::Relaxed),
        live_editor_tabs: LIVE_EDITOR_TABS.load(Ordering::Relaxed),
        pending_load_tasks: PENDING_LOAD_TASKS.load(Ordering::Relaxed),
        pending_save_tasks: PENDING_SAVE_TASKS.load(Ordering::Relaxed),
        active_parse_tasks: ACTIVE_PARSE_TASKS.load(Ordering::Relaxed),
    }
}

/// Emits one low-frequency lifecycle line at a stable `stage`.
pub(crate) fn log_snapshot(stage: &'static str) {
    let snapshot = snapshot();
    tracing::info!(
        target: "remote_file_editor::lifecycle",
        stage,
        live_editor_views = snapshot.live_editor_views,
        live_editor_tabs = snapshot.live_editor_tabs,
        pending_load_tasks = snapshot.pending_load_tasks,
        pending_save_tasks = snapshot.pending_save_tasks,
        active_parse_tasks = snapshot.active_parse_tasks,
        "remote file editor lifecycle counters"
    );
}

pub(crate) fn record_editor_view_created() {
    LIVE_EDITOR_VIEWS.fetch_add(1, Ordering::Relaxed);
}

/// Records that an editor window entity is being released.
///
/// Returns whether the editor is fully torn down afterwards. A `false` result
/// means some tab or background task outlived its own window, which is exactly
/// the condition this module exists to make visible.
pub(crate) fn record_editor_view_dropped() -> bool {
    LIVE_EDITOR_VIEWS.fetch_sub(1, Ordering::Relaxed);
    let snapshot = snapshot();
    log_snapshot("editor_view_dropped");
    let torn_down = snapshot.is_torn_down();
    if !torn_down {
        tracing::warn!(
            target: "remote_file_editor::lifecycle",
            stage = "editor_view_dropped_with_pending_work",
            live_editor_views = snapshot.live_editor_views,
            live_editor_tabs = snapshot.live_editor_tabs,
            pending_load_tasks = snapshot.pending_load_tasks,
            pending_save_tasks = snapshot.pending_save_tasks,
            active_parse_tasks = snapshot.active_parse_tasks,
            "an editor window was released while tabs or background tasks are still tracked"
        );
    }
    torn_down
}

pub(crate) fn record_editor_tab_created() {
    LIVE_EDITOR_TABS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_editor_tab_dropped() {
    LIVE_EDITOR_TABS.fetch_sub(1, Ordering::Relaxed);
}

/// Releases the view gauge together with every tab it still owned.
///
/// Used by `RemoteFileEditorWindow::drop`: tabs that are still open when the
/// window goes away never pass through the per-tab close path, so their gauge
/// has to be returned here or the counters would drift upward forever.
///
/// Returns the same value as [`record_editor_view_dropped`].
pub(crate) fn record_editor_view_released_with_tabs(remaining_tabs: usize) -> bool {
    for _ in 0..remaining_tabs {
        record_editor_tab_dropped();
    }
    record_editor_view_dropped()
}

/// Records that a finished background task had nowhere to deliver its result.
///
/// This is the explicit cancellation point of the weak-reference design: the
/// remote work completed, but its window is gone, so the result is dropped
/// instead of reviving the view. Seeing this line means the work drained
/// cleanly — its absence, with a rising task counter, means it did not.
pub(crate) fn log_task_result_discarded(kind: EditorTaskKind, tab_id: u64) {
    tracing::debug!(
        target: "remote_file_editor::lifecycle",
        stage = "editor_task_result_discarded",
        task = kind.as_str(),
        tab_id,
        "dropping a finished remote file editor task result because its window is gone"
    );
}

/// RAII guard for one in-flight background editor task.
///
/// Holding the guard inside the detached task future is what ties the counter
/// to the task's real lifetime; if the task is dropped (e.g. the window went
/// away) the counter still returns to its baseline.
#[must_use = "dropping the guard immediately makes the task uncounted"]
pub(crate) struct EditorTaskGuard {
    kind: EditorTaskKind,
    started_at: std::time::Instant,
}

impl EditorTaskGuard {
    pub(crate) fn begin(kind: EditorTaskKind, tab_id: u64, detail: EditorTaskDetail) -> Self {
        kind.gauge().fetch_add(1, Ordering::Relaxed);
        tracing::debug!(
            target: "remote_file_editor::lifecycle",
            stage = "editor_task_started",
            task = kind.as_str(),
            tab_id,
            size_bytes = detail.size_bytes,
            language = detail.language,
            "remote file editor task started"
        );
        Self {
            kind,
            started_at: std::time::Instant::now(),
        }
    }
}

impl Drop for EditorTaskGuard {
    fn drop(&mut self) {
        self.kind.gauge().fetch_sub(1, Ordering::Relaxed);
        tracing::debug!(
            target: "remote_file_editor::lifecycle",
            stage = "editor_task_finished",
            task = self.kind.as_str(),
            elapsed_ms = u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
            "remote file editor task finished"
        );
    }
}

/// Diagnostic detail attached to a task start line.
///
/// Only size and language are ever populated; never file contents, remote paths
/// or credentials.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct EditorTaskDetail<'a> {
    pub(crate) size_bytes: Option<usize>,
    pub(crate) language: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::{
        EditorLifecycleSnapshot, EditorTaskDetail, EditorTaskGuard, EditorTaskKind,
        record_editor_tab_created, record_editor_view_created, record_editor_view_released_with_tabs,
        snapshot,
    };

    /// Serializes the gauge assertions; the gauges are process-wide.
    static GAUGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn task_guard_returns_every_gauge_to_its_baseline() {
        let _lock = GAUGE_LOCK.lock().unwrap();
        let baseline = snapshot();

        {
            let _load = EditorTaskGuard::begin(
                EditorTaskKind::Load,
                7,
                EditorTaskDetail {
                    size_bytes: Some(1024),
                    language: Some("rust"),
                },
            );
            let _save = EditorTaskGuard::begin(EditorTaskKind::Save, 7, EditorTaskDetail::default());
            let _parse = EditorTaskGuard::begin(
                EditorTaskKind::Parse,
                7,
                EditorTaskDetail {
                    language: Some("rust"),
                    ..EditorTaskDetail::default()
                },
            );

            let active = snapshot();
            assert_eq!(baseline.pending_load_tasks + 1, active.pending_load_tasks);
            assert_eq!(baseline.pending_save_tasks + 1, active.pending_save_tasks);
            assert_eq!(baseline.active_parse_tasks + 1, active.active_parse_tasks);
            assert!(active.has_pending_work());
        }

        assert_eq!(baseline, snapshot());
        assert!(!snapshot().has_pending_work());
    }

    #[test]
    fn snapshot_reports_torn_down_only_when_views_tabs_and_tasks_are_gone() {
        let baseline = snapshot();
        let with_view = EditorLifecycleSnapshot {
            live_editor_views: 1,
            live_editor_tabs: 1,
            ..baseline
        };
        assert!(!with_view.is_torn_down());

        let with_task = EditorLifecycleSnapshot {
            pending_save_tasks: 1,
            ..baseline
        };
        assert!(!with_task.is_torn_down());
        assert!(with_task.has_pending_work());

        assert!(EditorLifecycleSnapshot::default().is_torn_down());
    }

    /// Mirrors what happens when the user closes the editor window while it
    /// still holds open tabs and no background work is left.
    #[test]
    fn releasing_a_view_with_open_tabs_returns_every_gauge_to_baseline() {
        let _lock = GAUGE_LOCK.lock().unwrap();
        let baseline = snapshot();

        record_editor_view_created();
        record_editor_tab_created();
        record_editor_tab_created();
        let open = snapshot();
        assert_eq!(baseline.live_editor_views + 1, open.live_editor_views);
        assert_eq!(baseline.live_editor_tabs + 2, open.live_editor_tabs);

        assert!(
            record_editor_view_released_with_tabs(2),
            "a window with no background work left should report a clean teardown"
        );
        assert_eq!(baseline, snapshot());
    }

    /// The exact signal this module exists for: the window is gone but a
    /// detached remote task is still draining.
    #[test]
    fn releasing_a_view_while_a_task_is_in_flight_is_reported_as_unclean() {
        let _lock = GAUGE_LOCK.lock().unwrap();
        let baseline = snapshot();

        let task = EditorTaskGuard::begin(EditorTaskKind::Save, 3, EditorTaskDetail::default());
        record_editor_view_created();
        record_editor_tab_created();

        assert!(
            !record_editor_view_released_with_tabs(1),
            "an in-flight save must be reported as work outliving its own window"
        );

        drop(task);
        assert_eq!(baseline, snapshot());
    }
}
