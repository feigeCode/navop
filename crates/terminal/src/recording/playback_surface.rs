use super::{
    RecordingEvent, RecordingEventKind, RecordingPlayback, RecordingPlaybackError,
    RecordingPlaybackTransition,
};
use crate::{GpuiEventProxy, TerminalEvent, TerminalPerformanceMetrics, TerminalSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

/// Counts how a playback batch was interpreted.
///
/// Input and markers deliberately remain display-only metadata. They are
/// counted so the UI can invalidate its timeline without ever feeding their
/// bytes to the terminal parser.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RecordingPlaybackApplySummary {
    pub(crate) output_events: usize,
    pub(crate) resize_events: usize,
    pub(crate) display_only_input_events: usize,
    pub(crate) display_only_marker_events: usize,
}

impl RecordingPlaybackApplySummary {
    fn record(&mut self, kind: &RecordingEventKind) {
        match kind {
            RecordingEventKind::Output(_) => self.output_events += 1,
            RecordingEventKind::Resize(_) => self.resize_events += 1,
            RecordingEventKind::Input(_) => self.display_only_input_events += 1,
            RecordingEventKind::Marker(_) => self.display_only_marker_events += 1,
        }
    }
}

/// Owns the parser and terminal surface used for untrusted recording bytes.
///
/// The parser is intentionally long-lived so event chunk boundaries cannot
/// corrupt split UTF-8 or escape sequences. Its event proxy is fail-closed and
/// has no backend response sink.
pub(crate) struct TerminalPlaybackRuntime {
    timeline: RecordingPlayback,
    processor: Processor<StdSyncHandler>,
    term: Arc<FairMutex<Term<GpuiEventProxy>>>,
    event_proxy: GpuiEventProxy,
    scrollback_lines: usize,
}

impl TerminalPlaybackRuntime {
    pub(crate) fn new(
        timeline: RecordingPlayback,
        scrollback_lines: usize,
        event_tx: UnboundedSender<TerminalEvent>,
        performance_metrics: Arc<TerminalPerformanceMetrics>,
    ) -> Self {
        let event_proxy = GpuiEventProxy::playback_safe(event_tx, performance_metrics);
        let initial_size = Self::initial_size_for(&timeline);
        let term = Arc::new(FairMutex::new(Self::new_term(
            initial_size,
            scrollback_lines,
            event_proxy.clone(),
        )));

        Self {
            timeline,
            processor: Processor::new(),
            term,
            event_proxy,
            scrollback_lines,
        }
    }

    pub(crate) fn timeline(&self) -> &RecordingPlayback {
        &self.timeline
    }

    #[cfg(test)]
    pub(crate) fn timeline_mut(&mut self) -> &mut RecordingPlayback {
        &mut self.timeline
    }

    pub(crate) fn term(&self) -> &Arc<FairMutex<Term<GpuiEventProxy>>> {
        &self.term
    }

    /// Returns only the wakeup de-duplication state needed by Terminal's event
    /// loop. The fail-closed playback proxy itself remains private so callers
    /// cannot attach a backend response sink or broaden its event policy.
    pub(crate) fn wakeup_pending_handle(&self) -> Arc<AtomicBool> {
        self.event_proxy.wakeup_pending_handle()
    }

    #[cfg(test)]
    pub(crate) fn event_proxy(&self) -> &GpuiEventProxy {
        &self.event_proxy
    }

    pub(crate) fn initial_size(&self) -> TerminalSize {
        Self::initial_size_for(&self.timeline)
    }

    pub(crate) fn set_scrollback_lines(&mut self, scrollback_lines: usize) {
        self.scrollback_lines = scrollback_lines;
        self.term.lock().set_options(TermConfig {
            scrolling_history: scrollback_lines,
            ..Default::default()
        });
    }

    pub(crate) fn resume(&mut self) -> RecordingPlaybackTransition {
        let transition = self.timeline.resume();
        self.queue_wakeup_for_transition(transition);
        transition
    }

    pub(crate) fn pause(&mut self) -> RecordingPlaybackTransition {
        let transition = self.timeline.pause();
        self.queue_wakeup_for_transition(transition);
        transition
    }

    pub(crate) fn set_speed(
        &mut self,
        speed: f64,
    ) -> Result<RecordingPlaybackTransition, RecordingPlaybackError> {
        let transition = self.timeline.set_speed(speed)?;
        self.queue_wakeup_for_transition(transition);
        Ok(transition)
    }

    pub(crate) fn advance(&mut self, wall_elapsed: Duration) -> RecordingPlaybackApplySummary {
        let previous_elapsed = self.timeline.elapsed();
        let previous_state = self.timeline.state();
        let Self {
            timeline,
            processor,
            term,
            ..
        } = self;
        let due = timeline.advance(wall_elapsed);
        let summary = Self::apply_events(processor, term, due);

        if !due.is_empty()
            || timeline.elapsed() != previous_elapsed
            || timeline.state() != previous_state
        {
            self.event_proxy.queue_wakeup();
        }
        summary
    }

    /// Rebuilds a fresh playback-only surface from the start of the recording.
    ///
    /// The old parser and grid are discarded before the selected prefix is
    /// applied. No live terminal or backend reference is accepted by this API.
    pub(crate) fn seek(&mut self, target: Duration) -> RecordingPlaybackApplySummary {
        let initial_size = self.initial_size();
        let fresh_term = Self::new_term(
            initial_size,
            self.scrollback_lines,
            self.event_proxy.clone(),
        );
        *self.term.lock() = fresh_term;
        self.processor = Processor::new();

        let Self {
            timeline,
            processor,
            term,
            ..
        } = self;
        let prefix = timeline.seek(target);
        let summary = Self::apply_events(processor, term, prefix);
        self.event_proxy.queue_wakeup();
        summary
    }

    fn queue_wakeup_for_transition(&self, transition: RecordingPlaybackTransition) {
        if transition == RecordingPlaybackTransition::Changed {
            self.event_proxy.queue_wakeup();
        }
    }

    fn initial_size_for(timeline: &RecordingPlayback) -> TerminalSize {
        let header = &timeline.recording().header;
        TerminalSize {
            rows: header.height,
            cols: header.width,
            pixel_width: 0,
            pixel_height: 0,
        }
    }

    fn new_term(
        size: TerminalSize,
        scrollback_lines: usize,
        event_proxy: GpuiEventProxy,
    ) -> Term<GpuiEventProxy> {
        Term::new(
            TermConfig {
                scrolling_history: scrollback_lines,
                ..Default::default()
            },
            &PlaybackDimensions::from(size),
            event_proxy,
        )
    }

    fn apply_events(
        processor: &mut Processor<StdSyncHandler>,
        term: &Arc<FairMutex<Term<GpuiEventProxy>>>,
        events: &[RecordingEvent],
    ) -> RecordingPlaybackApplySummary {
        let mut summary = RecordingPlaybackApplySummary::default();
        let mut term = term.lock();
        for event in events {
            summary.record(&event.kind);
            match &event.kind {
                RecordingEventKind::Output(bytes) => processor.advance(&mut *term, bytes),
                RecordingEventKind::Resize(size) => {
                    term.resize(PlaybackDimensions::from(*size));
                }
                RecordingEventKind::Input(_) | RecordingEventKind::Marker(_) => {
                    // Display-only timeline metadata. Never parse or execute.
                }
            }
        }
        summary
    }
}

struct PlaybackDimensions {
    cols: usize,
    rows: usize,
}

impl From<TerminalSize> for PlaybackDimensions {
    fn from(size: TerminalSize) -> Self {
        Self {
            cols: usize::from(size.cols),
            rows: usize::from(size.rows),
        }
    }
}

impl Dimensions for PlaybackDimensions {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}
