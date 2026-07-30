use super::*;
use crate::view::recording_footer::{
    RecordingFooterStatus, format_recording_elapsed, recording_snapshot_failure,
};
use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, AppContext, Context, DragMoveEvent, EntityId, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, LocalInputStyle},
};
use rust_i18n::t;

const COMMAND_BAR_COLLAPSED_HEIGHT: f32 = 30.0;
const COMMAND_BAR_INPUT_MIN_HEIGHT: f32 = 80.0;
const COMMAND_BAR_INPUT_MAX_HEIGHT: f32 = 400.0;
const COMMAND_BAR_RESIZE_HANDLE_HEIGHT: f32 = 6.0;
const COMMAND_BAR_POPOVER_GAP: f32 = 8.0;
const COMMAND_BAR_ACTIONS_WIDTH: f32 = 192.0;

#[derive(Clone)]
struct CommandBarResize {
    entity_id: EntityId,
    initial_height: f32,
    initial_y: Rc<Cell<Option<Pixels>>>,
}

impl Render for CommandBarResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

impl TerminalCommandBar {
    pub(super) fn popover_bottom_offset(&self) -> f32 {
        if self.collapsed {
            COMMAND_BAR_COLLAPSED_HEIGHT + COMMAND_BAR_POPOVER_GAP
        } else {
            self.input_height + COMMAND_BAR_POPOVER_GAP
        }
    }

    fn render_resize_handle(&self, cx: &mut Context<Self>) -> AnyElement {
        let initial_y = Rc::new(Cell::new(None));
        div()
            .id("terminal-command-resize-handle")
            .group("terminal-command-resize-handle")
            .w_full()
            .h(px(COMMAND_BAR_RESIZE_HANDLE_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .cursor_row_resize()
            .on_drag_move(cx.listener(Self::resize_input))
            .on_drag(
                CommandBarResize {
                    entity_id: cx.entity_id(),
                    initial_height: self.input_height,
                    initial_y,
                },
                |drag, _, window, cx| {
                    drag.initial_y.set(Some(window.mouse_position().y));
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                },
            )
            .child(
                div()
                    .w(px(32.0))
                    .h(px(2.0))
                    .rounded_full()
                    .bg(self.colors.border)
                    .group_hover("terminal-command-resize-handle", |handle| {
                        handle.w(px(48.0)).h(px(3.0)).bg(cx.theme().drag_border)
                    }),
            )
            .into_any_element()
    }

    fn resize_input(
        &mut self,
        event: &DragMoveEvent<CommandBarResize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let drag = event.drag(cx);
        if drag.entity_id != cx.entity_id() {
            return;
        }
        let Some(initial_y) = drag.initial_y.get() else {
            return;
        };
        let delta: f32 = (initial_y - event.event.position.y).into();
        self.input_height = (drag.initial_height + delta)
            .clamp(COMMAND_BAR_INPUT_MIN_HEIGHT, COMMAND_BAR_INPUT_MAX_HEIGHT);
        cx.notify();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .h(px(COMMAND_BAR_COLLAPSED_HEIGHT))
            .items_center()
            .justify_between()
            .child(self.render_toolbar_context(cx))
            .child(
                div()
                    .text_xs()
                    .text_color(self.colors.muted_foreground)
                    .child(t!("TerminalCommandBar.keyboard_hint").to_string()),
            )
            .into_any_element()
    }

    fn render_toolbar_context(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .min_w_0()
            .gap_1()
            .items_center()
            .child(
                Button::new("terminal-command-collapse-toggle")
                    .icon(if self.collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .ghost()
                    .xsmall()
                    .tooltip(if self.collapsed {
                        t!("TerminalCommandBar.expand").to_string()
                    } else {
                        t!("TerminalCommandBar.collapse").to_string()
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_collapsed(window, cx);
                    })),
            )
            .child(
                h_flex()
                    .min_w_0()
                    .gap_1()
                    .items_center()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(self.colors.border)
                    .px_2()
                    .py_px()
                    .text_xs()
                    .text_color(self.colors.muted_foreground)
                    .child(Icon::new(IconName::SquareTerminal).xsmall())
                    .child(div().truncate().child(self.target_label(cx))),
            )
            .child(self.render_quick_command_button(cx))
            .child(self.render_session_controls(cx))
            .into_any_element()
    }

    fn render_quick_command_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("terminal-command-quick")
            .icon(Icon::new(IconName::TerminalQuickCommandColor).color())
            .label(format!(
                "{} · {}",
                t!("TerminalCommandBar.quick_commands"),
                self.quick_commands.len()
            ))
            .ghost()
            .small()
            .when(self.quick_commands_open, |button| {
                button.bg(self.colors.muted)
            })
            .tooltip(t!("TerminalCommandBar.open_quick_commands").to_string())
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_quick_commands(window, cx);
            }))
            .into_any_element()
    }

    fn render_expanded_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap_1()
            .child(
                Button::new("terminal-command-collapse-toggle-expanded")
                    .icon(IconName::ChevronDown)
                    .ghost()
                    .xsmall()
                    .tooltip(t!("TerminalCommandBar.collapse").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_collapsed(window, cx);
                    })),
            )
            .child(self.render_quick_command_button(cx))
            .child(self.render_session_controls(cx))
            .into_any_element()
    }

    fn render_session_controls(&self, cx: &mut Context<Self>) -> AnyElement {
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
        let status_color = status.color(cx);
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

        h_flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(self.colors.border)
                    .px_2()
                    .py_px()
                    .text_xs()
                    .text_color(status_color)
                    .child(div().size_2().rounded_full().bg(status_color))
                    .child(status.label())
                    .when(
                        matches!(
                            status,
                            RecordingFooterStatus::Recording
                                | RecordingFooterStatus::Paused
                                | RecordingFooterStatus::Stopping
                        ),
                        |this| {
                            this.child(
                                div()
                                    .text_color(self.colors.muted_foreground)
                                    .child(format_recording_elapsed(elapsed)),
                            )
                        },
                    ),
            )
            .child(
                div()
                    .max_w(px(180.0))
                    .truncate()
                    .text_xs()
                    .text_color(
                        if runtime_error.is_some() || self.recording_control_error.is_some() {
                            cx.theme().danger
                        } else {
                            self.colors.muted_foreground
                        },
                    )
                    .child(detail),
            )
            .when(self.operation_history_available, |controls| {
                controls.child(
                    Button::new("terminal-command-operation-history-toggle")
                        .icon(IconName::TerminalHistoryColor)
                        .ghost()
                        .xsmall()
                        .when(self.operation_history_open, |button| {
                            button.bg(self.colors.muted)
                        })
                        .tooltip(t!("TerminalOperationHistory.tooltip").to_string())
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(TerminalCommandBarEvent::ToggleOperationHistory);
                        })),
                )
            })
            .children(self.render_recording_control_buttons(status, cx))
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
                    .ghost()
                    .xsmall()
                    .disabled(self.recording_path_prompt_pending)
                    .tooltip(t!("TerminalRecording.start").to_string())
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
                    .ghost()
                    .xsmall()
                    .tooltip(t!("TerminalRecording.pause").to_string())
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(TerminalCommandBarEvent::PauseRecording);
                    }))
                    .into_any_element(),
                self.render_stop_recording_button(false, cx),
            ],
            RecordingFooterStatus::Paused => vec![
                Button::new("terminal-command-recording-resume")
                    .icon(IconName::Play)
                    .ghost()
                    .xsmall()
                    .tooltip(t!("TerminalRecording.resume").to_string())
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
            .ghost()
            .xsmall()
            .disabled(disabled)
            .tooltip(t!("TerminalRecording.stop").to_string())
            .on_click(cx.listener(|_, _, _, cx| {
                cx.emit(TerminalCommandBarEvent::StopRecording);
            }))
            .into_any_element()
    }

    fn render_input_row(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .relative()
            .w_full()
            .h(px(self.input_height))
            .min_h(px(COMMAND_BAR_INPUT_MIN_HEIGHT))
            .py_1()
            .child(
                Input::new(&self.input_state)
                    .appearance(false)
                    .local_style(LocalInputStyle {
                        background: self.colors.background,
                        foreground: self.colors.foreground,
                        muted_foreground: self.colors.muted_foreground,
                        border: self.colors.border,
                    })
                    .h(px(self.input_height))
                    .w_full()
                    .pr(px(COMMAND_BAR_ACTIONS_WIDTH))
                    .text_color(self.colors.foreground)
                    .caret_color(self.colors.foreground)
                    .with_size(Size::Medium),
            )
            .child(
                div()
                    .absolute()
                    .top_2()
                    .right_0()
                    .child(self.render_expanded_actions(cx)),
            )
            .into_any_element()
    }
}

impl Render for TerminalCommandBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self
            .input_state
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        div()
            .relative()
            .flex_shrink_0()
            .w_full()
            .border_t_1()
            .border_color(self.colors.border)
            .bg(self.colors.background)
            .text_color(self.colors.foreground)
            .px_3()
            .py_px()
            .on_key_down(cx.listener(Self::handle_key_down))
            .when(
                focused && !self.collapsed && !self.suggestions.is_empty(),
                |bar| bar.child(self.render_suggestions(cx)),
            )
            .when(self.quick_commands_open, |bar| {
                bar.child(self.render_quick_commands(cx))
            })
            .when(self.collapsed, |bar| bar.child(self.render_toolbar(cx)))
            .when(!self.collapsed, |bar| {
                bar.child(self.render_resize_handle(cx))
                    .child(self.render_input_row(cx))
            })
    }
}
