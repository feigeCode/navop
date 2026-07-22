use super::command_bar_model::CommandSuggestion;
use crate::theme::TerminalColors;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle, Subscription, Window,
};
use gpui_component::input::InputState;
use gpui_component::{RopeExt as _, VirtualListScrollHandle};
use one_core::storage::QuickCommand;
use terminal::terminal::Terminal;

pub(super) const COMMAND_BAR_INPUT_DEFAULT_HEIGHT: f32 = 80.0;

mod interaction;
mod quick_interaction;
mod quick_render;
mod quick_render_list;
mod quick_render_sidebar;
mod render;
mod suggestion_render;

#[derive(Clone, Debug)]
pub(super) enum TerminalCommandBarEvent {
    Submit(String),
    FocusTerminal,
}

pub(super) struct TerminalCommandBarConfig {
    pub terminal: Entity<Terminal>,
    pub connection_id: Option<i64>,
    pub colors: TerminalColors,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum QuickGroupFilter {
    #[default]
    All,
    Ungrouped,
    Group(String),
}

pub(super) struct TerminalCommandBar {
    terminal: Entity<Terminal>,
    connection_id: Option<i64>,
    input_state: Entity<InputState>,
    quick_search_state: Entity<InputState>,
    quick_group_scroll_handle: ScrollHandle,
    quick_scroll_handle: VirtualListScrollHandle,
    quick_commands: Vec<QuickCommand>,
    suggestions: Vec<CommandSuggestion>,
    selected_suggestion: Option<usize>,
    quick_query: String,
    quick_group_filter: QuickGroupFilter,
    selected_quick_command: Option<usize>,
    quick_commands_open: bool,
    collapsed: bool,
    input_height: f32,
    autocomplete_enabled: bool,
    colors: TerminalColors,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TerminalCommandBarEvent> for TerminalCommandBar {}

impl Focusable for TerminalCommandBar {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input_state.read(cx).focus_handle(cx)
    }
}

fn set_command_input_value(
    state: &mut InputState,
    command: String,
    window: &mut Window,
    cx: &mut Context<InputState>,
) {
    let end_offset = command.len();
    state.set_value(command, window, cx);
    let end_position = state.text().offset_to_position(end_offset);
    state.set_cursor_position(end_position, window, cx);
}
