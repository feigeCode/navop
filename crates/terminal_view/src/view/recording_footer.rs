use super::*;
use chrono::{DateTime, Utc};
use std::path::Path;
use terminal::recording::{
    RecordingRuntimeError, RecordingSnapshot, RecordingState, RecordingTransition,
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecordingFooterStatus {
    Ready,
    Recording,
    Paused,
    Stopping,
    Stopped,
    Failed,
}

impl RecordingFooterStatus {
    pub(super) fn from_recording_state(state: &RecordingState) -> Self {
        match state {
            RecordingState::Idle => Self::Ready,
            RecordingState::Recording => Self::Recording,
            RecordingState::Paused => Self::Paused,
            RecordingState::Stopping => Self::Stopping,
            RecordingState::Stopped => Self::Stopped,
            RecordingState::Failed(_) => Self::Failed,
        }
    }

    pub(super) fn label(self) -> SharedString {
        match self {
            Self::Ready => t!("TerminalRecording.ready").to_string().into(),
            Self::Recording => t!("TerminalRecording.recording").to_string().into(),
            Self::Paused => t!("TerminalRecording.paused").to_string().into(),
            Self::Stopping => t!("TerminalRecording.stopping").to_string().into(),
            Self::Stopped => t!("TerminalRecording.stopped").to_string().into(),
            Self::Failed => t!("TerminalRecording.failed").to_string().into(),
        }
    }

    pub(super) fn color(self, cx: &App) -> Hsla {
        match self {
            Self::Ready => cx.theme().muted_foreground,
            Self::Recording => cx.theme().danger,
            Self::Paused | Self::Stopping => cx.theme().warning,
            Self::Stopped => cx.theme().success,
            Self::Failed => cx.theme().danger,
        }
    }

    pub(super) fn can_start(self) -> bool {
        matches!(self, Self::Ready | Self::Stopped | Self::Failed)
    }
}

impl TerminalView {
    pub(super) fn start_recording_action(
        &mut self,
        _: &StartRecording,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_recording_start(cx);
    }

    pub(super) fn pause_recording_action(
        &mut self,
        _: &PauseRecording,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_recording_pause(cx);
    }

    pub(super) fn resume_recording_action(
        &mut self,
        _: &ResumeRecording,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_recording_resume(cx);
    }

    pub(super) fn stop_recording_action(
        &mut self,
        _: &StopRecording,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_recording_stop(cx);
    }

    pub(super) fn request_recording_start(&mut self, cx: &mut Context<Self>) {
        if self.recording_path_prompt_pending || self.recording_session_is_active(cx) {
            return;
        }

        self.recording_path_prompt_pending = true;
        self.recording_control_error = None;
        self.sync_command_bar_session_controls(cx);
        cx.notify();

        let future = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(t!("TerminalRecording.select_directory").to_string().into()),
        });

        cx.spawn(async move |this, cx| {
            let selection: Result<Option<Vec<PathBuf>>, String> = match future.await {
                Ok(Ok(paths)) => Ok(paths),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };

            let _ = this.update(cx, |this, cx| {
                this.recording_path_prompt_pending = false;
                match selection {
                    Ok(Some(paths)) => {
                        if let Some(directory) = paths.into_iter().next() {
                            let output_path =
                                recording_output_path(&directory, Utc::now(), Uuid::new_v4());
                            let result = this.terminal.read(cx).start_output_recording(output_path);
                            this.apply_recording_control_result(result, cx);
                            return;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        this.recording_control_error = Some(format!(
                            "{}: {error}",
                            t!("TerminalRecording.select_directory_failed")
                        ));
                    }
                }

                this.sync_recording_ticker(cx);
                this.sync_command_bar_session_controls(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn request_recording_pause(&mut self, cx: &mut Context<Self>) {
        let result = self.terminal.read(cx).pause_recording();
        self.apply_recording_control_result(result, cx);
    }

    pub(super) fn request_recording_resume(&mut self, cx: &mut Context<Self>) {
        let result = self.terminal.read(cx).resume_recording();
        self.apply_recording_control_result(result, cx);
    }

    pub(super) fn request_recording_stop(&mut self, cx: &mut Context<Self>) {
        let result = self.terminal.read(cx).stop_recording();
        self.apply_recording_control_result(result, cx);
    }

    fn apply_recording_control_result(
        &mut self,
        result: Result<RecordingTransition, RecordingRuntimeError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(_) => self.recording_control_error = None,
            Err(error) => self.recording_control_error = Some(error.to_string()),
        }
        self.sync_recording_ticker(cx);
        self.sync_command_bar_session_controls(cx);
        cx.notify();
    }

    fn recording_session_is_active(&self, cx: &App) -> bool {
        self.terminal
            .read(cx)
            .recording_snapshot()
            .is_ok_and(|snapshot| {
                matches!(
                    snapshot.state,
                    RecordingState::Recording | RecordingState::Paused | RecordingState::Stopping
                )
            })
    }

    fn recording_elapsed_is_advancing(&self, cx: &App) -> bool {
        self.terminal
            .read(cx)
            .recording_snapshot()
            .is_ok_and(|snapshot| matches!(snapshot.state, RecordingState::Recording))
    }

    pub(super) fn sync_recording_ticker(&mut self, cx: &mut Context<Self>) {
        if self
            .recording_ticker
            .as_ref()
            .is_some_and(|task| task.is_ready())
        {
            self.recording_ticker.take();
        }

        if !self.recording_elapsed_is_advancing(cx) {
            self.recording_ticker.take();
            return;
        }
        if self.recording_ticker.is_some() {
            return;
        }

        self.recording_ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;

                let should_continue = this
                    .update(cx, |this, cx| {
                        let should_continue = this.recording_elapsed_is_advancing(cx);
                        if should_continue {
                            this.command_bar.update(cx, |bar, cx| {
                                bar.refresh_session_controls(cx);
                            });
                            cx.notify();
                        } else {
                            this.recording_ticker = None;
                        }
                        should_continue
                    })
                    .unwrap_or(false);
                if !should_continue {
                    break;
                }
            }
        }));
    }

    pub(super) fn sync_command_bar_session_controls(&self, cx: &mut Context<Self>) {
        let recording_path_prompt_pending = self.recording_path_prompt_pending;
        let recording_control_error = self.recording_control_error.clone();
        self.command_bar.update(cx, |bar, cx| {
            bar.set_session_controls_state(
                recording_path_prompt_pending,
                recording_control_error,
                cx,
            );
        });
    }
}

pub(super) fn recording_snapshot_failure(snapshot: &RecordingSnapshot) -> Option<String> {
    match &snapshot.state {
        RecordingState::Failed(failure) => Some(failure.to_string()),
        _ => snapshot.failure.as_ref().map(ToString::to_string),
    }
}

pub(super) fn format_recording_elapsed(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds / 60) % 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

pub(super) fn recording_output_path(
    directory: &Path,
    timestamp: DateTime<Utc>,
    recording_id: Uuid,
) -> PathBuf {
    directory.join(format!(
        "navop-terminal-{}-{recording_id}.cast",
        timestamp.format("%Y%m%d-%H%M%S")
    ))
}
