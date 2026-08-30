use super::*;
use crate::view::recording_footer::{
    RecordingFooterStatus, format_recording_elapsed, recording_snapshot_failure,
};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Styled, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    h_flex, v_flex,
};
use rust_i18n::t;

const RECORDING_POPOVER_WIDTH: f32 = 340.0;

struct RecordingControlsState {
    status: RecordingFooterStatus,
    elapsed: std::time::Duration,
    detail: String,
    has_error: bool,
}

impl TerminalCommandBar {
    fn recording_controls_state(&self, cx: &mut Context<Self>) -> RecordingControlsState {
        let snapshot = self.terminal.read(cx).recording_snapshot();
        let (status, elapsed, capture_input, runtime_error) = match snapshot {
            Ok(snapshot) => (
                RecordingFooterStatus::from_recording_state(&snapshot.state),
                snapshot.elapsed,
                snapshot.capture_input,
                recording_snapshot_failure(&snapshot),
            ),
            Err(error) => (
                RecordingFooterStatus::Failed,
                std::time::Duration::ZERO,
                false,
                Some(error.to_string()),
            ),
        };
        let detail = if self.recording_path_prompt_pending {
            t!("TerminalRecording.selecting_directory").to_string()
        } else if let Some(error) = self
            .recording_control_error
            .as_ref()
            .or(runtime_error.as_ref())
        {
            error.clone()
        } else if capture_input {
            t!("TerminalRecording.input_included").to_string()
        } else {
            t!("TerminalRecording.output_only").to_string()
        };

        RecordingControlsState {
            status,
            elapsed,
            detail,
            has_error: runtime_error.is_some() || self.recording_control_error.is_some(),
        }
    }

    pub(super) fn render_recording_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.recording_controls_state(cx);
        let status_color = state.status.color(cx);

        Button::new("terminal-command-recording")
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(div().size_2().rounded_full().bg(status_color))
                    .child(t!("TerminalRecording.control").to_string()),
            )
            .custom(
                ButtonCustomVariant::new(cx)
                    .foreground(self.colors.foreground)
                    .hover(self.colors.muted)
                    .active(self.colors.muted),
            )
            .selected(self.recording_controls_open)
            .small()
            .tooltip(state.status.label())
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_recording_controls(window, cx);
            }))
            .into_any_element()
    }

    pub(super) fn render_recording_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.recording_controls_state(cx);
        let status_color = state.status.color(cx);
        let is_active = matches!(
            state.status,
            RecordingFooterStatus::Recording
                | RecordingFooterStatus::Paused
                | RecordingFooterStatus::Stopping
        );

        v_flex()
            .absolute()
            .bottom(px(self.popover_bottom_offset()))
            .right_3()
            .w(px(RECORDING_POPOVER_WIDTH))
            .max_w(gpui::relative(0.96))
            .overflow_hidden()
            .occlude()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(self.colors.border)
            .bg(self.colors.background)
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                let command_bar = cx.entity().downgrade();
                window.defer(cx, move |window, cx| {
                    let _ = command_bar.update(cx, |this, cx| {
                        if this.recording_controls_open {
                            this.close_recording_controls(window, cx);
                        }
                    });
                });
                let _ = this;
            }))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(self.colors.border)
                    .px_3()
                    .py_2()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .text_color(status_color)
                            .child(div().size_2().rounded_full().bg(status_color))
                            .child(state.status.label()),
                    )
                    .when(is_active, |header| {
                        header.child(
                            div()
                                .text_xs()
                                .text_color(self.colors.muted_foreground)
                                .child(format_recording_elapsed(state.elapsed)),
                        )
                    }),
            )
            .child(
                div()
                    .px_3()
                    .py_3()
                    .text_sm()
                    .text_color(if state.has_error {
                        cx.theme().danger
                    } else {
                        self.colors.muted_foreground
                    })
                    .child(state.detail),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .border_t_1()
                    .border_color(self.colors.border)
                    .px_3()
                    .py_2()
                    .children(self.render_recording_control_buttons(state.status, cx)),
            )
            .into_any_element()
    }

    fn render_recording_control_buttons(
        &self,
        status: RecordingFooterStatus,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if status.can_start() {
            return vec![
                Button::new("terminal-command-recording-start")
                    .icon(IconName::Play)
                    .label(t!("TerminalRecording.start").to_string())
                    .primary()
                    .small()
                    .disabled(self.recording_path_prompt_pending)
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(TerminalCommandBarEvent::StartRecording);
                    }))
                    .into_any_element(),
            ];
        }

        match status {
            RecordingFooterStatus::Recording => vec![
                Button::new("terminal-command-recording-pause")
                    .icon(IconName::Pause)
                    .label(t!("TerminalRecording.pause").to_string())
                    .secondary()
                    .small()
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(TerminalCommandBarEvent::PauseRecording);
                    }))
                    .into_any_element(),
                self.render_stop_recording_button(false, cx),
            ],
            RecordingFooterStatus::Paused => vec![
                Button::new("terminal-command-recording-resume")
                    .icon(IconName::Play)
                    .label(t!("TerminalRecording.resume").to_string())
                    .secondary()
                    .small()
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(TerminalCommandBarEvent::ResumeRecording);
                    }))
                    .into_any_element(),
                self.render_stop_recording_button(false, cx),
            ],
            RecordingFooterStatus::Stopping => {
                vec![self.render_stop_recording_button(true, cx)]
            }
            RecordingFooterStatus::Ready
            | RecordingFooterStatus::Stopped
            | RecordingFooterStatus::Failed => Vec::new(),
        }
    }

    fn render_stop_recording_button(&self, disabled: bool, cx: &mut Context<Self>) -> AnyElement {
        Button::new("terminal-command-recording-stop")
            .icon(IconName::CircleX)
            .label(t!("TerminalRecording.stop").to_string())
            .danger()
            .small()
            .disabled(disabled)
            .on_click(cx.listener(|_, _, _, cx| {
                cx.emit(TerminalCommandBarEvent::StopRecording);
            }))
            .into_any_element()
    }
}
