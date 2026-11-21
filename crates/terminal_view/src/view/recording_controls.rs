//! 终端录制按钮与弹层。
//!
//! 录制入口从命令栏迁到 pane 右上角的悬浮工具条（分屏按钮旁）。状态与操作
//! 仍由每个 `TerminalView` 独占：按钮只负责开关弹层，具体的开始/暂停/恢复/
//! 停止直接调用 `request_recording_*`，不再经过命令栏事件。

use super::*;
use crate::view::recording_footer::{
    RecordingFooterStatus, format_recording_elapsed, recording_snapshot_failure,
};
use gpui::{
    AnyElement, Context, Hsla, InteractiveElement, IntoElement, ParentElement, Styled, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use one_assets::IconName;
use rust_i18n::t;

pub(crate) const RECORDING_POPOVER_WIDTH: f32 = 340.0;

struct RecordingControlsState {
    status: RecordingFooterStatus,
    elapsed: std::time::Duration,
    detail: String,
    has_error: bool,
}

impl TerminalView {
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

    /// 悬浮工具条上的录制按钮状态：文案、状态色。
    pub(crate) fn recording_indicator(&self, cx: &mut Context<Self>) -> (SharedString, Hsla) {
        let state = self.recording_controls_state(cx);
        (state.status.label(), state.status.color(cx))
    }

    pub(crate) fn recording_controls_open(&self) -> bool {
        self.recording_controls_open
    }

    /// 非分屏时 pane 工具条默认悬浮才显示；录制中或弹层打开时必须常驻，
    /// 否则用户看不到录制状态。
    pub(crate) fn recording_tool_visible(&self, cx: &mut Context<Self>) -> bool {
        self.recording_controls_open || self.recording_path_prompt_pending || {
            let state = self.recording_controls_state(cx);
            matches!(
                state.status,
                RecordingFooterStatus::Recording
                    | RecordingFooterStatus::Paused
                    | RecordingFooterStatus::Stopping
            )
        }
    }

    pub(crate) fn toggle_recording_controls(&mut self, cx: &mut Context<Self>) {
        if self.recording_controls_open {
            self.recording_controls_open = false;
        } else {
            self.recording_controls_open = true;
            self.command_bar.update(cx, |bar, cx| {
                bar.reset_overlays(cx);
            });
        }
        cx.emit(TerminalPaneEvent::RecordingChanged);
        cx.notify();
    }

    pub(crate) fn render_recording_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.recording_controls_state(cx);
        let status_color = state.status.color(cx);
        let border = cx.theme().border;
        let is_active = matches!(
            state.status,
            RecordingFooterStatus::Recording
                | RecordingFooterStatus::Paused
                | RecordingFooterStatus::Stopping
        );

        v_flex()
            .w(px(RECORDING_POPOVER_WIDTH))
            .max_w(gpui::relative(0.96))
            .overflow_hidden()
            .occlude()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(border)
            .bg(cx.theme().background)
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.recording_controls_open = false;
                cx.emit(TerminalPaneEvent::RecordingChanged);
                cx.notify();
            }))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(border)
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
                                .text_color(cx.theme().muted_foreground)
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
                        cx.theme().muted_foreground
                    })
                    .child(state.detail),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .border_t_1()
                    .border_color(border)
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
                Button::new("terminal-pane-recording-start")
                    .icon(IconName::Play)
                    .label(t!("TerminalRecording.start").to_string())
                    .primary()
                    .small()
                    .disabled(self.recording_path_prompt_pending)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.request_recording_start(cx);
                    }))
                    .into_any_element(),
            ];
        }

        match status {
            RecordingFooterStatus::Recording => vec![
                Button::new("terminal-pane-recording-pause")
                    .icon(IconName::Pause)
                    .label(t!("TerminalRecording.pause").to_string())
                    .secondary()
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.request_recording_pause(cx);
                    }))
                    .into_any_element(),
                self.render_stop_recording_button(false, cx),
            ],
            RecordingFooterStatus::Paused => vec![
                Button::new("terminal-pane-recording-resume")
                    .icon(IconName::Play)
                    .label(t!("TerminalRecording.resume").to_string())
                    .secondary()
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.request_recording_resume(cx);
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
        Button::new("terminal-pane-recording-stop")
            .icon(IconName::CircleX)
            .label(t!("TerminalRecording.stop").to_string())
            .danger()
            .small()
            .disabled(disabled)
            .on_click(cx.listener(|this, _, _, cx| {
                this.request_recording_stop(cx);
            }))
            .into_any_element()
    }
}
