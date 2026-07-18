use gpui::prelude::*;
use gpui::{
    Context, InteractiveElement, IntoElement, ListSizingBehavior, MouseButton, ParentElement,
    Render, SharedString, Styled, UniformListScrollHandle, Window, div, uniform_list,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    tooltip::Tooltip,
    v_flex,
};
use one_core::storage::{TerminalCommandHistory, TerminalCommandHistorySort};
use rust_i18n::t;
use std::ops::Range;

use super::HistoryCommandPanel;

impl HistoryCommandPanel {
    fn render_search_bar(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let has_query = !self.search_query.is_empty();
        h_flex()
            .flex_shrink_0()
            .h_8()
            .px_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(self.colors.border)
            .child(
                Icon::new(IconName::Search)
                    .xsmall()
                    .text_color(self.colors.muted_foreground),
            )
            .child(
                div().flex_1().child(
                    Input::new(&self.search_input_state)
                        .xsmall()
                        .appearance(false)
                        .cleanable(has_query),
                ),
            )
    }

    fn render_sort_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_shrink_0()
            .h_9()
            .px_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(self.colors.border)
            .child(self.render_sort_button(
                t!("HistoryCommand.most_used").to_string(),
                TerminalCommandHistorySort::MostUsed,
                cx,
            ))
            .child(self.render_sort_button(
                t!("HistoryCommand.latest").to_string(),
                TerminalCommandHistorySort::Latest,
                cx,
            ))
    }

    fn render_sort_button(
        &self,
        label: impl Into<SharedString>,
        sort: TerminalCommandHistorySort,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new(SharedString::from(format!("history-sort-{sort:?}")))
            .label(label.into())
            .xsmall()
            .when(self.sort == sort, |button| button.primary())
            .when(self.sort != sort, |button| button.ghost())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_sort(sort, cx);
            }))
    }

    fn render_command_item(
        &self,
        index: usize,
        item: &TerminalCommandHistory,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = item.id.unwrap_or(0);
        let command = item.command.clone();
        let command_for_row = command.clone();
        let command_for_paste = command.clone();
        let command_for_tooltip = command.clone();
        let favorite = item.favorite;
        let group_name = SharedString::from(format!("history-command-row-{index}"));
        let favorite_color = cx.theme().warning;

        div()
            .id(SharedString::from(format!("history-command-item-{index}")))
            .group(group_name.clone())
            .w_full()
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .hover(|s| s.bg(self.colors.muted))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.paste_command(command_for_row.clone(), cx);
                }),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(self.render_command_body(
                        index,
                        item,
                        favorite_color,
                        command_for_tooltip,
                    ))
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_1()
                            .invisible()
                            .group_hover(group_name, |s| s.visible())
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .child(self.render_favorite_button(
                                index,
                                id,
                                favorite,
                                favorite_color,
                                cx,
                            ))
                            .child(self.render_paste_button(index, command_for_paste, cx))
                            .child(self.render_delete_button(index, id, cx)),
                    ),
            )
    }

    fn render_command_body(
        &self,
        index: usize,
        item: &TerminalCommandHistory,
        favorite_color: gpui::Hsla,
        command_for_tooltip: String,
    ) -> impl IntoElement {
        let favorite = item.favorite;
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(if favorite {
                            IconName::StarFill
                        } else {
                            IconName::SquareTerminal
                        })
                        .with_size(Size::XSmall)
                        .text_color(if favorite {
                            favorite_color
                        } else {
                            self.colors.muted_foreground
                        }),
                    )
                    .child(self.render_command_text(index, &item.command, command_for_tooltip)),
            )
            .child(self.render_command_meta(item))
    }

    fn render_command_text(
        &self,
        index: usize,
        command: &str,
        tooltip: String,
    ) -> impl IntoElement {
        div()
            .id(SharedString::from(format!("history-command-text-{index}")))
            .flex_1()
            .min_w_0()
            .text_sm()
            .overflow_hidden()
            .text_ellipsis()
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(command.to_string())
    }

    fn render_command_meta(&self, item: &TerminalCommandHistory) -> impl IntoElement {
        h_flex()
            .gap_2()
            .text_xs()
            .text_color(self.colors.muted_foreground)
            .child(format!("x{}", item.use_count))
            .when_some(item.cwd.clone(), |this, cwd| {
                this.child(div().min_w_0().overflow_hidden().text_ellipsis().child(cwd))
            })
    }

    fn render_favorite_button(
        &self,
        index: usize,
        id: i64,
        favorite: bool,
        favorite_color: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new(SharedString::from(format!("history-favorite-{index}")))
            .icon(if favorite {
                IconName::StarOff
            } else {
                IconName::Star
            })
            .ghost()
            .xsmall()
            .tooltip(if favorite {
                t!("HistoryCommand.unfavorite").to_string()
            } else {
                t!("HistoryCommand.favorite").to_string()
            })
            .when(favorite, |button| button.text_color(favorite_color))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_favorite(id, cx);
            }))
    }

    fn render_paste_button(
        &self,
        index: usize,
        command: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new(SharedString::from(format!("history-paste-{index}")))
            .icon(IconName::Paste)
            .ghost()
            .xsmall()
            .tooltip(t!("HistoryCommand.paste").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.paste_command(command.clone(), cx);
            }))
    }

    fn render_delete_button(
        &self,
        index: usize,
        id: i64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new(SharedString::from(format!("history-delete-{index}")))
            .icon(IconName::Delete)
            .ghost()
            .xsmall()
            .tooltip(t!("HistoryCommand.delete").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.delete_command(id, cx);
            }))
    }

    fn render_empty_state(&self) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_sm()
                    .text_color(self.colors.muted_foreground)
                    .child(t!("HistoryCommand.no_commands").to_string()),
            )
    }
}

impl Render for HistoryCommandPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.commands.len();
        v_flex()
            .size_full()
            .bg(self.colors.background)
            .text_color(self.colors.foreground)
            .child(self.render_search_bar(cx))
            .child(self.render_sort_bar(cx))
            .when(count == 0, |this| this.child(self.render_empty_state()))
            .when(count > 0, |this| {
                this.child(self.render_command_list(count, &self.scroll_handle, cx))
            })
    }
}

impl HistoryCommandPanel {
    fn render_command_list(
        &self,
        count: usize,
        scroll_handle: &UniformListScrollHandle,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        uniform_list("history-command-list", count, {
            cx.processor(move |state: &mut Self, range: Range<usize>, _window, cx| {
                range
                    .map(|ix| {
                        let item = state.commands[ix].clone();
                        state.render_command_item(ix, &item, cx)
                    })
                    .collect()
            })
        })
        .flex_1()
        .size_full()
        .px_2()
        .py_1()
        .track_scroll(scroll_handle)
        .with_sizing_behavior(ListSizingBehavior::Auto)
    }
}
