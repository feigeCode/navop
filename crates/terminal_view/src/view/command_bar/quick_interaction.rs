use super::*;
use crate::view::command_bar_model::{
    SelectionDirection, bounded_selection, selected_quick_command,
};
use gpui::{AppContext, Context, KeyDownEvent, Window};
use gpui_component::input::InputEvent;

impl TerminalCommandBar {
    pub(super) fn create_quick_search_state(
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("TerminalCommandBar.search_quick_commands").to_string())
        })
    }

    pub(super) fn subscribe_quick_search(
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        let input = input.clone();
        cx.subscribe_in(
            &input.clone(),
            window,
            move |this, _state, event, window, cx| match event {
                InputEvent::Change => {
                    this.quick_query = input.read(cx).value().to_string();
                    this.selected_quick_command = None;
                    this.quick_scroll_handle = VirtualListScrollHandle::new();
                    cx.notify();
                }
                InputEvent::PressEnter { secondary } if !secondary => {
                    this.choose_selected_quick_command(window, cx);
                }
                _ => {}
            },
        )
    }

    pub(super) fn toggle_quick_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_commands_open {
            self.close_quick_commands(window, cx);
            return;
        }
        self.load_quick_commands(cx);
        self.quick_commands_open = true;
        self.suggestions.clear();
        self.selected_suggestion = None;
        self.clear_inline_completion(cx);
        self.selected_quick_command = None;
        self.quick_group_filter = QuickGroupFilter::All;
        self.quick_group_scroll_handle = ScrollHandle::new();
        self.quick_scroll_handle = VirtualListScrollHandle::new();
        self.quick_search_state.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        let quick_search = self.quick_search_state.clone();
        window.defer(cx, move |window, cx| {
            quick_search.update(cx, |state, cx| state.focus(window, cx));
        });
        cx.notify();
    }

    pub(super) fn choose_command(
        &mut self,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.collapsed {
            self.reset_overlays(cx);
            cx.emit(TerminalCommandBarEvent::InputToPty(command));
            cx.notify();
            return;
        }
        self.input_state.update(cx, |state, cx| {
            set_command_input_value(state, command, window, cx);
        });
        self.reset_overlays(cx);
        cx.notify();
    }

    pub(super) fn select_quick_group(&mut self, filter: QuickGroupFilter, cx: &mut Context<Self>) {
        self.quick_group_filter = filter;
        self.selected_quick_command = None;
        self.quick_scroll_handle = VirtualListScrollHandle::new();
        cx.notify();
    }

    pub(super) fn handle_quick_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.quick_search_has_focus(window, cx) {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control {
            return;
        }
        let handled = match event.keystroke.key.as_str() {
            "up" | "arrowup" => self.move_quick_selection(SelectionDirection::Previous, cx),
            "down" | "arrowdown" => self.move_quick_selection(SelectionDirection::Next, cx),
            "home" => self.select_quick_boundary(false, cx),
            "end" => self.select_quick_boundary(true, cx),
            "escape" => {
                self.close_quick_commands(window, cx);
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    fn move_quick_selection(
        &mut self,
        direction: SelectionDirection,
        cx: &mut Context<Self>,
    ) -> bool {
        let count = self.visible_quick_commands().len();
        self.selected_quick_command =
            bounded_selection(count, self.selected_quick_command, direction);
        cx.notify();
        count > 0
    }

    fn select_quick_boundary(&mut self, last: bool, cx: &mut Context<Self>) -> bool {
        let count = self.visible_quick_commands().len();
        self.selected_quick_command = count.checked_sub(1).map(|end| if last { end } else { 0 });
        cx.notify();
        count > 0
    }

    fn choose_selected_quick_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commands = self.visible_quick_commands();
        if let Some(command) = selected_quick_command(&commands, self.selected_quick_command) {
            self.choose_command(command, window, cx);
        }
    }

    fn visible_quick_commands(&self) -> Vec<QuickCommand> {
        self.filtered_quick_groups()
            .into_iter()
            .flat_map(|group| group.commands)
            .collect()
    }

    pub(super) fn close_quick_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_commands_open = false;
        self.selected_quick_command = None;
        if self.collapsed {
            cx.emit(TerminalCommandBarEvent::FocusTerminal);
            cx.notify();
        } else {
            self.focus_input(window, cx);
            self.refresh_suggestions(cx);
        }
    }

    fn quick_search_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.quick_search_state
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }
}
