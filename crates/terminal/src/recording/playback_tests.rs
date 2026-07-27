use super::{
    MAX_PLAYBACK_SPEED, MIN_PLAYBACK_SPEED, ParsedRecording, RecordingBackend,
    RecordingCompleteness, RecordingEvent, RecordingEventKind, RecordingHeader,
    RecordingHeaderMetadata, RecordingPlayback, RecordingPlaybackError, RecordingPlaybackLimits,
    RecordingPlaybackSearchKind, RecordingPlaybackState, RecordingPlaybackTransition,
};
use crate::TerminalSize;
use std::time::Duration;

fn seconds(value: u64) -> Duration {
    Duration::from_secs(value)
}

fn event(elapsed: u64, kind: RecordingEventKind) -> RecordingEvent {
    RecordingEvent {
        elapsed: seconds(elapsed),
        kind,
    }
}

fn parsed_recording(
    capture_input: bool,
    events: Vec<RecordingEvent>,
    completeness: RecordingCompleteness,
) -> ParsedRecording {
    ParsedRecording {
        header: RecordingHeader {
            version: 2,
            width: 80,
            height: 24,
            timestamp: 1_700_000_000,
            navop: RecordingHeaderMetadata {
                format_version: 1,
                recording_id: "recording-id".to_string(),
                session_id: "session-id".to_string(),
                backend: RecordingBackend::Ssh,
                application_version: "0.1.0-test".to_string(),
                started_at_unix_ms: 1_700_000_000_123,
                capture_input,
                event_stream: "terminal_parser_input_v1".to_string(),
            },
        },
        events,
        completeness,
    }
}

fn playback(events: Vec<RecordingEvent>) -> RecordingPlayback {
    RecordingPlayback::from_parsed(
        parsed_recording(false, events, RecordingCompleteness::Complete),
        RecordingPlaybackLimits::default(),
    )
    .expect("create playback")
}

#[test]
fn playback_starts_paused_and_advances_due_events_at_bounded_speed() {
    let mut playback = playback(vec![
        event(0, RecordingEventKind::Output(b"zero".to_vec())),
        event(1, RecordingEventKind::Output(b"one".to_vec())),
        event(2, RecordingEventKind::Output(b"two".to_vec())),
        event(3, RecordingEventKind::Output(b"three".to_vec())),
    ]);

    assert_eq!(RecordingPlaybackState::Paused, playback.state());
    assert!(playback.advance(seconds(1)).is_empty());
    assert_eq!(
        RecordingPlaybackTransition::Changed,
        playback.set_speed(2.0).unwrap()
    );
    assert_eq!(RecordingPlaybackTransition::Changed, playback.resume());

    let due = playback.advance(Duration::from_millis(500));
    assert_eq!(
        vec![b"zero".as_slice(), b"one".as_slice()],
        due.iter()
            .map(|event| match &event.kind {
                RecordingEventKind::Output(bytes) => bytes.as_slice(),
                _ => panic!("expected output"),
            })
            .collect::<Vec<_>>()
    );
    assert_eq!(seconds(1), playback.elapsed());
    assert_eq!(RecordingPlaybackState::Playing, playback.state());

    assert_eq!(RecordingPlaybackTransition::Changed, playback.pause());
    assert!(playback.advance(seconds(30)).is_empty());
    assert_eq!(seconds(1), playback.elapsed());
    assert_eq!(RecordingPlaybackTransition::Unchanged, playback.pause());

    playback.resume();
    assert_eq!(2, playback.advance(Duration::MAX).len());
    assert_eq!(seconds(3), playback.elapsed());
    assert_eq!(RecordingPlaybackState::Finished, playback.state());
}

#[test]
fn seek_returns_a_rebuild_prefix_without_replaying_any_backend_operation() {
    let mut playback = playback(vec![
        event(0, RecordingEventKind::Output(b"prompt".to_vec())),
        event(
            1,
            RecordingEventKind::Resize(TerminalSize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            }),
        ),
        event(2, RecordingEventKind::Marker("deploy".to_string())),
        event(3, RecordingEventKind::Output(b"done".to_vec())),
    ]);

    assert_eq!(3, playback.seek(seconds(2)).len());
    assert_eq!(3, playback.event_cursor());
    assert_eq!(RecordingPlaybackState::Paused, playback.state());

    assert_eq!(4, playback.seek(seconds(99)).len());
    assert_eq!(seconds(3), playback.elapsed());
    assert_eq!(RecordingPlaybackState::Finished, playback.state());

    assert_eq!(1, playback.seek(Duration::ZERO).len());
    assert_eq!(RecordingPlaybackState::Paused, playback.state());
    assert_eq!(RecordingPlaybackTransition::Changed, playback.resume());
    assert_eq!(3, playback.seek(seconds(2)).len());
    assert_eq!(RecordingPlaybackState::Playing, playback.state());
}

#[test]
fn playback_speed_rejects_non_finite_and_out_of_range_values() {
    let mut playback = playback(vec![event(1, RecordingEventKind::Output(b"done".to_vec()))]);

    for invalid in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        0.0,
        MIN_PLAYBACK_SPEED / 2.0,
        MAX_PLAYBACK_SPEED * 2.0,
    ] {
        assert!(matches!(
            playback.set_speed(invalid),
            Err(RecordingPlaybackError::InvalidSpeed(_))
        ));
    }
    playback.set_speed(MIN_PLAYBACK_SPEED).unwrap();
    playback.set_speed(MAX_PLAYBACK_SPEED).unwrap();
}

#[test]
fn search_labels_input_and_markers_as_display_only_and_skips_resize() {
    let recording = parsed_recording(
        true,
        vec![
            event(1, RecordingEventKind::Output(b"deploy output".to_vec())),
            event(2, RecordingEventKind::Input(b"deploy input".to_vec())),
            event(3, RecordingEventKind::Marker("deploy marker".to_string())),
            event(
                4,
                RecordingEventKind::Resize(TerminalSize {
                    rows: 30,
                    cols: 100,
                    pixel_width: 0,
                    pixel_height: 0,
                }),
            ),
        ],
        RecordingCompleteness::Complete,
    );
    let playback =
        RecordingPlayback::from_parsed(recording, RecordingPlaybackLimits::default()).unwrap();

    let results = playback.search("deploy", 10).unwrap();
    assert_eq!(3, results.matches.len());
    assert_eq!(
        vec![
            RecordingPlaybackSearchKind::Output,
            RecordingPlaybackSearchKind::InputDisplayOnly,
            RecordingPlaybackSearchKind::MarkerDisplayOnly,
        ],
        results
            .matches
            .iter()
            .map(|result| result.kind)
            .collect::<Vec<_>>()
    );
    assert!(!results.matches_truncated);
    assert!(!results.index_status.truncated);
}

#[test]
fn search_index_query_results_and_snippets_are_independently_bounded() {
    let limits = RecordingPlaybackLimits {
        max_indexed_events: 2,
        max_indexed_text_bytes: 16,
        max_search_query_bytes: 4,
        max_search_results: 1,
        max_search_snippet_bytes: 5,
    };
    let recording = parsed_recording(
        false,
        vec![
            event(1, RecordingEventKind::Output(b"xxhitxx".to_vec())),
            event(2, RecordingEventKind::Output(b"yyhityy".to_vec())),
            event(3, RecordingEventKind::Output(b"zzhitzz".to_vec())),
        ],
        RecordingCompleteness::Complete,
    );
    let playback = RecordingPlayback::from_parsed(recording, limits).unwrap();

    assert_eq!(2, playback.search_index_status().indexed_events);
    assert!(playback.search_index_status().indexed_text_bytes <= 16);
    assert!(playback.search_index_status().truncated);

    let results = playback.search("hit", 99).unwrap();
    assert_eq!(1, results.matches.len());
    assert!(results.matches_truncated);
    assert!(results.matches[0].snippet.len() <= 5);
    assert!(matches!(
        playback.search("12345", 1),
        Err(RecordingPlaybackError::SearchQueryTooLong {
            actual_bytes: 5,
            max_bytes: 4
        })
    ));
    assert!(matches!(
        playback.search("", 1),
        Err(RecordingPlaybackError::EmptySearchQuery)
    ));
}

#[test]
fn partial_recovery_state_remains_visible_to_playback_callers() {
    let completeness = RecordingCompleteness::Partial {
        discarded_bytes: 42,
    };
    let playback = RecordingPlayback::from_parsed(
        parsed_recording(
            false,
            vec![event(1, RecordingEventKind::Output(b"recovered".to_vec()))],
            completeness.clone(),
        ),
        RecordingPlaybackLimits::default(),
    )
    .unwrap();

    assert_eq!(&completeness, playback.completeness());
}

#[test]
fn fabricated_timeline_is_revalidated_before_playback() {
    let backwards = parsed_recording(
        false,
        vec![
            event(2, RecordingEventKind::Output(b"later".to_vec())),
            event(1, RecordingEventKind::Output(b"earlier".to_vec())),
        ],
        RecordingCompleteness::Complete,
    );
    assert!(matches!(
        RecordingPlayback::from_parsed(backwards, RecordingPlaybackLimits::default()),
        Err(RecordingPlaybackError::InvalidTimeline {
            event_index: 1,
            reason: "timestamp moved backwards"
        })
    ));

    let undisclosed_input = parsed_recording(
        false,
        vec![event(1, RecordingEventKind::Input(b"secret".to_vec()))],
        RecordingCompleteness::Complete,
    );
    assert!(matches!(
        RecordingPlayback::from_parsed(undisclosed_input, RecordingPlaybackLimits::default()),
        Err(RecordingPlaybackError::InvalidTimeline {
            event_index: 0,
            reason: "input event is present while capture_input is disabled"
        })
    ));
}
