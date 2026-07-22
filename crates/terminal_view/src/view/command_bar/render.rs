use super::*;
use gpui::{
    AnyElement, AppContext, Context, DragMoveEvent, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, LocalInputStyle},
};
use one_ui::resize_handle::ResizePanel;
use rust_i18n::t;

const COMMAND_BAR_COLLAPSED_HEIGHT: f32 = 30.0;
const COMMAND_BAR_INPUT_MIN_HEIGHT: f32 = 80.0;
const COMMAND_BAR_INPUT_MAX_HEIGHT: f32 = 400.0;
const COMMAND_BAR_RESIZE_HANDLE_HEIGHT: f32 = 6.0;

impl TerminalCommandBar {
    fn render_resize_handle(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("terminal-command-resize-handle")
            .w_full()
            .h(px(COMMAND_BAR_RESIZE_HANDLE_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .cursor_row_resize()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_drag_move(cx.listener(Self::resize_input))
            .on_drag(ResizePanel, |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .child(
                div()
                    .w(px(32.0))
                    .h(px(2.0))
                    .rounded_full()
                    .bg(self.colors.border),
            )
            .into_any_element()
    }

    fn resize_input(
        &mut self,
        event: &DragMoveEvent<ResizePanel>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = event.drag(cx);
        let delta: f32 = (event.bounds.center().y - event.event.position.y).into();
        self.input_height = (self.input_height + delta)
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

    fn render_input_row(&self, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .h(px(self.input_height))
            .min_h(px(COMMAND_BAR_INPUT_MIN_HEIGHT))
            .gap_2()
            .items_center()
            .py_1()
            .child(
                Button::new("terminal-command-collapse-toggle")
                    .icon(IconName::ChevronDown)
                    .ghost()
                    .xsmall()
                    .tooltip(t!("TerminalCommandBar.collapse").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_collapsed(window, cx);
                    })),
            )
            .child(
                h_flex()
                    .max_w(px(220.0))
                    .min_w_0()
                    .flex_shrink_0()
                    .gap_1()
                    .items_center()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(if focused {
                        self.colors.accent
                    } else {
                        self.colors.border
                    })
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(self.colors.muted_foreground)
                    .child(Icon::new(IconName::SquareTerminal).xsmall())
                    .child(div().truncate().child(self.target_label(cx))),
            )
            .child(Icon::new(IconName::ChevronRight).small().flex_shrink_0())
            .child(
                Input::new(&self.input_state)
                    .appearance(false)
                    .local_style(LocalInputStyle {
                        background: self.colors.background,
                        foreground: self.colors.foreground,
                        muted_foreground: self.colors.muted_foreground,
                        border: self.colors.border,
                    })
                    .w_full()
                    .text_color(self.colors.foreground)
                    .caret_color(self.colors.foreground)
                    .with_size(Size::Medium),
            )
            .child(self.render_quick_command_button(cx))
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
                    .child(self.render_input_row(focused, cx))
            })
    }
}
