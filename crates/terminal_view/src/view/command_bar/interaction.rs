use super::*;
use crate::view::command_bar_model::{
    SelectionDirection, build_command_suggestions, command_inline_suffix, next_selection,
};
use gpui::{AppContext, Context, KeyDownEvent, Window};
use gpui_component::input::InputEvent;
use one_core::storage::{GlobalStorageState, QuickCommandRepository};
use terminal::terminal::TerminalConnectionKind;

const HISTORY_QUERY_LIMIT: usize = 16;

impl TerminalCommandBar {
    pub(in crate::view) fn new(
        config: TerminalCommandBarConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input_state = Self::create_input_state(window, cx);
        let quick_search_state = Self::create_quick_search_state(window, cx);
        let subscriptions = vec![
            Self::subscribe_input(&input_state, window, cx),
            Self::subscribe_quick_search(&quick_search_state, window, cx),
        ];
        let mut this = Self {
            terminal: config.terminal,
            connection_id: config.connection_id,
            input_state,
            quick_search_state,
            quick_group_scroll_handle: ScrollHandle::new(),
            quick_scroll_handle: ScrollHandle::new(),
            quick_commands: Vec::new(),
            suggestions: Vec::new(),
            selected_suggestion: None,
            quick_query: String::new(),
            quick_group_filter: QuickGroupFilter::default(),
            selected_quick_command: None,
            quick_commands_open: false,
            collapsed: false,
            autocomplete_enabled: true,
            colors: config.colors,
            _subscriptions: subscriptions,
        };
        this.load_quick_commands(cx);
        this
    }

    fn create_input_state(window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .auto_grow(2, 8)
                .placeholder(rust_i18n::t!("TerminalCommandBar.placeholder").to_string())
        })
    }

    fn subscribe_input(
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            input,
            window,
            |this, _state, event, window, cx| match event {
                InputEvent::Change => this.refresh_suggestions(cx),
                InputEvent::Focus => {
                    this.load_quick_commands(cx);
                    this.refresh_suggestions(cx);
                }
                InputEvent::PressEnter { secondary } if !secondary => this.submit(window, cx),
                _ => {}
            },
        )
    }

    pub(in crate::view) fn set_colors(&mut self, colors: TerminalColors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
    }

    pub(in crate::view) fn set_autocomplete_enabled(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.autocomplete_enabled = enabled;
        if enabled {
            self.refresh_suggestions(cx);
        } else {
            self.suggestions.clear();
            self.selected_suggestion = None;
            self.clear_inline_completion(cx);
        }
        cx.notify();
    }

    pub(super) fn load_quick_commands(&mut self, cx: &mut Context<Self>) {
        self.quick_commands = cx
            .try_global::<GlobalStorageState>()
            .and_then(|state| state.storage.get::<QuickCommandRepository>())
            .and_then(|repository| repository.list_by_connection(self.connection_id).ok())
            .unwrap_or_default();
    }

    pub(super) fn refresh_suggestions(&mut self, cx: &mut Context<Self>) {
        if !self.autocomplete_enabled {
            return;
        }
        let query = self.input_state.read(cx).value().to_string();
        let history = self
            .terminal
            .read(cx)
            .history_suggestions(&query, HISTORY_QUERY_LIMIT);
        self.suggestions = build_command_suggestions(&query, &self.quick_commands, &history);
        self.selected_suggestion = None;
        self.quick_commands_open = false;
        let inline_suffix = command_inline_suffix(&query, &self.suggestions);
        self.input_state.update(cx, |state, cx| {
            state.set_inline_completion_text(inline_suffix, cx);
        });
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self
            .selected_suggestion
            .and_then(|index| self.suggestions.get(index))
            .map(|item| item.command.clone());
        let command = selected.unwrap_or_else(|| self.input_state.read(cx).value().to_string());
        if command.trim().is_empty() {
            return;
        }
        self.input_state
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.suggestions.clear();
        self.selected_suggestion = None;
        self.clear_inline_completion(cx);
        cx.emit(TerminalCommandBarEvent::Submit(command));
        cx.notify();
    }

    pub(super) fn toggle_collapsed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.collapsed = !self.collapsed;
        self.reset_overlays(cx);
        if self.collapsed {
            cx.emit(TerminalCommandBarEvent::FocusTerminal);
        } else {
            self.focus_input(window, cx);
        }
        cx.notify();
    }

    pub(super) fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input_state
            .update(cx, |state, cx| state.focus(window, cx));
    }

    pub(super) fn target_label(&self, cx: &App) -> String {
        let terminal = self.terminal.read(cx);
        if let Some(name) = terminal.connection_name().filter(|name| !name.is_empty()) {
            return name.to_string();
        }
        match terminal.connection_kind() {
            TerminalConnectionKind::Local => rust_i18n::t!("TerminalCommandBar.local_shell"),
            TerminalConnectionKind::Ssh => rust_i18n::t!("TerminalCommandBar.remote_shell"),
            TerminalConnectionKind::Serial => rust_i18n::t!("TerminalCommandBar.serial_terminal"),
        }
        .to_string()
    }

    fn move_selection(&mut self, direction: SelectionDirection, cx: &mut Context<Self>) {
        self.selected_suggestion =
            next_selection(self.suggestions.len(), self.selected_suggestion, direction);
        cx.notify();
    }

    fn accept_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(command) = self
            .selected_suggestion
            .and_then(|index| self.suggestions.get(index))
            .map(|item| item.command.clone())
        else {
            return false;
        };
        self.input_state.update(cx, |state, cx| {
            set_command_input_value(state, command, window, cx);
        });
        self.suggestions.clear();
        self.selected_suggestion = None;
        self.clear_inline_completion(cx);
        cx.notify();
        true
    }

    pub(super) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.input_has_focus(window, cx) {
            return;
        }
        let handled = match event.keystroke.key.as_str() {
            "up" if !self.suggestions.is_empty() => {
                self.move_selection(SelectionDirection::Previous, cx);
                true
            }
            "down" if !self.suggestions.is_empty() => {
                self.move_selection(SelectionDirection::Next, cx);
                true
            }
            "tab" if !self.suggestions.is_empty() => self.accept_selection(window, cx),
            "escape" if !self.suggestions.is_empty() || self.quick_commands_open => {
                self.reset_overlays(cx);
                cx.notify();
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    fn input_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.input_state
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }

    pub(super) fn reset_overlays(&mut self, cx: &mut Context<Self>) {
        self.quick_commands_open = false;
        self.selected_quick_command = None;
        self.suggestions.clear();
        self.selected_suggestion = None;
        self.clear_inline_completion(cx);
    }

    pub(super) fn clear_inline_completion(&self, cx: &mut Context<Self>) {
        self.input_state.update(cx, |state, cx| {
            state.set_inline_completion_text(None, cx);
        });
    }
}
