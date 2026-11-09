use super::*;
use crate::view::command_bar::quick_render::group_color;
use crate::view::command_bar_model::QuickCommandGroup;
use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Icon, Sizable, Size, button::Button, h_flex, input::Input,
    scroll::ScrollableElement, v_flex, v_virtual_list,
};
use one_assets::IconName;
use one_ui::{IconButton, IconButtonRole};
use rust_i18n::t;
use std::rc::Rc;

const QUICK_POPOVER_HEADER_HEIGHT: f32 = 48.0;
const QUICK_GROUP_ROW_HEIGHT: f32 = 32.0;
const QUICK_COMMAND_BASE_ROW_HEIGHT: f32 = 40.0;
const QUICK_COMMAND_METADATA_ROW_HEIGHT: f32 = 20.0;
const QUICK_COMMAND_GROUP_ID_STRIDE: usize = 10_000;

#[derive(Clone)]
enum QuickCommandListItem {
    Group {
        group_index: usize,
        title: String,
        color: gpui::Hsla,
    },
    Command {
        group_index: usize,
        command_index: usize,
        command: QuickCommand,
    },
}

impl QuickCommandListItem {
    fn height(&self) -> f32 {
        match self {
            Self::Group { .. } => QUICK_GROUP_ROW_HEIGHT,
            Self::Command { command, .. } => {
                let value = command.command.trim();
                let has_label = command
                    .name
                    .as_deref()
                    .is_some_and(|name| name.trim() != value);
                let metadata_rows =
                    usize::from(has_label) + usize::from(command.description.is_some());
                QUICK_COMMAND_BASE_ROW_HEIGHT
                    + metadata_rows as f32 * QUICK_COMMAND_METADATA_ROW_HEIGHT
            }
        }
    }
}

impl TerminalCommandBar {
    pub(super) fn render_quick_body(
        &self,
        groups: Vec<QuickCommandGroup>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let items = Rc::new(self.quick_command_list_items(groups));
        let item_sizes = Rc::new(
            items
                .iter()
                .map(|item| gpui::size(px(0.0), px(item.height())))
                .collect(),
        );
        let items_empty = items.is_empty();

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
                    .relative()
                    .when(items_empty, |body| body.child(self.render_quick_empty(cx)))
                    .when(!items_empty, |body| {
                        body.child(
                            v_virtual_list(
                                cx.entity().clone(),
                                "terminal-quick-command-list",
                                item_sizes,
                                move |this, visible_range, _window, cx| {
                                    visible_range
                                        .filter_map(|index| {
                                            items
                                                .get(index)
                                                .cloned()
                                                .map(|item| this.render_quick_list_item(item, cx))
                                        })
                                        .collect()
                                },
                            )
                            .size_full()
                            .track_scroll(&self.quick_scroll_handle),
                        )
                    })
                    .vertical_scrollbar(&self.quick_scroll_handle),
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
                    .text_color(self.colors.foreground)
                    .with_size(Size::Small),
            )
            .into_any_element()
    }

    fn render_quick_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .h(px(180.0))
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(self.colors.muted_foreground)
            .child(
                Icon::new(IconName::TerminalQuickCommandColor)
                    .color()
                    .with_size(Size::Medium),
            )
            .child(
                div()
                    .text_sm()
                    .child(t!("TerminalCommandBar.no_quick_commands").to_string()),
            )
            .child(
                Button::new("terminal-command-quick-empty-add")
                    .label(t!("QuickCommand.add_command").to_string())
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.quick_command_panel.update(cx, |panel, cx| {
                            panel.open_add_command_dialog(window, cx);
                        });
                    })),
            )
            .into_any_element()
    }

    fn quick_command_list_items(
        &self,
        groups: Vec<QuickCommandGroup>,
    ) -> Vec<QuickCommandListItem> {
        let show_group_header = self.quick_group_filter == QuickGroupFilter::All;
        let item_count = groups
            .iter()
            .map(|group| group.commands.len())
            .sum::<usize>()
            + usize::from(show_group_header) * groups.len();
        let mut items = Vec::with_capacity(item_count);
        let mut command_offset = 0;

        for (group_index, group) in groups.into_iter().enumerate() {
            if show_group_header {
                items.push(QuickCommandListItem::Group {
                    group_index,
                    title: group
                        .name
                        .clone()
                        .unwrap_or_else(|| t!("TerminalCommandBar.ungrouped").to_string()),
                    color: group_color(group.color.as_deref(), self.colors.accent),
                });
            }
            let command_count = group.commands.len();
            items.extend(
                group
                    .commands
                    .into_iter()
                    .enumerate()
                    .map(|(command_index, command)| QuickCommandListItem::Command {
                        group_index,
                        command_index: command_offset + command_index,
                        command,
                    }),
            );
            command_offset += command_count;
        }

        items
    }

    fn render_quick_list_item(
        &self,
        item: QuickCommandListItem,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match item {
            QuickCommandListItem::Group {
                group_index,
                title,
                color,
            } => h_flex()
                .id(("terminal-quick-command-group", group_index))
                .w_full()
                .h(px(QUICK_GROUP_ROW_HEIGHT))
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
                )
                .into_any_element(),
            QuickCommandListItem::Command {
                group_index,
                command_index,
                command,
            } => self.render_quick_command((group_index, command_index), command, cx),
        }
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
        let value_for_copy = value.clone();
        let value_for_delete = value.clone();
        let existing_for_edit = command.clone();
        let id = command.id.unwrap_or(0);
        let is_pinned = command.pinned;
        let hover_group = SharedString::from(format!(
            "terminal-quick-command-hover-{group_index}-{command_index}"
        ));
        let label = command
            .name
            .clone()
            .filter(|name| name.trim() != value.trim());
        let row_height = QUICK_COMMAND_BASE_ROW_HEIGHT
            + (usize::from(label.is_some()) + usize::from(command.description.is_some())) as f32
                * QUICK_COMMAND_METADATA_ROW_HEIGHT;
        div()
            .w_full()
            .h(px(row_height))
            .px_2()
            .pb_1()
            .child(
                h_flex()
                    .id((
                        "terminal-quick-command-item",
                        group_index * QUICK_COMMAND_GROUP_ID_STRIDE + command_index,
                    ))
                    .size_full()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .group(hover_group.clone())
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
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_1()
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_1()
                                    .invisible()
                                    .group_hover(hover_group, |style| style.visible())
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .child(
                                        IconButton::new(
                                            SharedString::from(format!(
                                                "terminal-quick-command-pin-{id}"
                                            )),
                                            if is_pinned {
                                                IconName::StarOff
                                            } else {
                                                IconName::Star
                                            },
                                        )
                                        .role(IconButtonRole::Compact)
                                        .tooltip(if is_pinned {
                                            t!("QuickCommand.unpin_tooltip").to_string()
                                        } else {
                                            t!("QuickCommand.pin_tooltip").to_string()
                                        })
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.quick_command_panel.update(cx, |panel, cx| {
                                                    panel.toggle_command_pin(id, cx);
                                                });
                                            }),
                                        ),
                                    )
                                    .child(
                                        IconButton::new(
                                            SharedString::from(format!(
                                                "terminal-quick-command-edit-{id}"
                                            )),
                                            IconName::Edit,
                                        )
                                        .role(IconButtonRole::Compact)
                                        .tooltip(t!("QuickCommand.edit_tooltip").to_string())
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.quick_command_panel.update(cx, |panel, cx| {
                                                    panel.open_edit_command_dialog(
                                                        existing_for_edit.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            }),
                                        ),
                                    )
                                    .child(
                                        IconButton::new(
                                            SharedString::from(format!(
                                                "terminal-quick-command-copy-{id}"
                                            )),
                                            IconName::Copy,
                                        )
                                        .role(IconButtonRole::Compact)
                                        .tooltip(t!("QuickCommand.copy_tooltip").to_string())
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.quick_command_panel.update(cx, |panel, cx| {
                                                    panel.copy_command_to_clipboard(
                                                        &value_for_copy,
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            }),
                                        ),
                                    )
                                    .child(
                                        IconButton::new(
                                            SharedString::from(format!(
                                                "terminal-quick-command-delete-{id}"
                                            )),
                                            IconName::Remove,
                                        )
                                        .role(IconButtonRole::Compact)
                                        .text_color(cx.theme().danger)
                                        .tooltip(t!("QuickCommand.delete_tooltip").to_string())
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.quick_command_panel.update(cx, |panel, cx| {
                                                    panel.open_delete_command_dialog(
                                                        id,
                                                        value_for_delete.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            }),
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(self.colors.muted_foreground)
                                    .child(t!("TerminalCommandBar.fill").to_string()),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_quick_command_text(
        &self,
        label: Option<String>,
        command: QuickCommand,
    ) -> AnyElement {
        v_flex()
            .min_w_0()
            .flex_1()
            .gap_1()
            .child(
                div()
                    .truncate()
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .font_family("monospace")
                    .text_color(self.colors.foreground)
                    .child(command.command),
            )
            .when_some(label, |row, label| {
                row.child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(self.colors.accent)
                        .child(label),
                )
            })
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
