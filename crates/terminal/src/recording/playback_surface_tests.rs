use super::{
    ASCIICAST_VERSION, NAVOP_EVENT_STREAM, NAVOP_RECORDING_FORMAT_VERSION, ParsedRecording,
    RecordingBackend, RecordingCompleteness, RecordingEvent, RecordingEventKind, RecordingHeader,
    RecordingHeaderMetadata, RecordingPlayback, RecordingPlaybackLimits, TerminalPlaybackRuntime,
};
use crate::{GpuiEventProxy, TerminalEvent, TerminalPerformanceMetrics, TerminalSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

fn milliseconds(value: u64) -> Duration {
    Duration::from_millis(value)
}

fn event(elapsed_ms: u64, kind: RecordingEventKind) -> RecordingEvent {
    RecordingEvent {
        elapsed: milliseconds(elapsed_ms),
        kind,
    }
}

fn parsed_recording(capture_input: bool, events: Vec<RecordingEvent>) -> ParsedRecording {
    ParsedRecording {
        header: RecordingHeader {
            version: ASCIICAST_VERSION,
            width: 80,
            height: 24,
            timestamp: 1_700_000_000,
            navop: RecordingHeaderMetadata {
                format_version: NAVOP_RECORDING_FORMAT_VERSION,
                recording_id: "playback-surface-recording".to_string(),
                session_id: "playback-surface-session".to_string(),
                backend: RecordingBackend::Ssh,
                application_version: "0.1.0-test".to_string(),
                started_at_unix_ms: 1_700_000_000_123,
                capture_input,
                event_stream: NAVOP_EVENT_STREAM.to_string(),
            },
        },
        events,
        completeness: RecordingCompleteness::Complete,
    }
}

fn playback(capture_input: bool, events: Vec<RecordingEvent>) -> RecordingPlayback {
    RecordingPlayback::from_parsed(
        parsed_recording(capture_input, events),
        RecordingPlaybackLimits::default(),
    )
    .expect("create recording playback")
}

fn runtime(
    capture_input: bool,
    events: Vec<RecordingEvent>,
) -> (
    TerminalPlaybackRuntime,
    UnboundedReceiver<TerminalEvent>,
    Arc<TerminalPerformanceMetrics>,
) {
    let (event_tx, event_rx) = unbounded_channel();
    let metrics = Arc::new(TerminalPerformanceMetrics::default());
    let runtime = TerminalPlaybackRuntime::new(
        playback(capture_input, events),
        1_000,
        event_tx,
        metrics.clone(),
    );
    (runtime, event_rx, metrics)
}

fn assert_grid_prefix(term: &Arc<FairMutex<Term<GpuiEventProxy>>>, expected: &str) {
    let term = term.lock();
    for (column, expected) in expected.chars().enumerate() {
        assert_eq!(
            expected,
            term.grid()[Line(0)][Column(column)].c,
            "unexpected cell at column {column}"
        );
    }
}

#[test]
fn parser_state_survives_recording_chunk_boundaries() {
    let utf8 = "界".as_bytes();
    let (mut runtime, _event_rx, _metrics) = runtime(
        false,
        vec![
            event(0, RecordingEventKind::Output(utf8[..1].to_vec())),
            event(1, RecordingEventKind::Output(utf8[1..].to_vec())),
            event(2, RecordingEventKind::Output(b"\x1b[31".to_vec())),
            event(3, RecordingEventKind::Output(b"mR".to_vec())),
        ],
    );

    runtime.timeline_mut().resume();
    assert_eq!(1, runtime.advance(Duration::ZERO).output_events);
    assert_eq!(1, runtime.advance(milliseconds(1)).output_events);
    assert_eq!('界', runtime.term().lock().grid()[Line(0)][Column(0)].c);

    assert_eq!(1, runtime.advance(milliseconds(1)).output_events);
    assert_eq!(1, runtime.advance(milliseconds(1)).output_events);
    let term = runtime.term().lock();
    assert_eq!('R', term.grid()[Line(0)][Column(2)].c);
    assert_ne!(
        term.grid()[Line(0)][Column(2)].fg,
        term.grid()[Line(0)][Column(0)].fg,
        "split SGR sequence should retain parser state and color the next cell"
    );
}

#[test]
fn malicious_terminal_sequences_cannot_escape_the_playback_surface() {
    let payload = concat!(
        "\x1b[c",
        "\x1b]52;c;c2VjcmV0\x07",
        "\x1b]0;recorded title\x07",
        "\x07",
        "\x1b]8;;https://example.com\x1b\\",
        "link",
        "\x1b]8;;\x1b\\"
    );
    let (mut runtime, mut event_rx, metrics) = runtime(
        false,
        vec![event(
            0,
            RecordingEventKind::Output(payload.as_bytes().to_vec()),
        )],
    );
    let (write_tx, mut write_rx) = unbounded_channel();

    // Even a malicious caller inside the crate cannot attach a response sink
    // after the proxy has been created with the playback-safe policy.
    runtime.event_proxy().set_ssh_write_back(write_tx);
    runtime.timeline_mut().resume();
    assert_eq!(1, runtime.advance(Duration::ZERO).output_events);

    assert_grid_prefix(runtime.term(), "link");
    assert!(
        write_rx.try_recv().is_err(),
        "playback parser must not write query responses to a backend"
    );

    let mut wakeups = 0;
    while let Ok(event) = event_rx.try_recv() {
        match event {
            TerminalEvent::Wakeup => wakeups += 1,
            TerminalEvent::SshMfaChanged
            | TerminalEvent::PromptStart
            | TerminalEvent::InputStart
            | TerminalEvent::CommandStart
            | TerminalEvent::TitleChanged(_)
            | TerminalEvent::Bell
            | TerminalEvent::ChildExit(_)
            | TerminalEvent::ClipboardStore(_, _)
            | TerminalEvent::ClipboardLoad(_)
            | TerminalEvent::WorkingDirChanged(_)
            | TerminalEvent::CommandFinished { .. }
            | TerminalEvent::CommandRecorded(_) => {
                panic!("playback-safe parser leaked a non-render event")
            }
        }
    }
    assert_eq!(1, wakeups, "wakeup requests should remain coalesced");
    assert_eq!(0, metrics.snapshot().terminal_response_bytes);
}

#[test]
fn input_and_markers_remain_display_only_metadata() {
    let (mut runtime, _event_rx, _metrics) = runtime(
        true,
        vec![
            event(0, RecordingEventKind::Output(b"safe".to_vec())),
            event(0, RecordingEventKind::Input(b"INJECTED".to_vec())),
            event(0, RecordingEventKind::Marker("\x1b[31mMARKER".to_string())),
        ],
    );

    runtime.timeline_mut().resume();
    let summary = runtime.advance(Duration::ZERO);

    assert_eq!(1, summary.output_events);
    assert_eq!(0, summary.resize_events);
    assert_eq!(1, summary.display_only_input_events);
    assert_eq!(1, summary.display_only_marker_events);
    assert_grid_prefix(runtime.term(), "safe");
    assert_eq!(
        ' ',
        runtime.term().lock().grid()[Line(0)][Column(4)].c,
        "display-only bytes must never enter the terminal parser"
    );
}

#[test]
fn resize_events_only_resize_the_playback_grid() {
    let (mut runtime, _event_rx, _metrics) = runtime(
        false,
        vec![event(
            0,
            RecordingEventKind::Resize(TerminalSize {
                cols: 100,
                rows: 40,
                pixel_width: 0,
                pixel_height: 0,
            }),
        )],
    );

    runtime.timeline_mut().resume();
    let summary = runtime.advance(Duration::ZERO);
    let term = runtime.term().lock();

    assert_eq!(1, summary.resize_events);
    assert_eq!(100, term.columns());
    assert_eq!(40, term.screen_lines());
}

#[test]
fn seek_discards_the_old_parser_and_grid_without_touching_a_live_term() {
    let (mut runtime, _event_rx, _metrics) = runtime(
        false,
        vec![
            event(0, RecordingEventKind::Output(b"one".to_vec())),
            event(1, RecordingEventKind::Output(b"two".to_vec())),
            event(2, RecordingEventKind::Output(b"three".to_vec())),
        ],
    );
    let (live_event_tx, _live_event_rx) = unbounded_channel();
    let live_proxy = GpuiEventProxy::new(live_event_tx);
    let live_term = Arc::new(FairMutex::new(Term::new(
        TermConfig::default(),
        &TestDimensions,
        live_proxy,
    )));
    let mut live_parser = Processor::<StdSyncHandler>::new();
    live_parser.advance(&mut *live_term.lock(), b"LIVE");

    runtime.timeline_mut().resume();
    assert_eq!(3, runtime.advance(milliseconds(2)).output_events);
    assert_grid_prefix(runtime.term(), "onetwothree");

    let summary = runtime.seek(Duration::ZERO);

    assert_eq!(1, summary.output_events);
    assert_grid_prefix(runtime.term(), "one");
    assert_eq!(
        ' ',
        runtime.term().lock().grid()[Line(0)][Column(3)].c,
        "fresh playback surface must not retain output after the seek target"
    );
    assert_grid_prefix(&live_term, "LIVE");
    assert!(
        !Arc::ptr_eq(runtime.term(), &live_term),
        "playback runtime must own a surface separate from every live term"
    );
}

struct TestDimensions;

impl Dimensions for TestDimensions {
    fn total_lines(&self) -> usize {
        24
    }

    fn screen_lines(&self) -> usize {
        24
    }

    fn columns(&self) -> usize {
        80
    }
}
