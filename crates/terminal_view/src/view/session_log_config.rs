use gpui::SharedString;
use terminal::recording::RecordingPlayback;

/// Input used to build a static, read-only terminal session log view.
///
/// Session logs reuse the recording container parser, but they are fully
/// materialized during construction and expose no playback timeline.
pub struct SessionLogViewConfig {
    playback: RecordingPlayback,
    display_name: SharedString,
}

impl SessionLogViewConfig {
    pub fn new(playback: RecordingPlayback, display_name: impl Into<SharedString>) -> Self {
        Self {
            playback,
            display_name: display_name.into(),
        }
    }

    pub(super) fn into_parts(self) -> (RecordingPlayback, SharedString) {
        (self.playback, self.display_name)
    }
}
