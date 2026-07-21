use super::{
    DiffEditors, DocumentKey, DocumentPolicy, EditorTab, GitDiffRequest, LoadRequest,
    LoadedDocument, PendingDocument, WorkspaceEditor, WorkspaceEditorEvent, diff_line_decorations,
    display_name,
};
use crate::diff::{aligned_side_by_side, parse_side_by_side};
use crate::file_system::load_file;
use crate::git::load_diff;
use crate::model::active_index_after_open;
use gpui::{AppContext as _, AsyncApp, Context, Task, WeakEntity, Window};
use gpui_component::{
    WindowExt as _,
    input::{InputEvent, InputState},
    notification::Notification,
};
use rust_i18n::t;
use std::path::PathBuf;
use std::rc::Rc;

impl WorkspaceEditor {
    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_document(
            PendingDocument {
                key: DocumentKey::File(path.clone()),
                display_name: display_name(&path),
                load_request: LoadRequest::File(path),
            },
            window,
            cx,
        );
    }

    pub(crate) fn open_git_change(
        &mut self,
        request: GitDiffRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = t!(
            "WorkspaceExplorer.editor.diff_tab",
            name = display_name(&request.change.path)
        )
        .to_string();
        self.open_document(
            PendingDocument {
                key: DocumentKey::Diff {
                    repository: request.repository.root.clone(),
                    path: request.change.path.clone(),
                },
                display_name: name,
                load_request: LoadRequest::Diff {
                    repository: request.repository,
                    change: request.change,
                },
            },
            window,
            cx,
        );
    }

    fn open_document(
        &mut self,
        document: PendingDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let identities = self
            .tabs
            .iter()
            .map(|tab| tab.key.identity_path())
            .collect::<Vec<_>>();
        let active_index = active_index_after_open(&identities, &document.key.identity_path());
        if active_index < self.tabs.len() {
            self.active_tab = active_index;
            self.focus_editor(window, cx);
            cx.notify();
            return;
        }
        let was_empty = self.tabs.is_empty();
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(EditorTab::new(tab_id, document));
        self.active_tab = active_index;
        if was_empty {
            cx.emit(WorkspaceEditorEvent::VisibilityChanged(true));
        }
        self.reload_tab(active_index, window, cx);
    }

    pub(super) fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_tab(self.active_tab, window, cx);
    }

    fn reload_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(attempt) = self.prepare_load(index, cx) else {
            return;
        };
        let entity = cx.entity().downgrade();
        let window_handle = window.window_handle();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let LoadAttempt { tab_id, key, task } = attempt;
            let outcome = task.await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let Some(entity) = entity.upgrade() else {
                    return;
                };
                entity.update(cx, |this, cx| match outcome {
                    Ok(document) => this.apply_loaded(
                        LoadCompletion {
                            tab_id,
                            key,
                            document,
                        },
                        window,
                        cx,
                    ),
                    Err(error) => this.report_load_error(
                        LoadFailure {
                            tab_id,
                            key,
                            message: error.to_string(),
                        },
                        window,
                        cx,
                    ),
                });
            });
        })
        .detach();
    }

    fn prepare_load(&mut self, index: usize, cx: &mut Context<Self>) -> Option<LoadAttempt> {
        let Some(tab) = self.tabs.get_mut(index) else {
            return None;
        };
        tab.loading = true;
        tab.load_error = None;
        tab.status_message = t!("WorkspaceExplorer.status.loading").to_string();
        cx.notify();

        let tab_id = tab.id;
        let key = tab.key.clone();
        let task = match &tab.load_request {
            LoadRequest::File(path) => {
                let path = path.clone();
                cx.background_spawn(async move { load_file(&path).map(LoadedDocument::from_file) })
            }
            LoadRequest::Diff { repository, change } => {
                let repository = repository.clone();
                let change = change.clone();
                cx.background_spawn(async move {
                    let language = remote_file_editor::language_for_path(
                        &change.path.to_string_lossy(),
                        false,
                    );
                    load_diff(&repository, &change)
                        .map(|diff| LoadedDocument::from_diff(diff, language))
                })
            }
        };

        Some(LoadAttempt { tab_id, key, task })
    }

    fn report_load_error(
        &mut self,
        failure: LoadFailure,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = failure.message.clone();
        self.apply_load_error(failure, cx);
        window.push_notification(Notification::error(message), cx);
    }

    fn apply_loaded(
        &mut self,
        completion: LoadCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tab_index(completion.tab_id, &completion.key) else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(index) else {
            return;
        };
        let document = completion.document;
        let initial_text = document.text.clone();
        let diff_language = document.diff_language.clone();
        let editor = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .code_editor(document.language)
                .line_number(true)
                .searchable(true)
                .soft_wrap(tab.soft_wrap);
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
        tab.editor = Some(editor.clone());
        tab.saved_text = document.text;
        tab.file_size = document.file_size;
        tab.policy = document.policy;
        tab.diff = match tab.policy {
            DocumentPolicy::Diff => {
                let parsed = parse_side_by_side(&tab.saved_text);
                (!parsed.rows.is_empty()).then(|| Rc::new(parsed))
            }
            _ => None,
        };
        tab.diff_change_cursor = None;
        tab.diff_editors = match (&tab.diff, diff_language) {
            (Some(diff), Some(language)) => {
                let (left_side, right_side) = aligned_side_by_side(diff);
                let left_decorations = diff_line_decorations(
                    &left_side,
                    self.theme.danger,
                    self.theme.muted.opacity(0.35),
                );
                let right_decorations = diff_line_decorations(
                    &right_side,
                    self.theme.success,
                    self.theme.muted.opacity(0.35),
                );
                let scroll_handle = tab.diff_scroll.clone();
                let left_scroll_handle = scroll_handle.clone();
                let left_language = language.clone();
                let left = cx.new(|cx| {
                    let mut state = InputState::new(window, cx)
                        .code_editor(left_language)
                        .folding(false)
                        .line_number(true)
                        .searchable(true)
                        .soft_wrap(false)
                        .read_only(true)
                        .shared_scroll_handle(left_scroll_handle)
                        .line_decorations(left_decorations);
                    state.set_value(left_side.text, window, cx);
                    state
                });
                let right = cx.new(|cx| {
                    let mut state = InputState::new(window, cx)
                        .code_editor(language)
                        .folding(false)
                        .line_number(true)
                        .searchable(true)
                        .soft_wrap(false)
                        .read_only(true)
                        .shared_scroll_handle(scroll_handle)
                        .line_decorations(right_decorations);
                    state.set_value(right_side.text, window, cx);
                    state
                });
                Some(DiffEditors { left, right })
            }
            _ => None,
        };
        tab.loading = false;
        tab.saving = false;
        tab.read_only = document.read_only;
        tab.load_error = None;
        tab.status_message = loaded_status(tab.policy).to_string();
        if index == self.active_tab && !tab.read_only {
            editor.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    fn apply_load_error(&mut self, failure: LoadFailure, cx: &mut Context<Self>) {
        let Some(index) = self.tab_index(failure.tab_id, &failure.key) else {
            return;
        };
        let tab = &mut self.tabs[index];
        tab.loading = false;
        tab.load_error = Some(failure.message);
        tab.status_message = t!("WorkspaceExplorer.status.load_failed").to_string();
        cx.notify();
    }
}

struct LoadCompletion {
    tab_id: u64,
    key: DocumentKey,
    document: LoadedDocument,
}

struct LoadAttempt {
    tab_id: u64,
    key: DocumentKey,
    task: Task<anyhow::Result<LoadedDocument>>,
}

struct LoadFailure {
    tab_id: u64,
    key: DocumentKey,
    message: String,
}

fn loaded_status(policy: DocumentPolicy) -> std::borrow::Cow<'static, str> {
    match policy {
        DocumentPolicy::Diff => t!("WorkspaceExplorer.status.diff_loaded"),
        DocumentPolicy::Code | DocumentPolicy::PlainText => {
            t!("WorkspaceExplorer.status.loaded")
        }
    }
}
