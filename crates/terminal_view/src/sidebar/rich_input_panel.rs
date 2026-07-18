//! Rich Input 面板
//!
//! 用于输入或粘贴长命令，再通过终端已有粘贴路径送入终端输入区。

use crate::theme::TerminalColors;
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, Styled, Window, div,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable, Size,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use rust_i18n::t;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RichInputSubmit {
    Empty,
    Command(String),
}

pub fn prepare_rich_input_submit(input: &str) -> RichInputSubmit {
    if input.trim().is_empty() {
        RichInputSubmit::Empty
    } else {
        RichInputSubmit::Command(input.to_string())
    }
}

#[derive(Clone, Debug)]
pub enum RichInputPanelEvent {
    ExecuteCommand(String),
}

pub struct RichInputPanel {
    input_state: Entity<InputState>,
    has_input: bool,
    focus_handle: FocusHandle,
    colors: TerminalColors,
    _subscriptions: Vec<gpui::Subscription>,
}

impl RichInputPanel {
    pub fn new(colors: TerminalColors, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(10, 18)
                .placeholder(t!("RichInput.command_placeholder").to_string())
        });

        let input_entity = input_state.clone();
        let input_sub = cx.subscribe_in(
            &input_state,
            window,
            move |this, _state, event, _window, cx| {
                if let InputEvent::Change = event {
                    this.has_input = !input_entity.read(cx).value().trim().is_empty();
                    cx.notify();
                }
            },
        );

        Self {
            input_state,
            has_input: false,
            focus_handle: cx.focus_handle(),
            colors,
            _subscriptions: vec![input_sub],
        }
    }

    pub fn set_colors(&mut self, colors: TerminalColors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
    }

    fn submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let value = self.input_state.read(cx).value().to_string();
        match prepare_rich_input_submit(&value) {
            RichInputSubmit::Empty => {}
            RichInputSubmit::Command(command) => {
                cx.emit(RichInputPanelEvent::ExecuteCommand(command));
            }
        }
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input_state.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.has_input = false;
        cx.notify();
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_shrink_0()
            .w_full()
            .items_center()
            .justify_end()
            .gap_2()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(self.colors.border)
            .bg(self.colors.muted)
            .child(
                Button::new("rich-input-clear")
                    .icon(IconName::Remove)
                    .label(t!("RichInput.clear").to_string())
                    .ghost()
                    .small()
                    .disabled(!self.has_input)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.clear(window, cx);
                    })),
            )
            .child(
                Button::new("rich-input-send")
                    .icon(IconName::Paste)
                    .label(t!("RichInput.send").to_string())
                    .primary()
                    .small()
                    .disabled(!self.has_input)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.submit(window, cx);
                    })),
            )
    }
}

impl EventEmitter<RichInputPanelEvent> for RichInputPanel {}

impl Focusable for RichInputPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RichInputPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input_focused = self
            .input_state
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);

        v_flex()
            .size_full()
            .bg(self.colors.background)
            .text_color(self.colors.foreground)
            .child(
                div().flex_1().min_h_0().p_3().child(
                    div()
                        .size_full()
                        .border_1()
                        .rounded(cx.theme().radius)
                        .border_color(if input_focused {
                            self.colors.accent
                        } else {
                            self.colors.border
                        })
                        .bg(self.colors.background)
                        .child(
                            Input::new(&self.input_state)
                                .h_full()
                                .w_full()
                                .appearance(false)
                                .text_color(self.colors.foreground)
                                .caret_color(self.colors.foreground)
                                .with_size(Size::Small),
                        ),
                ),
            )
            .child(self.render_footer(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{RichInputSubmit, prepare_rich_input_submit};

    #[test]
    fn rich_input_submit_preserves_multiline_command() {
        let command = "cat <<'EOF'\nhello\nEOF";

        assert_eq!(
            RichInputSubmit::Command(command.to_string()),
            prepare_rich_input_submit(command)
        );
    }

    #[test]
    fn rich_input_submit_ignores_blank_input() {
        assert_eq!(RichInputSubmit::Empty, prepare_rich_input_submit(" \n\t "));
    }
}
