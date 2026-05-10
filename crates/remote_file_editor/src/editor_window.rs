use crate::file_policy::{
    EditorMode, FilePolicy, MAX_EDITABLE_FILE_SIZE, decode_text_content, determine_file_policy,
};
use crate::language::language_for_path;
use crate::{
    CloseIntercept, active_index_after_close, active_index_after_open, decide_close_intercept,
};
use gpui::{
    AnyWindowHandle, App, AppContext, Context, Entity, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement, PromptLevel, Render, Styled, WeakEntity, Window, actions, div, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, Size, TitleBar, WindowExt,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState, Search},
    notification::Notification,
    tab::{Tab, TabBar},
    v_flex,
};
use one_core::{
    gpui_tokio::Tokio,
    popup_window::{PopupWindowOptions, open_popup_window},
};
use rust_i18n::t;
use sftp::{RusshSftpClient, SftpClient};
use std::sync::{Arc, Mutex as StdMutex, Once, OnceLock};
use tokio::sync::Mutex;

actions!(remote_file_editor, [OpenSearch, OpenReplace]);

const REMOTE_FILE_EDITOR_CONTEXT: &str = "RemoteFileEditor";
#[cfg(target_os = "macos")]
const REMOTE_EDITOR_SEARCH_SHORTCUT: &str = "cmd-f";
#[cfg(not(target_os = "macos"))]
const REMOTE_EDITOR_SEARCH_SHORTCUT: &str = "ctrl-f";
#[cfg(target_os = "macos")]
const REMOTE_EDITOR_REPLACE_SHORTCUT: &str = "cmd-r";
#[cfg(not(target_os = "macos"))]
const REMOTE_EDITOR_REPLACE_SHORTCUT: &str = "ctrl-r";

static REMOTE_EDITOR_KEYBINDINGS_INIT: Once = Once::new();
static REMOTE_EDITOR_WINDOW: OnceLock<StdMutex<Option<RemoteEditorWindowRef>>> = OnceLock::new();

#[derive(Clone)]
struct RemoteEditorWindowRef {
    window: AnyWindowHandle,
    view: WeakEntity<RemoteFileEditorWindow>,
}

pub fn open_remote_file_editor<T: 'static>(
    remote_path: String,
    client: Arc<Mutex<RusshSftpClient>>,
    cx: &mut Context<T>,
) {
    init_keybindings(cx);
    cx.spawn(async move |_this, cx| {
        let remote_path_for_log = remote_path.clone();
        let result = cx.update(|cx| {
            if open_in_existing_window(remote_path.clone(), cx)? {
                return Ok(());
            }

            let title = editor_window_title(&remote_path);
            open_popup_window(
                PopupWindowOptions::new(title).size(960.0, 720.0).min_width(640.0).min_height(480.0),
                move |window, cx| {
                    let view = cx.new(|cx| {
                        RemoteFileEditorWindow::new(remote_path, client, window, cx)
                    });
                    set_editor_window(RemoteEditorWindowRef {
                        window: window.window_handle(),
                        view: view.downgrade(),
                    });
                    view
                },
                cx,
            );

            Ok::<_, anyhow::Error>(())
        });

        if let Err(error) = result {
            tracing::error!(path = %remote_path_for_log, ?error, "failed to open remote file editor");
        }
    })
    .detach();
}

fn open_in_existing_window(remote_path: String, cx: &mut App) -> anyhow::Result<bool> {
    let Some(editor_window) = current_editor_window() else {
        return Ok(false);
    };

    let result = cx.update_window(editor_window.window, |_, window, cx| {
        window.activate_window();
        editor_window
            .view
            .update(cx, |this, cx| {
                this.open_or_focus_tab(remote_path, window, cx);
            })
            .is_ok()
    });

    match result {
        Ok(true) => Ok(true),
        Ok(false) | Err(_) => {
            clear_editor_window();
            Ok(false)
        }
    }
}

fn editor_window_slot() -> &'static StdMutex<Option<RemoteEditorWindowRef>> {
    REMOTE_EDITOR_WINDOW.get_or_init(|| StdMutex::new(None))
}

fn current_editor_window() -> Option<RemoteEditorWindowRef> {
    editor_window_slot().lock().ok()?.clone()
}

fn set_editor_window(window: RemoteEditorWindowRef) {
    if let Ok(mut slot) = editor_window_slot().lock() {
        *slot = Some(window);
    }
}

fn clear_editor_window() {
    if let Ok(mut slot) = editor_window_slot().lock() {
        *slot = None;
    }
}

fn init_keybindings(cx: &mut App) {
    REMOTE_EDITOR_KEYBINDINGS_INIT.call_once(|| {
        cx.bind_keys([
            KeyBinding::new(
                search_shortcut(),
                OpenSearch,
                Some(REMOTE_FILE_EDITOR_CONTEXT),
            ),
            KeyBinding::new(
                replace_shortcut(),
                OpenReplace,
                Some(REMOTE_FILE_EDITOR_CONTEXT),
            ),
        ]);
    });
}

fn search_shortcut() -> &'static str {
    REMOTE_EDITOR_SEARCH_SHORTCUT
}

fn replace_shortcut() -> &'static str {
    REMOTE_EDITOR_REPLACE_SHORTCUT
}

fn editor_window_title(remote_path: &str) -> String {
    t!(
        "RemoteFileEditor.title",
        name = display_name_from_path(remote_path)
    )
    .to_string()
}

struct LoadedFile {
    text: String,
    policy: FilePolicy,
    file_size: usize,
    language: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingCloseAction {
    Window,
    Tab(usize),
}

struct RemoteEditorTab {
    id: u64,
    remote_path: String,
    display_name: String,
    editor: Option<Entity<InputState>>,
    subscriptions: Vec<gpui::Subscription>,
    saved_text: String,
    file_size: usize,
    policy: FilePolicy,
    loading: bool,
    saving: bool,
    soft_wrap: bool,
    status_message: String,
    load_error: Option<String>,
}

impl RemoteEditorTab {
    fn new(id: u64, remote_path: String) -> Self {
        Self {
            id,
            display_name: display_name_from_path(&remote_path),
            remote_path,
            editor: None,
            subscriptions: Vec::new(),
            saved_text: String::new(),
            file_size: 0,
            policy: FilePolicy {
                mode: EditorMode::Code,
                is_large_file: false,
            },
            loading: true,
            saving: false,
            soft_wrap: false,
            status_message: t!("RemoteFileEditor.status.loading").to_string(),
            load_error: None,
        }
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.editor
            .as_ref()
            .map(|editor| editor.read(cx).text() != self.saved_text.as_str())
            .unwrap_or(false)
    }

    fn policy_label(&self) -> String {
        match self.policy.mode {
            EditorMode::Code => t!("RemoteFileEditor.policy.code").to_string(),
            EditorMode::PlainText => t!("RemoteFileEditor.policy.plain_text").to_string(),
        }
    }
}

struct RemoteFileEditorWindow {
    client: Arc<Mutex<RusshSftpClient>>,
    tabs: Vec<RemoteEditorTab>,
    active_tab: usize,
    close_prompt_open: bool,
    pending_close_action: Option<PendingCloseAction>,
    close_window_after_saves: bool,
    next_tab_id: u64,
}

impl RemoteFileEditorWindow {
    fn new(
        remote_path: String,
        client: Arc<Mutex<RusshSftpClient>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            client,
            tabs: Vec::new(),
            active_tab: 0,
            close_prompt_open: false,
            pending_close_action: None,
            close_window_after_saves: false,
            next_tab_id: 1,
        };
        this.register_close_guard(window, cx);
        this.open_or_focus_tab(remote_path, window, cx);
        this
    }

    fn register_close_guard(&self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            view.update(cx, |this, cx| this.handle_window_should_close(window, cx))
                .unwrap_or(true)
        });
    }

    fn open_or_focus_tab(
        &mut self,
        remote_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = self.tab_paths();
        let active_index = active_index_after_open(&paths, &remote_path);
        if active_index == self.tabs.len() {
            let tab_id = self.next_tab_id;
            self.next_tab_id += 1;
            self.tabs.push(RemoteEditorTab::new(tab_id, remote_path));
            self.active_tab = active_index;
            self.reload_tab(active_index, window, cx);
        } else {
            self.active_tab = active_index;
            self.focus_editor(window, cx);
            cx.notify();
        }
        self.update_window_title(window);
    }

    fn tab_paths(&self) -> Vec<String> {
        self.tabs
            .iter()
            .map(|tab| tab.remote_path.clone())
            .collect()
    }

    fn tab_index_by_identity(&self, tab_id: u64, remote_path: &str) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.id == tab_id && tab.remote_path == remote_path)
    }

    fn active_tab(&self) -> Option<&RemoteEditorTab> {
        self.tabs.get(self.active_tab)
    }

    fn active_tab_mut(&mut self) -> Option<&mut RemoteEditorTab> {
        self.tabs.get_mut(self.active_tab)
    }

    fn update_window_title(&self, window: &mut Window) {
        if let Some(tab) = self.active_tab() {
            window.set_window_title(&editor_window_title(&tab.remote_path));
        }
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_tab(self.active_tab, window, cx);
    }

    fn reload_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };

        tab.loading = true;
        tab.load_error = None;
        tab.status_message = t!("RemoteFileEditor.status.loading").to_string();
        cx.notify();

        let tab_id = tab.id;
        let remote_path = tab.remote_path.clone();
        let task_remote_path = remote_path.clone();
        let client = self.client.clone();
        let task = Tokio::spawn(cx, async move {
            let bytes = {
                let mut client = client.lock().await;
                client
                    .read_file(&task_remote_path, MAX_EDITABLE_FILE_SIZE)
                    .await?
            };
            let file_size = bytes.len();
            let policy = determine_file_policy(file_size)?;
            let text = decode_text_content(&bytes)?;
            let language = language_for_path(&task_remote_path, policy.is_large_file).to_string();
            Ok::<_, anyhow::Error>(LoadedFile {
                text,
                policy,
                file_size,
                language,
            })
        });

        let view = cx.entity().clone();
        window
            .spawn(cx, async move |cx| match task.await {
                Ok(Ok(loaded)) => {
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.apply_loaded_file(tab_id, &remote_path, loaded, window, cx);
                    });
                }
                Ok(Err(error)) => {
                    let message = error.to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.apply_load_error(tab_id, &remote_path, message.clone(), cx);
                        window.push_notification(Notification::error(message), cx);
                    });
                }
                Err(error) => {
                    let message = error.to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.apply_load_error(tab_id, &remote_path, message.clone(), cx);
                        window.push_notification(Notification::error(message), cx);
                    });
                }
            })
            .detach();
    }

    fn apply_loaded_file(
        &mut self,
        tab_id: u64,
        remote_path: &str,
        loaded: LoadedFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index_by_identity(tab_id, remote_path) else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        let LoadedFile {
            text,
            policy,
            file_size,
            language,
        } = loaded;

        let initial_text = text.clone();
        let soft_wrap = tab.soft_wrap;
        let editor = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .code_editor(language)
                .line_number(true)
                .searchable(true)
                .soft_wrap(soft_wrap);
            state.set_value(initial_text, window, cx);
            state
        });

        tab.subscriptions.clear();
        tab.subscriptions.push(
            cx.subscribe(&editor, |_this, _input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        );

        if index == self.active_tab {
            editor.update(cx, |state: &mut InputState, cx| {
                state.focus(window, cx);
            });
        }

        tab.editor = Some(editor);
        tab.saved_text = text;
        tab.file_size = file_size;
        tab.policy = policy;
        tab.loading = false;
        tab.saving = false;
        tab.load_error = None;
        tab.status_message = if policy.is_large_file {
            t!("RemoteFileEditor.status.loaded_plain_text").to_string()
        } else {
            t!("RemoteFileEditor.status.loaded").to_string()
        };
        cx.notify();
    }

    fn apply_load_error(
        &mut self,
        tab_id: u64,
        remote_path: &str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index_by_identity(tab_id, remote_path) else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        tab.loading = false;
        tab.load_error = Some(message);
        tab.status_message = t!("RemoteFileEditor.status.load_failed").to_string();
        cx.notify();
    }

    fn save(&mut self, close_after_save: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.save_tab(self.active_tab, close_after_save, window, cx);
    }

    fn save_tab(
        &mut self,
        index: usize,
        close_after_save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        let Some(editor) = tab.editor.clone() else {
            if close_after_save {
                self.close_clean_tab(index, window, cx);
            }
            return;
        };

        if tab.saving {
            return;
        }

        let text = editor.read(cx).text().to_string();
        tab.saving = true;
        tab.status_message = t!("RemoteFileEditor.status.saving").to_string();
        cx.notify();

        let tab_id = tab.id;
        let remote_path = tab.remote_path.clone();
        let task_remote_path = remote_path.clone();
        let client = self.client.clone();
        let task = Tokio::spawn(cx, async move {
            let mut client = client.lock().await;
            client
                .write_file(&task_remote_path, text.as_bytes())
                .await?;
            Ok::<_, anyhow::Error>(text)
        });

        let view = cx.entity().clone();
        window
            .spawn(cx, async move |cx| match task.await {
                Ok(Ok(saved_text)) => {
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.apply_saved_file(
                            tab_id,
                            &remote_path,
                            saved_text,
                            close_after_save,
                            window,
                            cx,
                        );
                    });
                }
                Ok(Err(error)) => {
                    let message = error.to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.apply_save_error(tab_id, &remote_path, message.clone(), cx);
                        window.push_notification(Notification::error(message), cx);
                    });
                }
                Err(error) => {
                    let message = error.to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.apply_save_error(tab_id, &remote_path, message.clone(), cx);
                        window.push_notification(Notification::error(message), cx);
                    });
                }
            })
            .detach();
    }

    fn apply_saved_file(
        &mut self,
        tab_id: u64,
        remote_path: &str,
        saved_text: String,
        close_after_save: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index_by_identity(tab_id, remote_path) else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        tab.saved_text = saved_text;
        tab.file_size = tab.saved_text.len();
        tab.saving = false;
        tab.status_message = t!("RemoteFileEditor.status.saved").to_string();

        if self.close_window_after_saves && !self.has_dirty_tabs(cx) {
            self.close_window_after_saves = false;
            clear_editor_window();
            window.remove_window();
        } else if close_after_save {
            self.close_clean_tab(index, window, cx);
        } else {
            window.push_notification(
                Notification::success(t!("RemoteFileEditor.notification.saved").to_string()),
                cx,
            );
            cx.notify();
        }
    }

    fn apply_save_error(
        &mut self,
        tab_id: u64,
        remote_path: &str,
        _message: String,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index_by_identity(tab_id, remote_path) else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        tab.saving = false;
        tab.status_message = t!("RemoteFileEditor.status.save_failed").to_string();
        self.close_window_after_saves = false;
        cx.notify();
    }

    fn handle_window_should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match decide_close_intercept(self.has_dirty_tabs(cx), self.close_prompt_open) {
            CloseIntercept::Allow => {
                clear_editor_window();
                true
            }
            CloseIntercept::Ignore => false,
            CloseIntercept::Prompt => {
                if let Some(index) = self.first_dirty_tab(cx) {
                    self.active_tab = index;
                    self.update_window_title(window);
                    self.focus_editor(window, cx);
                }
                self.show_unsaved_changes_prompt(PendingCloseAction::Window, window, cx);
                false
            }
        }
    }

    fn request_close_active_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request_close_tab(self.active_tab, window, cx);
    }

    fn request_close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }

        match decide_close_intercept(self.is_tab_dirty(index, cx), self.close_prompt_open) {
            CloseIntercept::Allow => self.close_clean_tab(index, window, cx),
            CloseIntercept::Ignore => {}
            CloseIntercept::Prompt => {
                self.show_unsaved_changes_prompt(PendingCloseAction::Tab(index), window, cx);
            }
        }
    }

    fn close_clean_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }

        let next_active = active_index_after_close(self.active_tab, index, self.tabs.len());
        self.tabs.remove(index);
        if let Some(next_active) = next_active {
            self.active_tab = next_active;
            self.update_window_title(window);
            self.focus_editor(window, cx);
            cx.notify();
        } else {
            clear_editor_window();
            window.remove_window();
        }
    }

    fn discard_close_action(
        &mut self,
        action: PendingCloseAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            PendingCloseAction::Window => {
                clear_editor_window();
                window.remove_window();
            }
            PendingCloseAction::Tab(index) => self.close_clean_tab(index, window, cx),
        }
    }

    fn save_close_action(
        &mut self,
        action: PendingCloseAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            PendingCloseAction::Window => self.save_dirty_tabs_and_close_window(window, cx),
            PendingCloseAction::Tab(index) => self.save_tab(index, true, window, cx),
        }
    }

    fn save_dirty_tabs_and_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty_indexes = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| tab.is_dirty(cx).then_some(index))
            .collect::<Vec<_>>();

        if dirty_indexes.is_empty() {
            clear_editor_window();
            window.remove_window();
            return;
        }

        self.close_window_after_saves = true;
        for index in dirty_indexes {
            self.save_tab(index, false, window, cx);
        }
    }

    fn show_unsaved_changes_prompt(
        &mut self,
        action: PendingCloseAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_prompt_open = true;
        self.pending_close_action = Some(action);
        let prompt_title = t!("RemoteFileEditor.prompt.unsaved_title").to_string();
        let prompt_message = t!("RemoteFileEditor.prompt.unsaved_message").to_string();
        let save_label = t!("RemoteFileEditor.action.save").to_string();
        let discard_label = t!("RemoteFileEditor.action.discard").to_string();
        let cancel_label = t!("RemoteFileEditor.action.cancel").to_string();
        let buttons = [
            save_label.as_str(),
            discard_label.as_str(),
            cancel_label.as_str(),
        ];
        let answer = window.prompt(
            PromptLevel::Warning,
            &prompt_title,
            Some(&prompt_message),
            &buttons,
            cx,
        );
        let window_handle = window.window_handle();

        cx.spawn(async move |this, cx| {
            let selection = answer.await.ok();
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    let action = this.pending_close_action.take();
                    this.close_prompt_open = false;
                    match (selection, action) {
                        (Some(0), Some(action)) => this.save_close_action(action, window, cx),
                        (Some(1), Some(action)) => this.discard_close_action(action, window, cx),
                        _ => {}
                    }
                });
            });
        })
        .detach();
    }

    fn trigger_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_editor(window, cx);
        window.dispatch_action(Box::new(Search), cx);
    }

    fn on_action_open_search(
        &mut self,
        _: &OpenSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.trigger_search(window, cx);
    }

    fn on_action_open_replace(
        &mut self,
        _: &OpenReplace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.trigger_replace(window, cx);
    }

    fn trigger_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_tab().and_then(|tab| tab.editor.as_ref()) else {
            return;
        };

        editor.update(cx, |state, cx| {
            state.open_search_and_replace(window, cx);
        });
    }

    fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = self.active_tab().and_then(|tab| tab.editor.as_ref()) else {
            return;
        };

        editor.update(cx, |state, cx| {
            state.focus(window, cx);
        });
    }

    fn toggle_soft_wrap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        tab.soft_wrap = !tab.soft_wrap;
        if let Some(editor) = tab.editor.as_ref() {
            editor.update(cx, |state, cx| {
                state.set_soft_wrap(tab.soft_wrap, window, cx);
            });
        }
        cx.notify();
    }

    fn switch_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() || index == self.active_tab {
            return;
        }

        self.active_tab = index;
        self.update_window_title(window);
        self.focus_editor(window, cx);
        cx.notify();
    }

    fn is_tab_dirty(&self, index: usize, cx: &App) -> bool {
        self.tabs
            .get(index)
            .map(|tab| tab.is_dirty(cx))
            .unwrap_or(false)
    }

    fn has_dirty_tabs(&self, cx: &App) -> bool {
        self.tabs.iter().any(|tab| tab.is_dirty(cx))
    }

    fn first_dirty_tab(&self, cx: &App) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.is_dirty(cx))
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tab_bar = TabBar::new("remote-file-editor-tabs")
            .menu(true)
            .with_size(Size::Small)
            .selected_index(self.active_tab)
            .on_click({
                let view = cx.entity().clone();
                move |index, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.switch_tab(*index, window, cx);
                    });
                }
            });

        for (index, tab) in self.tabs.iter().enumerate() {
            let label = if tab.is_dirty(cx) {
                format!("* {}", tab.display_name)
            } else {
                tab.display_name.clone()
            };
            tab_bar = tab_bar.child(
                Tab::new().label(label).suffix(
                    Button::new(format!("remote-file-close-tab-{index}"))
                        .label("×")
                        .with_size(Size::XSmall)
                        .disabled(tab.saving)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.request_close_tab(index, window, cx);
                        })),
                ),
            );
        }

        h_flex()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().tab_bar)
            .child(tab_bar)
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.active_tab();
        let dirty = tab.map(|tab| tab.is_dirty(cx)).unwrap_or(false);
        let disabled = tab
            .map(|tab| tab.loading || tab.saving || tab.editor.is_none())
            .unwrap_or(true);
        let loading_or_saving = tab.map(|tab| tab.loading || tab.saving).unwrap_or(true);
        let soft_wrap = tab.map(|tab| tab.soft_wrap).unwrap_or(false);

        h_flex()
            .gap_2()
            .items_center()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().title_bar)
            .child(
                Button::new("remote-file-save")
                    .label(t!("RemoteFileEditor.action.save"))
                    .with_size(Size::Small)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.save(false, window, cx);
                    })),
            )
            .child(
                Button::new("remote-file-search")
                    .label(t!("RemoteFileEditor.action.search"))
                    .with_size(Size::Small)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.trigger_search(window, cx);
                    })),
            )
            .child(
                Button::new("remote-file-replace")
                    .label(t!("RemoteFileEditor.action.replace"))
                    .with_size(Size::Small)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.trigger_replace(window, cx);
                    })),
            )
            .child(
                Button::new("remote-file-reload")
                    .label(t!("RemoteFileEditor.action.reload"))
                    .with_size(Size::Small)
                    .disabled(loading_or_saving)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.reload(window, cx);
                    })),
            )
            .child(
                Button::new("remote-file-soft-wrap")
                    .label(t!("RemoteFileEditor.action.soft_wrap"))
                    .selected(soft_wrap)
                    .with_size(Size::Small)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_soft_wrap(window, cx);
                    })),
            )
            .child(
                Button::new("remote-file-close-active-tab")
                    .label(t!("RemoteFileEditor.action.close_tab"))
                    .with_size(Size::Small)
                    .disabled(loading_or_saving || self.tabs.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.request_close_active_tab(window, cx);
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tab.map(RemoteEditorTab::policy_label).unwrap_or_default()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(if dirty {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(if dirty {
                        t!("RemoteFileEditor.state.modified")
                    } else {
                        t!("RemoteFileEditor.state.saved")
                    }),
            )
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let remote_path = self
            .active_tab()
            .map(|tab| tab.remote_path.clone())
            .unwrap_or_default();
        let file_size = self
            .active_tab()
            .map(|tab| tab.file_size)
            .unwrap_or_default();
        let status_message = self
            .active_tab()
            .map(|tab| tab.status_message.clone())
            .unwrap_or_default();

        h_flex()
            .gap_2()
            .items_center()
            .px_3()
            .py_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(remote_path),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format_size(file_size)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(status_message),
            )
    }

    fn render_body(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(tab) = self.active_tab() else {
            return v_flex().size_full().into_any_element();
        };

        if tab.loading {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(div().text_sm().child(t!("RemoteFileEditor.body.loading")))
                .into_any_element();
        }

        if let Some(error) = tab.load_error.as_ref() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .text_base()
                        .child(t!("RemoteFileEditor.body.unable_to_open")),
                )
                .child(
                    div()
                        .max_w(px(560.0))
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(error.clone()),
                )
                .into_any_element();
        }

        match tab.editor.as_ref() {
            Some(editor) => v_flex()
                .size_full()
                .child(Input::new(editor).size_full())
                .into_any_element(),
            None => v_flex().size_full().into_any_element(),
        }
    }
}

impl Render for RemoteFileEditorWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .active_tab()
            .map(|tab| tab.display_name.clone())
            .unwrap_or_default();

        v_flex()
            .size_full()
            .key_context(REMOTE_FILE_EDITOR_CONTEXT)
            .on_action(cx.listener(Self::on_action_open_search))
            .on_action(cx.listener(Self::on_action_open_replace))
            .bg(cx.theme().background)
            .child(
                TitleBar::new().child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_1()
                        .text_sm()
                        .child(title),
                ),
            )
            .child(self.render_tabs(cx))
            .child(self.render_toolbar(cx))
            .child(v_flex().flex_1().child(self.render_body(window, cx)))
            .child(self.render_status_bar(cx))
    }
}

fn display_name_from_path(path: &str) -> String {
    path.rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn format_size(size: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = 1024 * 1024;

    if size >= MIB {
        format!("{:.1} MiB", size as f64 / MIB as f64)
    } else if size >= KIB {
        format!("{:.1} KiB", size as f64 / KIB as f64)
    } else {
        format!("{} B", size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    const EXPECTED_SEARCH_SHORTCUT: &str = "cmd-f";
    #[cfg(not(target_os = "macos"))]
    const EXPECTED_SEARCH_SHORTCUT: &str = "ctrl-f";

    #[cfg(target_os = "macos")]
    const EXPECTED_REPLACE_SHORTCUT: &str = "cmd-r";
    #[cfg(not(target_os = "macos"))]
    const EXPECTED_REPLACE_SHORTCUT: &str = "ctrl-r";

    #[test]
    fn test_platform_shortcuts_match_editor_expectations() {
        assert_eq!(search_shortcut(), EXPECTED_SEARCH_SHORTCUT);
        assert_eq!(replace_shortcut(), EXPECTED_REPLACE_SHORTCUT);
    }

    #[test]
    fn display_name_ignores_trailing_slash() {
        assert_eq!(display_name_from_path("/tmp/example/"), "example");
    }

    #[test]
    fn format_size_uses_binary_units() {
        assert_eq!(format_size(42), "42 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1024 * 1024), "1.0 MiB");
    }
}
