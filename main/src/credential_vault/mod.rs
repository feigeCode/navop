mod actions;
mod form;
mod form_render;
mod form_window;
mod render;

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Window,
};
use gpui_component::{
    Icon, IconName,
    input::{InputEvent, InputState},
};
use one_core::{
    crypto,
    storage::{CredentialRepository, CredentialSummary, GlobalStorageState, StorageManager},
    tab_container::{TabContent, TabContentEvent},
};
use rust_i18n::t;

pub(crate) struct CredentialVaultView {
    focus_handle: FocusHandle,
    storage_manager: StorageManager,
    summaries: Vec<CredentialSummary>,
    search_input: Entity<InputState>,
    load_error: Option<String>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl CredentialVaultView {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let storage_manager = cx.global::<GlobalStorageState>().storage.clone();
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("CredentialVault.search_placeholder").to_string())
        });
        let subscription = cx.subscribe(&search_input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let mut view = Self {
            focus_handle: cx.focus_handle(),
            storage_manager,
            summaries: Vec::new(),
            search_input,
            load_error: None,
            _subscriptions: vec![subscription],
        };
        view.reload(cx);
        view
    }

    fn repository(&self) -> Result<std::sync::Arc<CredentialRepository>, String> {
        self.storage_manager
            .get::<CredentialRepository>()
            .ok_or_else(|| t!("CredentialVault.repository_unavailable").to_string())
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let result = self
            .repository()
            .and_then(|repository| repository.list_summaries().map_err(|e| e.to_string()));
        match result {
            Ok(summaries) => {
                self.summaries = summaries;
                self.load_error = None;
            }
            Err(error) => self.load_error = Some(error),
        }
        cx.notify();
    }

    fn filtered_summaries(&self, cx: &App) -> Vec<CredentialSummary> {
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        if query.is_empty() {
            return self.summaries.clone();
        }
        self.summaries
            .iter()
            .filter(|summary| summary_matches(summary, &query))
            .cloned()
            .collect()
    }
}

impl Focusable for CredentialVaultView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for CredentialVaultView {}

impl TabContent for CredentialVaultView {
    fn content_key(&self) -> &'static str {
        "CredentialVault"
    }

    fn title(&self, _cx: &App) -> SharedString {
        t!("CredentialVault.title").into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::Key.color())
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn can_rename(&self, _cx: &App) -> bool {
        false
    }
}

fn summary_matches(summary: &CredentialSummary, query: &str) -> bool {
    summary.name.to_lowercase().contains(query)
        || summary
            .username
            .as_deref()
            .is_some_and(|username| username.to_lowercase().contains(query))
}

pub(super) fn vault_unlocked() -> bool {
    crypto::has_master_key()
}

pub(super) fn button_id(prefix: &str, id: i64) -> SharedString {
    format!("{prefix}-{id}").into()
}
