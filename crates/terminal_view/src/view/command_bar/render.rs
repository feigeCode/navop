use super::*;
use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, AppContext, Context, DragMoveEvent, EntityId, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Icon, Selectable, Sizable,
    button::{Button, ButtonCustomVariant, ButtonVariants},
    h_flex,
    input::Textarea,
};
use one_assets::IconName;
use rust_i18n::t;

const COMMAND_BAR_COLLAPSED_HEIGHT: f32 = 30.0;
const COMMAND_BAR_INPUT_MIN_HEIGHT: f32 = 80.0;
const COMMAND_BAR_INPUT_MAX_HEIGHT: f32 = 400.0;
const COMMAND_BAR_RESIZE_HANDLE_HEIGHT: f32 = 6.0;
const COMMAND_BAR_RESIZE_GRIP_WIDTH: f32 = 32.0;
const COMMAND_BAR_RESIZE_GRIP_HOVER_WIDTH: f32 = 48.0;
const COMMAND_BAR_RESIZE_GRIP_HEIGHT: f32 = 2.0;
const COMMAND_BAR_POPOVER_GAP: f32 = 8.0;

/// 弹层底边偏移：命令栏主体高度 + 间距。
pub(super) fn popover_offset_for(collapsed: bool, input_height: f32) -> f32 {
    let body_height = if collapsed {
        COMMAND_BAR_COLLAPSED_HEIGHT
    } else {
        input_height
    };
    body_height + COMMAND_BAR_POPOVER_GAP
}

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
        popover_offset_for(self.collapsed, self.input_height)
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
                    .w(px(COMMAND_BAR_RESIZE_GRIP_WIDTH))
                    .h(px(COMMAND_BAR_RESIZE_GRIP_HEIGHT))
                    .rounded_full()
                    .bg(self.colors.border)
                    .group_hover("terminal-command-resize-handle", |handle| {
                        handle
                            .w(px(COMMAND_BAR_RESIZE_GRIP_HOVER_WIDTH))
                            .bg(cx.theme().drag_border)
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

    /// 折叠态这一行就是置顶快捷指令横条：左侧滚动展示 chips，右侧固定
    /// 「快捷指令」与「展开命令输入」两个图标按钮。没有置顶指令时左侧留空，
    /// 避免把入口挤到左边。
    fn render_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .h(px(COMMAND_BAR_COLLAPSED_HEIGHT))
            .items_center()
            .gap_1()
            .child(self.render_pinned_quick_chips(cx))
            .child(self.render_quick_command_icon_button(cx))
            .child(self.render_command_input_toggle_button(cx))
            .into_any_element()
    }

    /// 折叠态右侧的快捷指令入口：只显示图标，完整弹层里再做搜索、分组与编辑。
    fn render_quick_command_icon_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("terminal-command-quick")
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(Icon::new(IconName::QuickCommands).xsmall())
                    .when(!self.quick_commands.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(self.colors.muted_foreground)
                                .child(self.quick_commands.len().to_string()),
                        )
                    }),
            )
            .custom(
                ButtonCustomVariant::new(cx)
                    .foreground(self.colors.foreground)
                    .hover(self.colors.muted)
                    .active(self.colors.muted),
            )
            .selected(self.quick_commands_open)
            .small()
            .tooltip(if self.quick_commands_open {
                t!("TerminalCommandBar.close_quick_commands").to_string()
            } else {
                t!("TerminalCommandBar.open_quick_commands").to_string()
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_quick_commands(window, cx);
            }))
            .into_any_element()
    }

    /// 展开 / 收起命令输入：入口固定在折叠行右侧，展开后同一按钮移到输入框
    /// 右侧，位置保持一致。
    fn render_command_input_toggle_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let chevron = if self.collapsed {
            IconName::ChevronUp
        } else {
            IconName::ChevronDown
        };
        Button::new("terminal-command-input-toggle")
            .child(
                h_flex()
                    .items_center()
                    .gap_0p5()
                    .child(Icon::new(IconName::SquareTerminal).xsmall())
                    .child(Icon::new(chevron).xsmall()),
            )
            .custom(
                ButtonCustomVariant::new(cx)
                    .foreground(self.colors.foreground)
                    .hover(self.colors.muted)
                    .active(self.colors.muted),
            )
            .selected(!self.collapsed)
            .small()
            .tooltip(if self.collapsed {
                t!("TerminalCommandBar.expand").to_string()
            } else {
                t!("TerminalCommandBar.collapse").to_string()
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_collapsed(window, cx);
            }))
            .into_any_element()
    }

    fn render_expanded_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap_1()
            .child(self.render_quick_command_icon_button(cx))
            .child(self.render_command_input_toggle_button(cx))
            .into_any_element()
    }

    fn render_input_row(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .h(px(self.input_height))
            .min_h(px(COMMAND_BAR_INPUT_MIN_HEIGHT))
            .min_w_0()
            .items_start()
            .gap_1()
            .py_1()
            .child(
                div().flex_1().min_w_0().h(px(self.input_height)).child(
                    Textarea::new(&self.input_state)
                        .appearance(false)
                        .h(px(self.input_height))
                        .w_full()
                        .text_color(self.colors.foreground),
                ),
            )
            .child(
                div()
                    .flex_shrink_0()
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
            .on_action(cx.listener(Self::handle_history_previous))
            .on_action(cx.listener(Self::handle_history_next))
            .on_key_down(cx.listener(Self::handle_key_down))
            .when(
                focused
                    && !self.collapsed
                    && self.suggestion_popup_enabled
                    && !self.suggestions.is_empty(),
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
