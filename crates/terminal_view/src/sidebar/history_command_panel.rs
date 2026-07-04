//! 历史命令面板。

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    UniformListScrollHandle, Window,
};
use gpui_component::input::{InputEvent, InputState};
use one_core::storage::{
    GlobalStorageState, TerminalCommandHistory, TerminalCommandHistoryRepository,
    TerminalCommandHistorySort, TerminalHistoryScope,
};

mod render;

const HISTORY_PANEL_LIMIT: usize = 200;

#[derive(Clone, Debug)]
pub enum HistoryCommandPanelEvent {
    ExecuteCommand(String),
}

pub struct HistoryCommandPanel {
    search_input_state: Entity<InputState>,
    scope: TerminalHistoryScope,
    commands: Vec<TerminalCommandHistory>,
    sort: TerminalCommandHistorySort,
    search_query: String,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    colors: crate::theme::TerminalColors,
    _subscriptions: Vec<gpui::Subscription>,
}

impl HistoryCommandPanel {
    pub fn new(
        scope: TerminalHistoryScope,
        colors: crate::theme::TerminalColors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input_state = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let input_entity = search_input_state.clone();
        let subscription = cx.subscribe_in(
            &search_input_state,
            window,
            move |this, _state, event, _window, cx| {
                if let InputEvent::Change = event {
                    this.search_query = input_entity.read(cx).value().to_string();
                    this.load_commands(cx);
                }
            },
        );

        let mut panel = Self {
            search_input_state,
            scope,
            commands: Vec::new(),
            sort: TerminalCommandHistorySort::Latest,
            search_query: String::new(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            colors,
            _subscriptions: vec![subscription],
        };
        panel.load_commands(cx);
        panel
    }

    pub fn set_colors(&mut self, colors: crate::theme::TerminalColors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
    }

    fn repository(&self, cx: &App) -> Option<std::sync::Arc<TerminalCommandHistoryRepository>> {
        cx.try_global::<GlobalStorageState>()
            .and_then(|state| state.storage.get::<TerminalCommandHistoryRepository>())
    }

    fn load_commands(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repository(cx) else {
            tracing::warn!("TerminalCommandHistoryRepository not found");
            self.commands.clear();
            cx.notify();
            return;
        };
        let query = (!self.search_query.trim().is_empty()).then_some(self.search_query.as_str());
        match repo.list(&self.scope, self.sort, query, HISTORY_PANEL_LIMIT) {
            Ok(commands) => self.commands = commands,
            Err(error) => {
                tracing::warn!(%error, "failed to load terminal command history");
                self.commands.clear();
            }
        }
        cx.notify();
    }

    fn set_sort(&mut self, sort: TerminalCommandHistorySort, cx: &mut Context<Self>) {
        if self.sort == sort {
            return;
        }
        self.sort = sort;
        self.load_commands(cx);
    }

    fn toggle_favorite(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(repo) = self.repository(cx) else {
            return;
        };
        if let Err(error) = repo.toggle_favorite(id) {
            tracing::warn!(%error, "failed to toggle terminal command favorite");
            return;
        }
        self.load_commands(cx);
    }

    fn paste_command(&self, command: String, cx: &mut Context<Self>) {
        cx.emit(HistoryCommandPanelEvent::ExecuteCommand(command));
    }
}

impl EventEmitter<HistoryCommandPanelEvent> for HistoryCommandPanel {}

impl Focusable for HistoryCommandPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
