use super::{
    DocumentKey, DocumentPolicy, EditorTab, GitDiffRequest, LoadRequest, LoadedDocument,
    PendingDocument, WorkspaceEditor, WorkspaceEditorEvent, display_name,
};
use crate::editor::markdown::create_markdown_editor;
use crate::git::load_diff;
use crate::model::active_index_after_open;
use gpui::{AppContext as _, AsyncApp, Context, Entity, Task, WeakEntity, Window};
use gpui_component::{
    WindowExt as _,
    diff::{DiffFile, DiffMode, DiffState},
    input::{EditorState, InputEvent},
    notification::Notification,
};
use one_ui::StatusPresentation;
use rust_i18n::t;
use std::path::PathBuf;

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

    /// 打开（或聚焦已打开的）整轮快照 diff 标签页。
    ///
    /// 同一 `navop://last-turn-review` key 复用标签页：每轮刷新同一页，
    /// 不会为每一轮开出新标签。
    pub fn open_snapshot_diff(
        &mut self,
        display_name: String,
        diff_text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_document(
            PendingDocument {
                key: DocumentKey::SnapshotDiff,
                display_name,
                load_request: LoadRequest::SnapshotDiff { text: diff_text },
            },
            window,
            cx,
        );
    }

    /// 打开（或聚焦已打开的）某条 Git 变更的 diff 标签页。
    pub fn open_diff(
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

    /// 打开（或聚焦已打开的）单个文件的审阅 diff 标签页。
    ///
    /// 内容是**本轮快照里该文件那一段**（由
    /// [`crate::WorkspaceExplorer::open_review_file`] 裁好送进来），所以按文件
    /// 分页：不同文件各占一页，同一文件再来一遍就刷新它。
    pub fn open_review_diff(
        &mut self,
        path: PathBuf,
        diff_text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = t!(
            "WorkspaceExplorer.editor.diff_tab",
            name = display_name(&path)
        )
        .to_string();
        self.open_document(
            PendingDocument {
                key: DocumentKey::ReviewFile(path),
                display_name: name,
                load_request: LoadRequest::SnapshotDiff { text: diff_text },
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
            // 差异页是只读快照：同一 key 再次打开意味着来了一份新 patch（每轮的
            // last-turn diff 刷新同一页），必须换掉内容——只聚焦会让面板一直停在
            // 旧的那一轮。文件页相反：重载会吞掉用户未保存的改动，只能聚焦。
            if document.load_request.is_diff() && self.tabs[active_index].read_only {
                let tab = &mut self.tabs[active_index];
                tab.load_request = document.load_request;
                tab.display_name = document.display_name;
                self.reload_tab(active_index, window, cx);
            } else {
                self.focus_editor(window, cx);
                cx.notify();
            }
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
        if let Some(markdown) = self
            .tabs
            .get(self.active_tab)
            .and_then(|tab| tab.markdown.clone())
        {
            markdown.update(cx, |view, cx| {
                view.reload_active_markdown_from_disk(window, cx)
            });
            return;
        }
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
        tab.status_presentation = StatusPresentation::Progress;
        cx.notify();

        let tab_id = tab.id;
        let key = tab.key.clone();
        let task = match &tab.load_request {
            LoadRequest::File(path) => {
                let path = path.clone();
                let backend = self.backend.clone();
                cx.background_spawn(async move {
                    backend
                        .load_file(&path)
                        .map(|file| LoadedDocument::from_file(&path, file))
                })
            }
            LoadRequest::Diff { repository, change } => {
                let repository = repository.clone();
                let change = change.clone();
                cx.background_spawn(async move {
                    load_diff(&repository, &change).map(LoadedDocument::from_diff)
                })
            }
            LoadRequest::SnapshotDiff { text } => {
                let document = LoadedDocument::from_snapshot_diff(text.clone());
                cx.background_spawn(async move { anyhow::Ok(document) })
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
        let is_diff = matches!(document.policy, DocumentPolicy::Diff);
        let markdown_path = match (&tab.key, document.policy) {
            (DocumentKey::File(path), DocumentPolicy::Markdown) => Some(path.clone()),
            _ => None,
        };
        // diff 不再走编辑器:渲染全部交给 Diff 组件(见下方 `diff_state`)。
        let editor = (markdown_path.is_none() && !is_diff).then(|| {
            cx.new(|cx| {
                let mut state = EditorState::new(window, cx)
                    .language(document.language)
                    .line_number(true)
                    .searchable(true)
                    .soft_wrap(tab.soft_wrap);
                state.set_value(initial_text, window, cx);
                state
            })
        });
        tab.subscriptions.clear();
        if let Some(editor) = editor.as_ref() {
            tab.subscriptions.push(cx.subscribe(
                editor,
                |_this, _input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                },
            ));
        }
        let markdown =
            markdown_path.map(|path| create_markdown_editor(path, self.theme, window, cx));
        tab.markdown = markdown.as_ref().map(|(view, _)| view.clone());
        if let Some((_, subscriptions)) = markdown {
            tab.subscriptions.extend(subscriptions);
        }
        tab.editor = editor.clone();
        tab.saved_text = document.text;
        tab.file_size = document.file_size;
        tab.policy = document.policy;
        tab.diff_state = if is_diff {
            build_diff_state(&tab.saved_text, tab.diff_side_by_side, cx)
        } else {
            None
        };
        if let Some(diff_state) = tab.diff_state.as_ref() {
            // 组件内部会异步准备行布局与语法高亮,完成后需要重渲染。
            tab.subscriptions
                .push(cx.observe(diff_state, |_, _, cx| cx.notify()));
        }
        tab.loading = false;
        tab.saving = false;
        tab.read_only = document.read_only;
        tab.load_error = None;
        tab.status_message = loaded_status(tab.policy).to_string();
        tab.status_presentation = StatusPresentation::Neutral;
        if index == self.active_tab && !tab.read_only {
            if let Some(editor) = editor {
                editor.update(cx, |state, cx| state.focus(window, cx));
            } else if let Some(markdown) = tab.markdown.as_ref() {
                markdown.update(cx, |view, cx| view.focus_active_editor(window, cx));
            }
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
        tab.status_presentation = StatusPresentation::Error;
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
        DocumentPolicy::Markdown => t!("WorkspaceExplorer.status.markdown_loaded"),
        DocumentPolicy::Code | DocumentPolicy::PlainText => {
            t!("WorkspaceExplorer.status.loaded")
        }
    }
}

/// 从 patch 原文构建 Diff 组件状态。
///
/// 渲染(行对齐、增删底色、行号、Split/Unified、语法高亮、虚拟滚动)全部由
/// 组件负责,这里只负责把 patch 文本喂进去,并按文件扩展名标注语言——组件的
/// 语言表认小写扩展名并自带别名映射(`rs` → `rust` 等),对不上号就只少一层
/// 高亮。解析失败(空 patch、坏 patch)返回 `None`,渲染层落到「空 diff」。
fn build_diff_state(
    text: &str,
    side_by_side: bool,
    cx: &mut Context<WorkspaceEditor>,
) -> Option<Entity<DiffState>> {
    let files: Vec<DiffFile> = DiffFile::parse(text)
        .ok()?
        .into_iter()
        .map(|file| {
            let language = language_name_for_path(file.path());
            match language {
                Some(language) => file.with_language(language),
                None => file,
            }
        })
        .collect();
    if files.is_empty() {
        return None;
    }
    let mode = if side_by_side {
        DiffMode::Split
    } else {
        DiffMode::Unified
    };
    Some(cx.new(|cx| DiffState::new(files, cx).with_mode(mode)))
}

/// 文件扩展名的小写形式,作为组件语法高亮的语言名。
fn language_name_for_path(path: &str) -> Option<String> {
    let extension = path.rsplit_once('.')?.1;
    if extension.is_empty() || extension.contains('/') || extension.contains('\\') {
        return None;
    }
    Some(extension.to_lowercase())
}
