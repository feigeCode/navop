use std::time::Duration;
use terminal::recording::{
    RecordingCompleteness, RecordingPlaybackSearchIndexStatus, RecordingPlaybackState,
};

pub(super) const PLAYBACK_SPEED_PRESETS: [f64; 5] = [0.25, 0.5, 1.0, 2.0, 4.0];
const PLAYBACK_SPEED_SELECTION_EPSILON: f64 = 1.0e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecordingPlaybackFooterStatus {
    Playing,
    Paused,
    Finished,
}

impl From<RecordingPlaybackState> for RecordingPlaybackFooterStatus {
    fn from(state: RecordingPlaybackState) -> Self {
        match state {
            RecordingPlaybackState::Playing => Self::Playing,
            RecordingPlaybackState::Paused => Self::Paused,
            RecordingPlaybackState::Finished => Self::Finished,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RecordingPlaybackPartialRecoveryNotice {
    pub(super) discarded_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RecordingPlaybackSearchIndexNotice {
    pub(super) indexed_events: usize,
    pub(super) indexed_text_bytes: usize,
}

pub(super) fn playback_progress(elapsed: Duration, duration: Duration) -> f32 {
    if duration.is_zero() {
        return 0.0;
    }

    (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0) as f32
}

pub(super) fn playback_seek_target(progress: f32, duration: Duration) -> Duration {
    if !progress.is_finite() || progress <= 0.0 || duration.is_zero() {
        return Duration::ZERO;
    }
    if progress >= 1.0 {
        return duration;
    }

    duration.mul_f64(f64::from(progress))
}

pub(super) fn format_playback_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds / 60) % 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

pub(super) fn format_playback_position(elapsed: Duration, duration: Duration) -> String {
    let elapsed = elapsed.min(duration);
    format!(
        "{} / {}",
        format_playback_duration(elapsed),
        format_playback_duration(duration)
    )
}

pub(super) fn playback_partial_recovery_notice(
    completeness: &RecordingCompleteness,
) -> Option<RecordingPlaybackPartialRecoveryNotice> {
    match completeness {
        RecordingCompleteness::Complete => None,
        RecordingCompleteness::Partial { discarded_bytes } => {
            Some(RecordingPlaybackPartialRecoveryNotice {
                discarded_bytes: *discarded_bytes,
            })
        }
    }
}

pub(super) fn playback_search_index_notice(
    status: RecordingPlaybackSearchIndexStatus,
) -> Option<RecordingPlaybackSearchIndexNotice> {
    status
        .truncated
        .then_some(RecordingPlaybackSearchIndexNotice {
            indexed_events: status.indexed_events,
            indexed_text_bytes: status.indexed_text_bytes,
        })
}

pub(super) fn playback_speed_is_selected(current: f64, preset: f64) -> bool {
    current.is_finite()
        && preset.is_finite()
        && (current - preset).abs() <= PLAYBACK_SPEED_SELECTION_EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use terminal::recording::{
        RecordingCompleteness, RecordingPlaybackSearchIndexStatus, RecordingPlaybackState,
    };

    #[test]
    fn playback_progress_handles_zero_duration_and_clamps_elapsed() {
        assert_eq!(0.0, playback_progress(Duration::ZERO, Duration::ZERO));
        assert_eq!(
            0.5,
            playback_progress(Duration::from_secs(5), Duration::from_secs(10))
        );
        assert_eq!(
            1.0,
            playback_progress(Duration::from_secs(11), Duration::from_secs(10))
        );
    }

    #[test]
    fn playback_seek_target_sanitizes_slider_progress() {
        let duration = Duration::from_secs(10);

        assert_eq!(Duration::ZERO, playback_seek_target(f32::NAN, duration));
        assert_eq!(Duration::ZERO, playback_seek_target(-1.0, duration));
        assert_eq!(
            Duration::from_millis(2_500),
            playback_seek_target(0.25, duration)
        );
        assert_eq!(duration, playback_seek_target(2.0, duration));
    }

    #[test]
    fn playback_seek_target_handles_maximum_duration_without_overflow() {
        assert_eq!(Duration::MAX, playback_seek_target(1.0, Duration::MAX));
        let midpoint = playback_seek_target(0.5, Duration::MAX);
        assert!(midpoint > Duration::ZERO);
        assert!(midpoint < Duration::MAX);
    }

    #[test]
    fn playback_position_does_not_wrap_long_hours() {
        let duration = Duration::from_secs(123 * 3_600 + 4 * 60 + 5);

        assert_eq!("123:04:05", format_playback_duration(duration));
        assert_eq!(
            "01:02:03 / 123:04:05",
            format_playback_position(Duration::from_secs(3_600 + 2 * 60 + 3), duration)
        );
    }

    #[test]
    fn playback_partial_recovery_notice_preserves_discarded_bytes() {
        assert_eq!(
            None,
            playback_partial_recovery_notice(&RecordingCompleteness::Complete)
        );
        assert_eq!(
            Some(RecordingPlaybackPartialRecoveryNotice {
                discarded_bytes: 4_096,
            }),
            playback_partial_recovery_notice(&RecordingCompleteness::Partial {
                discarded_bytes: 4_096,
            })
        );
    }

    #[test]
    fn playback_index_notice_is_only_present_when_truncated() {
        let complete = RecordingPlaybackSearchIndexStatus {
            indexed_events: 20,
            indexed_text_bytes: 2_048,
            truncated: false,
        };
        let truncated = RecordingPlaybackSearchIndexStatus {
            indexed_events: 100,
            indexed_text_bytes: 16_384,
            truncated: true,
        };

        assert_eq!(None, playback_search_index_notice(complete));
        assert_eq!(
            Some(RecordingPlaybackSearchIndexNotice {
                indexed_events: 100,
                indexed_text_bytes: 16_384,
            }),
            playback_search_index_notice(truncated)
        );
    }

    #[test]
    fn playback_footer_status_follows_timeline_state() {
        assert_eq!(
            RecordingPlaybackFooterStatus::Playing,
            RecordingPlaybackFooterStatus::from(RecordingPlaybackState::Playing)
        );
        assert_eq!(
            RecordingPlaybackFooterStatus::Paused,
            RecordingPlaybackFooterStatus::from(RecordingPlaybackState::Paused)
        );
        assert_eq!(
            RecordingPlaybackFooterStatus::Finished,
            RecordingPlaybackFooterStatus::from(RecordingPlaybackState::Finished)
        );
    }

    #[test]
    fn playback_speed_presets_are_bounded_and_selection_is_tolerant() {
        assert_eq!([0.25, 0.5, 1.0, 2.0, 4.0], PLAYBACK_SPEED_PRESETS);
        assert!(playback_speed_is_selected(1.0 + 1.0e-10, 1.0));
        assert!(!playback_speed_is_selected(1.01, 1.0));
        assert!(!playback_speed_is_selected(f64::NAN, 1.0));
    }
}
