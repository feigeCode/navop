use super::{
    RecordingCompleteness, RecordingFileLimits, RecordingPlayback, RecordingPlaybackError,
    RecordingPlaybackLimits, TerminalPlaybackRuntime,
};
use crate::TerminalPerformanceMetrics;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Line;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingTextExport {
    pub text: String,
    pub completeness: RecordingCompleteness,
    pub history_size: usize,
    pub screen_lines: usize,
    pub columns: usize,
}

/// Renders a recording through the real terminal parser and exports the text
/// still retained in scrollback plus the current screen.
///
/// This is intentionally a derived representation. It does not replace the
/// `.cast` source because timing, resize, binary bytes and playback metadata
/// cannot be represented faithfully in a text file.
pub fn export_recording_text(
    path: impl AsRef<Path>,
    file_limits: RecordingFileLimits,
    scrollback_lines: usize,
) -> Result<RecordingTextExport, RecordingPlaybackError> {
    let playback = RecordingPlayback::open(path, file_limits, RecordingPlaybackLimits::default())?;
    let completeness = playback.completeness().clone();
    let (event_tx, _event_rx) = unbounded_channel();
    let metrics = Arc::new(TerminalPerformanceMetrics::default());
    let mut runtime = TerminalPlaybackRuntime::new(playback, scrollback_lines, event_tx, metrics);

    let duration = runtime.timeline().duration();
    runtime.resume();
    runtime.advance(duration);

    let term = runtime.term().lock();
    let history_size = term.history_size();
    let screen_lines = term.screen_lines();
    let columns = term.columns();
    let mut lines = retained_lines(&term, history_size, screen_lines);
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }

    Ok(RecordingTextExport {
        text: lines.join("\n"),
        completeness,
        history_size,
        screen_lines,
        columns,
    })
}

fn retained_lines(
    term: &alacritty_terminal::term::Term<crate::GpuiEventProxy>,
    history_size: usize,
    screen_lines: usize,
) -> Vec<String> {
    let grid = term.grid();
    let (top, bottom) = retained_line_bounds(history_size, screen_lines);
    (top..=bottom)
        .map(|line| {
            let text: String = grid[Line(line)][..].iter().map(|cell| cell.c).collect();
            text.trim_end_matches(|ch: char| ch == ' ' || ch == '\0')
                .to_string()
        })
        .collect()
}

pub(super) fn retained_line_bounds(history_size: usize, screen_lines: usize) -> (i32, i32) {
    let retained_history = history_size.min(i32::MAX as usize) as i32;
    let retained_screen_lines = screen_lines.min(i32::MAX as usize) as i32;
    (-retained_history, retained_screen_lines.saturating_sub(1))
}
