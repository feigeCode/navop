use super::*;
use crate::view::command_bar_model::{CommandSuggestion, CommandSuggestionKind};
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, h_flex, scroll::ScrollableElement, v_flex,
};
use rust_i18n::t;

const COMMAND_POPOVER_WIDTH: f32 = 720.0;
const COMMAND_POPOVER_MAX_HEIGHT: f32 = 300.0;

impl TerminalCommandBar {
    pub(super) fn render_suggestions(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = v_flex().w_full();
        let mut group = None;
        for (index, suggestion) in self.suggestions.iter().cloned().enumerate() {
            if group != Some(suggestion.kind) {
                group = Some(suggestion.kind);
                content = content.child(self.render_suggestion_group(suggestion.kind));
            }
            content = content.child(self.render_suggestion(index, suggestion, cx));
        }
        v_flex()
            .absolute()
            .bottom(px(self.popover_bottom_offset()))
            .left_3()
            .w(px(COMMAND_POPOVER_WIDTH))
            .max_w(gpui::relative(0.96))
            .max_h(px(COMMAND_POPOVER_MAX_HEIGHT))
            .overflow_hidden()
            .occlude()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(self.colors.border)
            .bg(self.colors.background)
            .text_color(self.colors.foreground)
            .shadow_lg()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                content
                    .max_h(px(COMMAND_POPOVER_MAX_HEIGHT))
                    .overflow_y_scrollbar(),
            )
            .into_any_element()
    }

    fn render_suggestion_group(&self, kind: CommandSuggestionKind) -> AnyElement {
        let title = match kind {
            CommandSuggestionKind::QuickCommand => t!("TerminalCommandBar.quick_commands"),
            CommandSuggestionKind::History => t!("TerminalCommandBar.history"),
        };
        div()
            .w_full()
            .border_b_1()
            .border_color(self.colors.border)
            .bg(self.colors.muted)
            .px_3()
            .py_1()
            .text_xs()
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(self.colors.muted_foreground)
            .child(title.to_string().to_uppercase())
            .into_any_element()
    }

    fn render_suggestion(
        &self,
        index: usize,
        suggestion: CommandSuggestion,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected_suggestion == Some(index);
        let command = suggestion.command.clone();
        h_flex()
            .id(("terminal-command-suggestion", index))
            .w_full()
            .min_w_0()
            .gap_3()
            .items_center()
            .px_3()
            .py_2()
            .cursor_pointer()
            .text_color(if selected {
                self.colors.foreground
            } else {
                self.colors.muted_foreground
            })
            .when(selected, |row| row.bg(self.colors.muted))
            .hover(|row| row.bg(self.colors.muted).text_color(self.colors.foreground))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.choose_command(command.clone(), window, cx);
            }))
            .child(self.render_suggestion_text(&suggestion))
            .child(self.render_source_pill(suggestion.kind))
            .into_any_element()
    }

    fn render_suggestion_text(&self, suggestion: &CommandSuggestion) -> AnyElement {
        h_flex()
            .min_w_0()
            .flex_1()
            .gap_3()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_sm()
                    .child(suggestion.label.clone()),
            )
            .when_some(suggestion.detail.clone(), |row, detail| {
                row.child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_xs()
                        .text_color(self.colors.muted_foreground)
                        .child(detail),
                )
            })
            .into_any_element()
    }

    fn render_source_pill(&self, kind: CommandSuggestionKind) -> AnyElement {
        let (icon, label) = match kind {
            CommandSuggestionKind::QuickCommand => (
                IconName::TerminalQuickCommandColor,
                t!("TerminalCommandBar.quick_source").to_string(),
            ),
            CommandSuggestionKind::History => (
                IconName::TerminalHistoryColor,
                t!("TerminalCommandBar.history_source").to_string(),
            ),
        };
        h_flex()
            .flex_shrink_0()
            .gap_1()
            .items_center()
            .rounded_full()
            .border_1()
            .border_color(self.colors.border)
            .px_2()
            .py_px()
            .text_xs()
            .child(Icon::new(icon).color().xsmall())
            .child(label)
            .into_any_element()
    }
}
