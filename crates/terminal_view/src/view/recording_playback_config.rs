use gpui::SharedString;
use terminal::recording::RecordingPlayback;

pub struct RecordingPlaybackViewConfig {
    playback: RecordingPlayback,
    display_name: SharedString,
}

impl RecordingPlaybackViewConfig {
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
