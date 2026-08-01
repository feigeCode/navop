use super::{
    ParsedRecording, RecordingCompleteness, RecordingEvent, RecordingEventKind, RecordingFileError,
    RecordingFileLimits, read_recording_for_playback,
};
use std::fmt;
use std::path::Path;
use std::time::Duration;

pub const MIN_PLAYBACK_SPEED: f64 = 0.25;
pub const MAX_PLAYBACK_SPEED: f64 = 4.0;
pub const DEFAULT_MAX_PLAYBACK_INDEXED_EVENTS: usize = 100_000;
pub const DEFAULT_MAX_PLAYBACK_INDEXED_TEXT_BYTES: usize = 16 * 1024 * 1024;
pub const DEFAULT_MAX_PLAYBACK_SEARCH_QUERY_BYTES: usize = 4 * 1024;
pub const DEFAULT_MAX_PLAYBACK_SEARCH_RESULTS: usize = 1_000;
pub const DEFAULT_MAX_PLAYBACK_SEARCH_SNIPPET_BYTES: usize = 512;

/// Resource limits for the read-only search surface built over a recording.
///
/// Recording file parsing has its own byte and event limits. These limits
/// independently bound the extra memory retained by playback search.
#[derive(Clone, Debug)]
pub struct RecordingPlaybackLimits {
    pub max_indexed_events: usize,
    pub max_indexed_text_bytes: usize,
    pub max_search_query_bytes: usize,
    pub max_search_results: usize,
    pub max_search_snippet_bytes: usize,
}

impl Default for RecordingPlaybackLimits {
    fn default() -> Self {
        Self {
            max_indexed_events: DEFAULT_MAX_PLAYBACK_INDEXED_EVENTS,
            max_indexed_text_bytes: DEFAULT_MAX_PLAYBACK_INDEXED_TEXT_BYTES,
            max_search_query_bytes: DEFAULT_MAX_PLAYBACK_SEARCH_QUERY_BYTES,
            max_search_results: DEFAULT_MAX_PLAYBACK_SEARCH_RESULTS,
            max_search_snippet_bytes: DEFAULT_MAX_PLAYBACK_SEARCH_SNIPPET_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingPlaybackState {
    Playing,
    Paused,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingPlaybackTransition {
    Changed,
    Unchanged,
}

/// Search categories deliberately distinguish non-output events as display
/// only. Playback surfaces must never feed input or marker data to a terminal
/// parser or backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingPlaybackSearchKind {
    Output,
    InputDisplayOnly,
    MarkerDisplayOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingPlaybackSearchMatch {
    pub event_index: usize,
    pub elapsed: Duration,
    pub kind: RecordingPlaybackSearchKind,
    pub match_byte_offset: usize,
    pub snippet: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordingPlaybackSearchIndexStatus {
    pub indexed_events: usize,
    pub indexed_text_bytes: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingPlaybackSearchResults {
    pub matches: Vec<RecordingPlaybackSearchMatch>,
    pub matches_truncated: bool,
    pub index_status: RecordingPlaybackSearchIndexStatus,
}

#[derive(Debug)]
pub enum RecordingPlaybackError {
    RecordingFile(RecordingFileError),
    NotPlaybackSession,
    InvalidLimit(&'static str),
    InvalidTimeline {
        event_index: usize,
        reason: &'static str,
    },
    InvalidSpeed(f64),
    EmptySearchQuery,
    SearchQueryTooLong {
        actual_bytes: usize,
        max_bytes: usize,
    },
}

impl fmt::Display for RecordingPlaybackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecordingFile(error) => write!(formatter, "{error}"),
            Self::NotPlaybackSession => {
                formatter.write_str("terminal is not a recording playback session")
            }
            Self::InvalidLimit(name) => {
                write!(
                    formatter,
                    "recording playback limit must be non-zero: {name}"
                )
            }
            Self::InvalidTimeline {
                event_index,
                reason,
            } => write!(
                formatter,
                "invalid recording playback event at index {event_index}: {reason}"
            ),
            Self::InvalidSpeed(speed) => write!(
                formatter,
                "recording playback speed must be finite and between \
                 {MIN_PLAYBACK_SPEED} and {MAX_PLAYBACK_SPEED}: {speed}"
            ),
            Self::EmptySearchQuery => {
                formatter.write_str("recording playback search query must not be empty")
            }
            Self::SearchQueryTooLong {
                actual_bytes,
                max_bytes,
            } => write!(
                formatter,
                "recording playback search query is too long: \
                 {actual_bytes} bytes exceeds {max_bytes} bytes"
            ),
        }
    }
}

impl std::error::Error for RecordingPlaybackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::RecordingFile(error) => Some(error),
            _ => None,
        }
    }
}

impl From<RecordingFileError> for RecordingPlaybackError {
    fn from(error: RecordingFileError) -> Self {
        Self::RecordingFile(error)
    }
}

#[derive(Debug)]
struct SearchEntry {
    event_index: usize,
    elapsed: Duration,
    kind: RecordingPlaybackSearchKind,
    text: String,
}

/// A bounded, read-only timeline over a parsed terminal recording.
///
/// This type intentionally has no terminal backend, input handle, parser, or
/// connection metadata. It only decides which immutable events are due. A
/// playback renderer must apply `Output` and `Resize` itself while keeping
/// `Input` and `Marker` events on a display-only metadata surface.
#[derive(Debug)]
pub struct RecordingPlayback {
    recording: ParsedRecording,
    state: RecordingPlaybackState,
    elapsed: Duration,
    duration: Duration,
    cursor: usize,
    speed: f64,
    limits: RecordingPlaybackLimits,
    search_entries: Vec<SearchEntry>,
    search_index_status: RecordingPlaybackSearchIndexStatus,
}

impl RecordingPlayback {
    pub fn open(
        path: impl AsRef<Path>,
        file_limits: RecordingFileLimits,
        playback_limits: RecordingPlaybackLimits,
    ) -> Result<Self, RecordingPlaybackError> {
        let recording = read_recording_for_playback(path, file_limits)?;
        Self::from_parsed(recording, playback_limits)
    }

    pub fn from_parsed(
        recording: ParsedRecording,
        limits: RecordingPlaybackLimits,
    ) -> Result<Self, RecordingPlaybackError> {
        validate_limits(&limits)?;
        recording.header.validate()?;
        validate_timeline(&recording)?;

        let duration = recording
            .events
            .last()
            .map_or(Duration::ZERO, |event| event.elapsed);
        let (search_entries, search_index_status) = build_search_index(&recording, &limits);

        Ok(Self {
            recording,
            state: RecordingPlaybackState::Paused,
            elapsed: Duration::ZERO,
            duration,
            cursor: 0,
            speed: 1.0,
            limits,
            search_entries,
            search_index_status,
        })
    }

    pub fn recording(&self) -> &ParsedRecording {
        &self.recording
    }

    pub fn completeness(&self) -> &RecordingCompleteness {
        &self.recording.completeness
    }

    pub fn state(&self) -> RecordingPlaybackState {
        self.state
    }

    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn event_cursor(&self) -> usize {
        self.cursor
    }

    pub fn search_index_status(&self) -> RecordingPlaybackSearchIndexStatus {
        self.search_index_status
    }

    pub fn resume(&mut self) -> RecordingPlaybackTransition {
        let next_state = if self.cursor >= self.recording.events.len() {
            RecordingPlaybackState::Finished
        } else {
            RecordingPlaybackState::Playing
        };
        self.replace_state(next_state)
    }

    pub fn pause(&mut self) -> RecordingPlaybackTransition {
        if self.state == RecordingPlaybackState::Playing {
            self.replace_state(RecordingPlaybackState::Paused)
        } else {
            RecordingPlaybackTransition::Unchanged
        }
    }

    pub fn set_speed(
        &mut self,
        speed: f64,
    ) -> Result<RecordingPlaybackTransition, RecordingPlaybackError> {
        if !speed.is_finite() || !(MIN_PLAYBACK_SPEED..=MAX_PLAYBACK_SPEED).contains(&speed) {
            return Err(RecordingPlaybackError::InvalidSpeed(speed));
        }
        if self.speed == speed {
            return Ok(RecordingPlaybackTransition::Unchanged);
        }
        self.speed = speed;
        Ok(RecordingPlaybackTransition::Changed)
    }

    /// Advances playback by wall-clock time and returns the newly due events.
    ///
    /// The returned slice borrows this playback instance so callers naturally
    /// finish consuming one bounded batch before advancing the cursor again.
    pub fn advance(&mut self, wall_elapsed: Duration) -> &[RecordingEvent] {
        if self.state != RecordingPlaybackState::Playing {
            return &self.recording.events[self.cursor..self.cursor];
        }

        let start = self.cursor;
        let remaining = self.duration.saturating_sub(self.elapsed);
        let timeline_elapsed = scaled_duration_bounded(wall_elapsed, self.speed, remaining);
        self.elapsed = self
            .elapsed
            .checked_add(timeline_elapsed)
            .unwrap_or(self.duration)
            .min(self.duration);
        self.cursor = self
            .recording
            .events
            .partition_point(|event| event.elapsed <= self.elapsed);
        if self.cursor >= self.recording.events.len() && self.elapsed >= self.duration {
            self.state = RecordingPlaybackState::Finished;
        }

        &self.recording.events[start..self.cursor]
    }

    /// Moves to a timeline position and returns every event needed to rebuild
    /// a fresh playback terminal up to that point.
    ///
    /// The target is clamped to the recording duration. Seeking never applies
    /// events itself and never writes to a live backend.
    pub fn seek(&mut self, target: Duration) -> &[RecordingEvent] {
        let was_playing = self.state == RecordingPlaybackState::Playing;
        self.elapsed = target.min(self.duration);
        self.cursor = self
            .recording
            .events
            .partition_point(|event| event.elapsed <= self.elapsed);
        self.state = if self.cursor >= self.recording.events.len() && self.elapsed >= self.duration
        {
            RecordingPlaybackState::Finished
        } else if was_playing {
            RecordingPlaybackState::Playing
        } else {
            RecordingPlaybackState::Paused
        };

        &self.recording.events[..self.cursor]
    }

    pub fn search(
        &self,
        query: &str,
        requested_results: usize,
    ) -> Result<RecordingPlaybackSearchResults, RecordingPlaybackError> {
        if query.is_empty() {
            return Err(RecordingPlaybackError::EmptySearchQuery);
        }
        if query.len() > self.limits.max_search_query_bytes {
            return Err(RecordingPlaybackError::SearchQueryTooLong {
                actual_bytes: query.len(),
                max_bytes: self.limits.max_search_query_bytes,
            });
        }

        let result_limit = requested_results.min(self.limits.max_search_results);
        let mut matches = Vec::with_capacity(result_limit.min(self.search_entries.len()));
        let mut matches_truncated = false;
        for entry in &self.search_entries {
            let Some(match_byte_offset) = entry.text.find(query) else {
                continue;
            };
            if matches.len() >= result_limit {
                matches_truncated = true;
                break;
            }
            matches.push(RecordingPlaybackSearchMatch {
                event_index: entry.event_index,
                elapsed: entry.elapsed,
                kind: entry.kind,
                match_byte_offset,
                snippet: search_snippet(
                    &entry.text,
                    match_byte_offset,
                    query.len(),
                    self.limits.max_search_snippet_bytes,
                ),
            });
        }

        Ok(RecordingPlaybackSearchResults {
            matches,
            matches_truncated,
            index_status: self.search_index_status,
        })
    }

    fn replace_state(&mut self, next_state: RecordingPlaybackState) -> RecordingPlaybackTransition {
        if self.state == next_state {
            RecordingPlaybackTransition::Unchanged
        } else {
            self.state = next_state;
            RecordingPlaybackTransition::Changed
        }
    }
}

fn validate_limits(limits: &RecordingPlaybackLimits) -> Result<(), RecordingPlaybackError> {
    for (name, value) in [
        ("max_indexed_events", limits.max_indexed_events),
        ("max_indexed_text_bytes", limits.max_indexed_text_bytes),
        ("max_search_query_bytes", limits.max_search_query_bytes),
        ("max_search_results", limits.max_search_results),
        ("max_search_snippet_bytes", limits.max_search_snippet_bytes),
    ] {
        if value == 0 {
            return Err(RecordingPlaybackError::InvalidLimit(name));
        }
    }
    Ok(())
}

fn validate_timeline(recording: &ParsedRecording) -> Result<(), RecordingPlaybackError> {
    let mut previous_elapsed = None;
    for (event_index, event) in recording.events.iter().enumerate() {
        if previous_elapsed.is_some_and(|elapsed| event.elapsed < elapsed) {
            return Err(RecordingPlaybackError::InvalidTimeline {
                event_index,
                reason: "timestamp moved backwards",
            });
        }
        match &event.kind {
            RecordingEventKind::Input(_) if !recording.header.navop.capture_input => {
                return Err(RecordingPlaybackError::InvalidTimeline {
                    event_index,
                    reason: "input event is present while capture_input is disabled",
                });
            }
            RecordingEventKind::Resize(size) if size.cols == 0 || size.rows == 0 => {
                return Err(RecordingPlaybackError::InvalidTimeline {
                    event_index,
                    reason: "resize dimensions must be non-zero",
                });
            }
            _ => {}
        }
        previous_elapsed = Some(event.elapsed);
    }
    Ok(())
}

fn build_search_index(
    recording: &ParsedRecording,
    limits: &RecordingPlaybackLimits,
) -> (Vec<SearchEntry>, RecordingPlaybackSearchIndexStatus) {
    let mut entries = Vec::new();
    let mut indexed_text_bytes = 0;
    let mut truncated = false;

    for (event_index, event) in recording.events.iter().enumerate() {
        let (kind, source) = match &event.kind {
            RecordingEventKind::Output(bytes) => {
                (RecordingPlaybackSearchKind::Output, bytes.as_slice())
            }
            RecordingEventKind::Input(bytes) => (
                RecordingPlaybackSearchKind::InputDisplayOnly,
                bytes.as_slice(),
            ),
            RecordingEventKind::Marker(marker) => (
                RecordingPlaybackSearchKind::MarkerDisplayOnly,
                marker.as_bytes(),
            ),
            RecordingEventKind::Resize(_) => continue,
        };

        if entries.len() >= limits.max_indexed_events {
            truncated = true;
            break;
        }
        let remaining = limits
            .max_indexed_text_bytes
            .saturating_sub(indexed_text_bytes);
        if remaining == 0 {
            truncated = true;
            break;
        }

        let (text, source_truncated) = bounded_lossy_text(source, remaining);
        indexed_text_bytes = indexed_text_bytes.saturating_add(text.len());
        entries.push(SearchEntry {
            event_index,
            elapsed: event.elapsed,
            kind,
            text,
        });
        if source_truncated {
            truncated = true;
            break;
        }
    }

    let status = RecordingPlaybackSearchIndexStatus {
        indexed_events: entries.len(),
        indexed_text_bytes,
        truncated,
    };
    (entries, status)
}

fn bounded_lossy_text(source: &[u8], max_bytes: usize) -> (String, bool) {
    let source_bytes = source.len().min(max_bytes);
    let mut text = String::from_utf8_lossy(&source[..source_bytes]).into_owned();
    let mut truncated = source_bytes < source.len();
    if text.len() > max_bytes {
        let end = floor_char_boundary(&text, max_bytes);
        text.truncate(end);
        truncated = true;
    }
    (text, truncated)
}

fn search_snippet(text: &str, match_start: usize, match_len: usize, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }

    let match_end = match_start.saturating_add(match_len).min(text.len());
    let context_before = max_bytes / 3;
    let mut start = match_start.saturating_sub(context_before);
    start = floor_char_boundary(text, start);

    let minimum_end = match_end.min(start.saturating_add(max_bytes));
    let mut end = start.saturating_add(max_bytes).min(text.len());
    end = floor_char_boundary(text, end);
    if end < minimum_end {
        end = floor_char_boundary(text, minimum_end);
    }
    if end <= start {
        return String::new();
    }
    text[start..end].to_string()
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn scaled_duration_bounded(wall_elapsed: Duration, speed: f64, remaining: Duration) -> Duration {
    if wall_elapsed.is_zero() || remaining.is_zero() {
        return Duration::ZERO;
    }

    let scaled_seconds = wall_elapsed.as_secs_f64() * speed;
    if !scaled_seconds.is_finite() || scaled_seconds >= remaining.as_secs_f64() {
        return remaining;
    }
    Duration::try_from_secs_f64(scaled_seconds)
        .unwrap_or(remaining)
        .min(remaining)
}
