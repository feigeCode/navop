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
use rust_i18n::t;
use terminal::history::normalize_recorded_command;

mod render;

const HISTORY_PANEL_LIMIT: usize = 200;

#[derive(Clone, Debug)]
pub enum HistoryCommandPanelEvent {
    ExecuteCommand(String),
}

pub struct HistoryCommandPanel {
    search_input_state: Entity<InputState>,
    scope: TerminalHistoryScope,
    history_user: Option<String>,
    commands: Vec<TerminalCommandHistory>,
    sort: TerminalCommandHistorySort,
    search_query: String,
    scroll_handle: UniformListScrollHandle,
    focus_handle: FocusHandle,
    colors: crate::theme::TerminalColors,
    /// 本会话没有读到 shell 历史（堡垒机/网络设备常见），空状态要说明原因。
    remote_history_unavailable: bool,
    _subscriptions: Vec<gpui::Subscription>,
}

impl HistoryCommandPanel {
    pub fn new(
        scope: TerminalHistoryScope,
        history_user: Option<String>,
        colors: crate::theme::TerminalColors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input_state = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("HistoryCommand.search").to_string())
        });
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
            history_user,
            commands: Vec::new(),
            sort: TerminalCommandHistorySort::Latest,
            search_query: String::new(),
            scroll_handle: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            colors,
            remote_history_unavailable: false,
            _subscriptions: vec![subscription],
        };
        panel.load_commands(cx);
        panel
    }

    pub fn set_colors(&mut self, colors: crate::theme::TerminalColors, cx: &mut Context<Self>) {
        self.colors = colors;
        cx.notify();
    }

    pub fn refresh_commands(&mut self, cx: &mut Context<Self>) {
        self.load_commands(cx);
    }

    pub fn set_remote_history_unavailable(&mut self, unavailable: bool, cx: &mut Context<Self>) {
        if self.remote_history_unavailable != unavailable {
            self.remote_history_unavailable = unavailable;
            cx.notify();
        }
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
            Ok(commands) => {
                self.commands = normalize_history_commands(commands, self.history_user.as_deref())
            }
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

    fn delete_command(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(repo) = self.repository(cx) else {
            return;
        };
        if let Err(error) = repo.delete_command(id) {
            tracing::warn!(%error, "failed to delete terminal command history");
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

fn normalize_history_commands(
    commands: Vec<TerminalCommandHistory>,
    history_user: Option<&str>,
) -> Vec<TerminalCommandHistory> {
    let mut seen = std::collections::HashSet::new();
    let mut normalized = Vec::new();
    for mut item in commands {
        if let Some(command) = normalize_recorded_command(&item.command, history_user) {
            item.command = command;
        }
        if seen.insert(item.command.clone()) {
            normalized.push(item);
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::normalize_history_commands;
    use one_core::storage::TerminalCommandHistory;

    #[test]
    fn normalize_history_commands_strips_recorded_prefix_for_display() {
        let commands = normalize_history_commands(
            vec![item(1, "2026-07-05 08:07:16 root cd /data/Seeyon/Comi")],
            Some("root"),
        );

        assert_eq!("cd /data/Seeyon/Comi", commands[0].command);
    }

    #[test]
    fn normalize_history_commands_deduplicates_cleaned_commands() {
        let commands = normalize_history_commands(
            vec![
                item(1, "2026-07-05 08:07:16 root cd /data/app"),
                item(2, "cd /data/app"),
            ],
            Some("root"),
        );

        assert_eq!(1, commands.len());
        assert_eq!("cd /data/app", commands[0].command);
    }

    fn item(id: i64, command: &str) -> TerminalCommandHistory {
        TerminalCommandHistory {
            id: Some(id),
            scope_key: "ssh:1".to_string(),
            scope_kind: "ssh".to_string(),
            connection_id: Some(1),
            command: command.to_string(),
            use_count: 1,
            favorite: false,
            first_used_at: Some(1),
            last_used_at: Some(1),
            last_exit_code: Some(0),
            cwd: None,
            created_at: Some(1),
            updated_at: Some(1),
        }
    }

    #[test]
    fn empty_state_explains_when_the_session_has_no_shell_history() {
        // issue #570：堡垒机读不到历史时，面板不能只显示「还没有命令」，
        // 还要告诉用户原因（内联文案，不弹浮层）。
        let render = include_str!("history_command_panel/render.rs");
        assert!(render.contains("remote_history_unavailable"));
        assert!(render.contains("HistoryCommand.remote_history_unavailable"));
    }
}
