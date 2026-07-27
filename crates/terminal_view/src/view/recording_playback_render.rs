use super::recording_playback_footer::{
    PLAYBACK_SPEED_PRESETS, RecordingPlaybackFooterStatus, RecordingPlaybackPartialRecoveryNotice,
    RecordingPlaybackSearchIndexNotice, format_playback_position, playback_partial_recovery_notice,
    playback_search_index_notice, playback_seek_target, playback_speed_is_selected,
};
use super::*;

pub(super) const RECORDING_PLAYBACK_FOOTER_HEIGHT: Pixels = px(64.0);

struct RecordingPlaybackFooterRenderState {
    available: bool,
    status: RecordingPlaybackFooterStatus,
    elapsed: Duration,
    duration: Duration,
    speed: f64,
    partial_recovery: Option<RecordingPlaybackPartialRecoveryNotice>,
    search_index: Option<RecordingPlaybackSearchIndexNotice>,
    error: Option<String>,
}

impl RecordingPlaybackFooterStatus {
    fn label(self) -> SharedString {
        match self {
            Self::Playing => t!("TerminalRecordingPlayback.playing").to_string().into(),
            Self::Paused => t!("TerminalRecordingPlayback.paused").to_string().into(),
            Self::Finished => t!("TerminalRecordingPlayback.finished").to_string().into(),
        }
    }

    fn color(self, cx: &App) -> Hsla {
        match self {
            Self::Playing => cx.theme().success,
            Self::Paused => cx.theme().warning,
            Self::Finished => cx.theme().muted_foreground,
        }
    }
}

impl TerminalView {
    pub(super) fn render_terminal_session_footer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.terminal.read(cx).is_recording_playback() {
            self.render_recording_playback_footer(cx)
        } else {
            self.render_recording_footer(cx)
        }
    }

    fn recording_playback_footer_render_state(
        &self,
        cx: &App,
    ) -> RecordingPlaybackFooterRenderState {
        let terminal = self.terminal.read(cx);
        let state = terminal.recording_playback_state();
        RecordingPlaybackFooterRenderState {
            available: state.is_some(),
            status: state
                .map(RecordingPlaybackFooterStatus::from)
                .unwrap_or(RecordingPlaybackFooterStatus::Paused),
            elapsed: terminal.recording_playback_elapsed().unwrap_or_default(),
            duration: terminal.recording_playback_duration().unwrap_or_default(),
            speed: terminal.recording_playback_speed().unwrap_or(1.0),
            partial_recovery: terminal
                .recording_playback_completeness()
                .and_then(playback_partial_recovery_notice),
            search_index: terminal
                .recording_playback_search_index_status()
                .and_then(playback_search_index_notice),
            error: self.recording_playback_control_error.clone(),
        }
    }

    pub(super) fn render_recording_playback_footer(
        &mut self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.recording_playback_footer_render_state(cx);
        let terminal_colors = self.current_theme.colors();
        let status_color = state.status.color(cx);
        let displayed_elapsed = self.recording_playback_displayed_elapsed(&state, cx);
        let detail = playback_detail(&state, cx);

        v_flex()
            .debug_selector(|| "terminal-recording-playback-footer".to_string())
            .w_full()
            .h(RECORDING_PLAYBACK_FOOTER_HEIGHT)
            .min_h(RECORDING_PLAYBACK_FOOTER_HEIGHT)
            .max_h(RECORDING_PLAYBACK_FOOTER_HEIGHT)
            .flex_shrink_0()
            .overflow_hidden()
            .gap_1()
            .px_3()
            .py_1()
            .border_t_1()
            .border_color(terminal_colors.border)
            .bg(terminal_colors.background)
            .text_size(px(11.0))
            .text_color(terminal_colors.foreground)
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(div().size_2().rounded_full().bg(status_color))
                    .child(
                        div()
                            .whitespace_nowrap()
                            .child(t!("TerminalRecordingPlayback.title")),
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
                            .text_color(cx.theme().warning)
                            .child(t!("TerminalRecordingPlayback.read_only")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(detail.1)
                            .child(detail.0),
                    )
                    .child(self.render_recording_playback_controls(&state, cx)),
            )
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(128.0))
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .text_color(terminal_colors.muted_foreground)
                            .child(format_playback_position(displayed_elapsed, state.duration)),
                    )
                    .child(
                        Slider::new(&self.recording_playback_slider)
                            .flex_1()
                            .min_w_0()
                            .disabled(!state.available),
                    ),
            )
            .into_any_element()
    }

    fn recording_playback_displayed_elapsed(
        &self,
        state: &RecordingPlaybackFooterRenderState,
        cx: &App,
    ) -> Duration {
        if !self.recording_playback_slider_dragging {
            return state.elapsed;
        }
        let progress = self.recording_playback_slider.read(cx).value().start();
        playback_seek_target(progress, state.duration)
    }

    fn render_recording_playback_controls(
        &mut self,
        state: &RecordingPlaybackFooterRenderState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .child(self.render_recording_playback_transport(state, cx))
            .children(
                PLAYBACK_SPEED_PRESETS
                    .into_iter()
                    .enumerate()
                    .map(|(index, speed)| {
                        Button::new(("terminal-recording-playback-speed", index))
                            .label(format!("{speed}x"))
                            .ghost()
                            .xsmall()
                            .selected(playback_speed_is_selected(state.speed, speed))
                            .disabled(!state.available)
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.request_recording_playback_speed(speed, cx);
                            }))
                    }),
            )
            .into_any_element()
    }

    fn render_recording_playback_transport(
        &mut self,
        state: &RecordingPlaybackFooterRenderState,
        cx: &mut Context<Self>,
    ) -> Button {
        match state.status {
            RecordingPlaybackFooterStatus::Playing => {
                Button::new("terminal-recording-playback-pause")
                    .label(t!("TerminalRecordingPlayback.pause"))
                    .icon(IconName::Pause)
                    .ghost()
                    .xsmall()
                    .disabled(!state.available)
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.request_recording_playback_pause(cx);
                    }))
            }
            RecordingPlaybackFooterStatus::Paused => {
                Button::new("terminal-recording-playback-resume")
                    .label(t!("TerminalRecordingPlayback.resume"))
                    .icon(IconName::Play)
                    .ghost()
                    .xsmall()
                    .disabled(!state.available)
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.request_recording_playback_resume(cx);
                    }))
            }
            RecordingPlaybackFooterStatus::Finished => {
                Button::new("terminal-recording-playback-replay")
                    .label(t!("TerminalRecordingPlayback.replay"))
                    .icon(IconName::Undo2)
                    .ghost()
                    .xsmall()
                    .disabled(!state.available)
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.request_recording_playback_replay(cx);
                    }))
            }
        }
    }
}

fn playback_detail(state: &RecordingPlaybackFooterRenderState, cx: &App) -> (String, Hsla) {
    if let Some(error) = &state.error {
        return (error.clone(), cx.theme().danger);
    }
    if !state.available {
        return (
            t!("TerminalRecordingPlayback.unavailable").to_string(),
            cx.theme().danger,
        );
    }

    let mut warnings = Vec::new();
    if let Some(notice) = state.partial_recovery {
        warnings.push(
            t!(
                "TerminalRecordingPlayback.partial_recovery",
                bytes = notice.discarded_bytes
            )
            .to_string(),
        );
    }
    if let Some(notice) = state.search_index {
        warnings.push(
            t!(
                "TerminalRecordingPlayback.search_index_truncated",
                events = notice.indexed_events,
                bytes = notice.indexed_text_bytes
            )
            .to_string(),
        );
    }
    (warnings.join(" · "), cx.theme().warning)
}
