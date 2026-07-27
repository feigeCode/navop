use super::*;
use chrono::{DateTime, Utc};
use std::path::Path;
use terminal::recording::{
    RecordingRuntimeError, RecordingSnapshot, RecordingState, RecordingTransition,
};
use uuid::Uuid;

pub(super) const RECORDING_FOOTER_HEIGHT: Pixels = px(40.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordingFooterStatus {
    Ready,
    Recording,
    Paused,
    Stopping,
    Stopped,
    Failed,
}

impl RecordingFooterStatus {
    fn from_recording_state(state: &RecordingState) -> Self {
        match state {
            RecordingState::Idle => Self::Ready,
            RecordingState::Recording => Self::Recording,
            RecordingState::Paused => Self::Paused,
            RecordingState::Stopping => Self::Stopping,
            RecordingState::Stopped => Self::Stopped,
            RecordingState::Failed(_) => Self::Failed,
        }
    }

    fn label(self) -> SharedString {
        match self {
            Self::Ready => t!("TerminalRecording.ready").to_string().into(),
            Self::Recording => t!("TerminalRecording.recording").to_string().into(),
            Self::Paused => t!("TerminalRecording.paused").to_string().into(),
            Self::Stopping => t!("TerminalRecording.stopping").to_string().into(),
            Self::Stopped => t!("TerminalRecording.stopped").to_string().into(),
            Self::Failed => t!("TerminalRecording.failed").to_string().into(),
        }
    }

    fn color(self, cx: &App) -> Hsla {
        match self {
            Self::Ready => cx.theme().muted_foreground,
            Self::Recording => cx.theme().danger,
            Self::Paused | Self::Stopping => cx.theme().warning,
            Self::Stopped => cx.theme().success,
            Self::Failed => cx.theme().danger,
        }
    }

    fn can_start(self) -> bool {
        matches!(self, Self::Ready | Self::Stopped | Self::Failed)
    }
}

struct RecordingFooterRenderState {
    status: RecordingFooterStatus,
    elapsed: Duration,
    capture_input: bool,
    path_prompt_pending: bool,
    error: Option<String>,
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

    fn request_recording_start(&mut self, cx: &mut Context<Self>) {
        if self.recording_path_prompt_pending || self.recording_session_is_active(cx) {
            return;
        }

        self.recording_path_prompt_pending = true;
        self.recording_control_error = None;
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
                cx.notify();
            });
        })
        .detach();
    }

    fn request_recording_pause(&mut self, cx: &mut Context<Self>) {
        let result = self.terminal.read(cx).pause_recording();
        self.apply_recording_control_result(result, cx);
    }

    fn request_recording_resume(&mut self, cx: &mut Context<Self>) {
        let result = self.terminal.read(cx).resume_recording();
        self.apply_recording_control_result(result, cx);
    }

    fn request_recording_stop(&mut self, cx: &mut Context<Self>) {
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

    fn recording_footer_render_state(&self, cx: &App) -> RecordingFooterRenderState {
        let control_error = self.recording_control_error.clone();
        match self.terminal.read(cx).recording_snapshot() {
            Ok(snapshot) => RecordingFooterRenderState {
                status: RecordingFooterStatus::from_recording_state(&snapshot.state),
                elapsed: snapshot.elapsed,
                capture_input: snapshot.capture_input,
                path_prompt_pending: self.recording_path_prompt_pending,
                error: control_error.or_else(|| recording_snapshot_failure(&snapshot)),
            },
            Err(error) => RecordingFooterRenderState {
                status: RecordingFooterStatus::Failed,
                elapsed: Duration::ZERO,
                capture_input: false,
                path_prompt_pending: self.recording_path_prompt_pending,
                error: control_error.or_else(|| Some(error.to_string())),
            },
        }
    }

    pub(super) fn render_recording_footer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.recording_footer_render_state(cx);
        let terminal_colors = self.current_theme.colors();
        let status_color = state.status.color(cx);
        let disclosure = if state.capture_input {
            t!("TerminalRecording.input_included").to_string()
        } else {
            t!("TerminalRecording.output_only").to_string()
        };
        let disclosure_color = if state.capture_input {
            cx.theme().warning
        } else {
            terminal_colors.muted_foreground
        };
        let detail = if state.path_prompt_pending {
            Some((
                t!("TerminalRecording.selecting_directory").to_string(),
                terminal_colors.muted_foreground,
            ))
        } else {
            state.error.clone().map(|error| (error, cx.theme().danger))
        };

        h_flex()
            .debug_selector(|| "terminal-recording-footer".to_string())
            .w_full()
            .h(RECORDING_FOOTER_HEIGHT)
            .min_h(RECORDING_FOOTER_HEIGHT)
            .max_h(RECORDING_FOOTER_HEIGHT)
            .flex_shrink_0()
            .overflow_hidden()
            .items_center()
            .gap_3()
            .px_3()
            .border_t_1()
            .border_color(terminal_colors.border)
            .bg(terminal_colors.background)
            .text_size(px(11.0))
            .text_color(terminal_colors.foreground)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size_2()
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(status_color),
                    )
                    .child(
                        div()
                            .whitespace_nowrap()
                            .text_color(status_color)
                            .child(state.status.label()),
                    )
                    .child(
                        div()
                            .whitespace_nowrap()
                            .text_color(terminal_colors.muted_foreground)
                            .child(format_recording_elapsed(state.elapsed)),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .justify_center()
                    .overflow_hidden()
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(disclosure_color)
                            .child(disclosure),
                    )
                    .when_some(detail, |this, (message, color)| {
                        this.child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_color(color)
                                .child(message),
                        )
                    }),
            )
            .child(self.render_recording_controls(state.status, state.path_prompt_pending, cx))
            .into_any_element()
    }

    fn render_recording_controls(
        &mut self,
        status: RecordingFooterStatus,
        path_prompt_pending: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let controls = h_flex().flex_shrink_0().items_center().gap_1();
        match status {
            status if status.can_start() => controls
                .child(
                    Button::new("terminal-recording-start")
                        .label(t!("TerminalRecording.start"))
                        .icon(IconName::Play)
                        .ghost()
                        .xsmall()
                        .disabled(path_prompt_pending)
                        .on_click(cx.listener(|this, _, _window, cx| {
                            this.request_recording_start(cx);
                        })),
                )
                .into_any_element(),
            RecordingFooterStatus::Recording => controls
                .child(
                    Button::new("terminal-recording-pause")
                        .label(t!("TerminalRecording.pause"))
                        .icon(IconName::Pause)
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|this, _, _window, cx| {
                            this.request_recording_pause(cx);
                        })),
                )
                .child(
                    Button::new("terminal-recording-stop")
                        .label(t!("TerminalRecording.stop"))
                        .icon(IconName::CircleX)
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|this, _, _window, cx| {
                            this.request_recording_stop(cx);
                        })),
                )
                .into_any_element(),
            RecordingFooterStatus::Paused => controls
                .child(
                    Button::new("terminal-recording-resume")
                        .label(t!("TerminalRecording.resume"))
                        .icon(IconName::Play)
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|this, _, _window, cx| {
                            this.request_recording_resume(cx);
                        })),
                )
                .child(
                    Button::new("terminal-recording-stop")
                        .label(t!("TerminalRecording.stop"))
                        .icon(IconName::CircleX)
                        .ghost()
                        .xsmall()
                        .on_click(cx.listener(|this, _, _window, cx| {
                            this.request_recording_stop(cx);
                        })),
                )
                .into_any_element(),
            RecordingFooterStatus::Stopping => controls
                .child(
                    Button::new("terminal-recording-stopping")
                        .label(t!("TerminalRecording.stop"))
                        .icon(IconName::CircleX)
                        .ghost()
                        .xsmall()
                        .disabled(true),
                )
                .into_any_element(),
            RecordingFooterStatus::Ready
            | RecordingFooterStatus::Stopped
            | RecordingFooterStatus::Failed => unreachable!(),
        }
    }
}

fn recording_snapshot_failure(snapshot: &RecordingSnapshot) -> Option<String> {
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
