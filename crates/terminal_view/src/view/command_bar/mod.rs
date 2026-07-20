use super::command_bar_model::CommandSuggestion;
use crate::theme::TerminalColors;
use gpui::{App, Entity, EventEmitter, FocusHandle, Focusable, Subscription};
use gpui_component::input::InputState;
use one_core::storage::QuickCommand;
use terminal::terminal::Terminal;

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
    PasteTerminal(String),
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
    quick_commands: Vec<QuickCommand>,
    suggestions: Vec<CommandSuggestion>,
    selected_suggestion: Option<usize>,
    quick_query: String,
    quick_group_filter: QuickGroupFilter,
    selected_quick_command: Option<usize>,
    quick_commands_open: bool,
    collapsed: bool,
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
