use super::*;
use crate::view::command_bar::quick_render::group_color;
use crate::view::command_bar_model::QuickCommandGroup;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, h_flex, input::Input, scroll::ScrollableElement,
    v_flex,
};
use rust_i18n::t;

const QUICK_POPOVER_HEADER_HEIGHT: f32 = 48.0;
const QUICK_COMMAND_GROUP_ID_STRIDE: usize = 10_000;

impl TerminalCommandBar {
    pub(super) fn render_quick_body(
        &self,
        groups: Vec<QuickCommandGroup>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .overflow_hidden()
            .child(self.render_quick_search())
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .when(groups.is_empty(), |list| {
                        list.child(self.render_quick_empty())
                    })
                    .children(index_quick_groups(groups).map(
                        |(group_index, command_offset, group)| {
                            self.render_quick_group(group_index, command_offset, group, cx)
                        },
                    )),
            )
            .into_any_element()
    }

    fn render_quick_search(&self) -> AnyElement {
        h_flex()
            .h(px(QUICK_POPOVER_HEADER_HEIGHT))
            .flex_shrink_0()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(self.colors.border)
            .px_3()
            .child(Icon::new(IconName::Search).xsmall())
            .child(
                Input::new(&self.quick_search_state)
                    .appearance(false)
                    .w_full()
                    .with_size(Size::Small),
            )
            .into_any_element()
    }

    fn render_quick_empty(&self) -> AnyElement {
        v_flex()
            .h(px(180.0))
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(self.colors.muted_foreground)
            .child(Icon::new(IconName::TerminalQuickCommandColor).with_size(Size::Medium))
            .child(
                div()
                    .text_sm()
                    .child(t!("TerminalCommandBar.no_quick_commands").to_string()),
            )
            .into_any_element()
    }

    fn render_quick_group(
        &self,
        group_index: usize,
        command_offset: usize,
        group: QuickCommandGroup,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = group
            .name
            .unwrap_or_else(|| t!("TerminalCommandBar.ungrouped").to_string());
        let color = group_color(group.color.as_deref(), self.colors.accent);
        v_flex()
            .id(("terminal-quick-command-group", group_index))
            .w_full()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .child(div().size_2().rounded_full().bg(color))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(title),
                    ),
            )
            .children(
                group
                    .commands
                    .into_iter()
                    .enumerate()
                    .map(|(command_index, command)| {
                        self.render_quick_command(
                            (group_index, command_offset + command_index),
                            command,
                            cx,
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_quick_command(
        &self,
        item_index: (usize, usize),
        command: QuickCommand,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (group_index, command_index) = item_index;
        let selected = self.selected_quick_command == Some(command_index);
        let value = command.command.clone();
        let label = command.name.clone().unwrap_or_else(|| value.clone());
        h_flex()
            .id((
                "terminal-quick-command-item",
                group_index * QUICK_COMMAND_GROUP_ID_STRIDE + command_index,
            ))
            .mx_2()
            .mb_1()
            .min_w_0()
            .gap_2()
            .items_center()
            .rounded(cx.theme().radius)
            .px_2()
            .py_2()
            .cursor_pointer()
            .when(selected, |row| row.bg(self.colors.muted))
            .hover(|row| row.bg(self.colors.muted))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.choose_command(value.clone(), window, cx);
            }))
            .child(self.render_quick_command_text(label, command))
            .child(Icon::new(IconName::ChevronRight).xsmall())
            .into_any_element()
    }

    fn render_quick_command_text(&self, label: String, command: QuickCommand) -> AnyElement {
        v_flex()
            .min_w_0()
            .flex_1()
            .gap_1()
            .child(
                div()
                    .truncate()
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(label),
            )
            .child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(self.colors.accent)
                    .child(command.command),
            )
            .when_some(command.description, |row, description| {
                row.child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(self.colors.muted_foreground)
                        .child(description),
                )
            })
            .into_any_element()
    }
}

fn index_quick_groups(
    groups: Vec<QuickCommandGroup>,
) -> impl Iterator<Item = (usize, usize, QuickCommandGroup)> {
    groups
        .into_iter()
        .enumerate()
        .scan(0, |offset, (group_index, group)| {
            let command_offset = *offset;
            *offset += group.commands.len();
            Some((group_index, command_offset, group))
        })
}
