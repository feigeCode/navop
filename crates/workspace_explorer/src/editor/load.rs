use super::{
    DIFF_LANGUAGE, DiffEditors, DocumentKey, DocumentPolicy, EditorTab, GitDiffRequest,
    LoadRequest, LoadedDocument, PendingDocument, WorkspaceEditor, WorkspaceEditorEvent,
    display_name,
};
use crate::diff::{
    AlignedDiffSide, AlignedSpanKind, DiffTextSpanKind, aligned_side_by_side, aligned_span_ranges,
    diff_text_spans, parse_side_by_side,
};
use crate::editor::markdown::create_markdown_editor;
use crate::git::load_diff;
use crate::model::active_index_after_open;
use gpui::{
    AppContext as _, AsyncApp, Context, Entity, Hsla, Pixels, Task, WeakEntity, Window, px,
};
use gpui_component::{
    WindowExt as _,
    input::{EditorState, InputEvent, RangeDecoration, RangeDecorationStyle},
    notification::Notification,
};
use one_ui::StatusPresentation;
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
                    ensure_diff_language();
                    let language = remote_file_editor::load_language_for_path(
                        &change.path.to_string_lossy(),
                        false,
                    )?;
                    load_diff(&repository, &change)
                        .map(|diff| LoadedDocument::from_diff(diff, language))
                })
            }
            LoadRequest::SnapshotDiff { text } => {
                let document = LoadedDocument::from_snapshot_diff(text.clone());
                cx.background_spawn(async move {
                    ensure_diff_language();
                    anyhow::Ok(document)
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
        let markdown_path = match (&tab.key, document.policy) {
            (DocumentKey::File(path), DocumentPolicy::Markdown) => Some(path.clone()),
            _ => None,
        };
        let editor = markdown_path.is_none().then(|| {
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
        tab.diff = match tab.policy {
            DocumentPolicy::Diff => {
                let parsed = parse_side_by_side(&tab.saved_text);
                (!parsed.rows.is_empty()).then(|| Rc::new(parsed))
            }
            _ => None,
        };
        // diff 的三种行背景。直接复用工作区主题已有的语义色，不为 diff 另开一套：
        // `danger` / `success` 在应用主题里是饱和的红绿，压到 20% 铺在背景上依然
        // 一眼可辨；`muted` 本来就是"比背景略深一层"的表面色，正好用来表示
        // "这一侧没有对应行"。
        let removed_background = self.theme.danger.opacity(0.20);
        let added_background = self.theme.success.opacity(0.20);
        let gap_background = self.theme.muted;
        tab.diff_change_cursor = None;
        // 单栏视图(`tab.editor`)上的底色。并排两栏各自带装饰，退回单栏
        //（整轮快照 diff 恒为单栏；单文件 diff 关掉「并排对比」后也是单栏）
        // 时若不补这一份，整屏就只剩黑白文本，看不出哪行增、哪行删。
        tab.diff_spans = match (editor.as_ref(), tab.policy) {
            (Some(editor), DocumentPolicy::Diff) => {
                let decorations = diff_text_decorations(
                    &tab.saved_text,
                    removed_background,
                    added_background,
                    gap_background,
                );
                Some(editor.update(cx, |state, cx| {
                    state.create_range_decorations_collection(decorations, cx)
                }))
            }
            _ => None,
        };
        tab.diff_editors = match (&tab.diff, diff_language) {
            (Some(diff), Some(language)) => {
                let (left_side, right_side) = aligned_side_by_side(diff);
                // 行背景必须赶在 `set_value` 之前算出来：那一步会把两侧文本移进
                // 编辑器，之后就借不出 `changed` / `placeholders` 了。
                let left_decorations =
                    diff_span_decorations(&left_side, removed_background, gap_background);
                let right_decorations =
                    diff_span_decorations(&right_side, added_background, gap_background);
                let left_language = language.clone();
                let left = cx.new(|cx| {
                    let mut state = EditorState::new(window, cx)
                        .language(left_language)
                        .folding(false)
                        .line_number(true)
                        .searchable(true)
                        .soft_wrap(false);
                    state.set_value(left_side.text, window, cx);
                    state
                });
                let right = cx.new(|cx| {
                    let mut state = EditorState::new(window, cx)
                        .language(language)
                        .folding(false)
                        .line_number(true)
                        .searchable(true)
                        .soft_wrap(false);
                    state.set_value(right_side.text, window, cx);
                    state
                });
                // 并排本身只把两份对齐文本摆在一起，不含任何"这里不一样"的信息：
                // `changed` / `placeholders` 算出来了却没人用，用户看到的就是两栏
                // 一模一样的黑白代码。挂上填充装饰，左栏变更行是删除色、右栏是新增
                // 色、没有对应行的那一侧是补白色。
                let left_spans = left.update(cx, |state, cx| {
                    state.create_range_decorations_collection(left_decorations, cx)
                });
                let right_spans = right.update(cx, |state, cx| {
                    state.create_range_decorations_collection(right_decorations, cx)
                });
                Some(DiffEditors {
                    left,
                    right,
                    _left_spans: left_spans,
                    _right_spans: right_spans,
                })
            }
            _ => None,
        };
        // 并排模式必须真的有并排数据才成立。快照 diff（整轮多文件）刻意不做双栏
        // 对齐，`diff_editors` 因此是 `None`；若这里留着 `EditorTab::new` 的默认
        // `true`，`render_body` 会跳过并排落到单栏，而工具栏「并排对比」仍显示为
        // 已开启——按钮在撒谎，点它也只是把它关掉，视口毫无变化。
        tab.diff_side_by_side = tab.diff_editors.is_some();
        // 并排两栏是同一份内容逐行对齐的结果，各滚各的会让"并排对比"失去意义。
        // 只同步纵向：占位行已经把两栏补到同样多行，纵向偏移天然一一对应；横向
        // 留给各自——两侧内容宽度不同，强行对齐只会把窄的那栏推进空白。
        if let Some(editors) = tab.diff_editors.as_ref() {
            tab.subscriptions.push(cx.observe(&editors.left, {
                let target = editors.right.clone();
                move |_this, source, cx| sync_scroll(source, &target, cx)
            }));
            tab.subscriptions.push(cx.observe(&editors.right, {
                let target = editors.left.clone();
                move |_this, source, cx| sync_scroll(source, &target, cx)
            }));
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

/// 把对齐结果里需要提示的行变成编辑器行背景。
///
/// 只画变更行与占位行：两侧相同的行不需要任何标记，给每一行都铺底色反而会
/// 稀释真正的差异。
fn diff_span_decorations(
    side: &AlignedDiffSide,
    changed_background: Hsla,
    placeholder_background: Hsla,
) -> Vec<RangeDecoration> {
    aligned_span_ranges(side)
        .into_iter()
        .filter_map(|(range, kind)| {
            let color = match kind {
                AlignedSpanKind::Context => return None,
                AlignedSpanKind::Changed => changed_background,
                AlignedSpanKind::Placeholder => placeholder_background,
            };
            Some(
                RangeDecoration::new(range)
                    .with_style(RangeDecorationStyle::Fill)
                    .with_color(color),
            )
        })
        .collect()
}

/// 单栏 diff 原文的行背景：新增、删除、以及 `diff --git` / `@@` 段落标记。
///
/// 与并排两栏同一套语义色，区别只在输入：这里吃的是 patch 原文，不需要事先
/// 对齐。段落标记用 `muted`——整轮多文件 diff 是单栏的一整片文本，没有这层
/// 标记就找不到文件与 hunk 的边界。
fn diff_text_decorations(
    diff: &str,
    removed_background: Hsla,
    added_background: Hsla,
    marker_background: Hsla,
) -> Vec<RangeDecoration> {
    diff_text_spans(diff)
        .into_iter()
        .map(|(range, kind)| {
            let color = match kind {
                DiffTextSpanKind::Added => added_background,
                DiffTextSpanKind::Removed => removed_background,
                DiffTextSpanKind::Marker => marker_background,
            };
            RangeDecoration::new(range)
                .with_style(RangeDecorationStyle::Fill)
                .with_color(color)
        })
        .collect()
}

/// 确保只读 diff 视图用的语法已加载。
///
/// 调用方是 `background_spawn` 的任务：加载要编译 tree-sitter wasm，放在 UI
/// 线程上会卡住整帧。
///
/// 失败只记日志、不打断加载——语法高亮是锦上添花，diff 原文本身仍然可读；
/// 反过来让「语法扩展损坏」变成「diff 打不开」才是真的糟。
fn ensure_diff_language() {
    if let Err(error) = remote_file_editor::load_language(DIFF_LANGUAGE) {
        tracing::warn!(
            language = DIFF_LANGUAGE,
            %error,
            "failed to load the grammar for the read-only diff view"
        );
    }
}

/// 需要写进目标栏的纵向偏移；`None` 表示两栏已在容差内对齐。
///
/// 抽成纯函数是为了让它可测：真正落笔要经过 `Entity`，而"什么时候**不该**写"
/// 才是容易出错的地方。
fn scroll_sync_target(source_y: Pixels, target_y: Pixels) -> Option<Pixels> {
    if (target_y - source_y).abs() < px(SCROLL_SYNC_TOLERANCE) {
        return None;
    }
    Some(source_y)
}

/// 两栏纵向偏移的容差。`set_scroll_offset` 会被各自视口高度 clamp，边界处可能
/// 差出亚像素；不留容差，两栏就会互相推来推去。
const SCROLL_SYNC_TOLERANCE: f32 = 0.5;

/// 把并排栏的纵向滚动对齐到另一栏。
///
/// 写入是"先比较、相等就跳过"，所以不需要防重入标志：`set_scroll_offset` 要到
/// 下次布局才生效，而 `update_scroll_offset` 在偏移没变时不会 `notify`，回环
/// 自然终止。
fn sync_scroll(
    source: Entity<EditorState>,
    target: &Entity<EditorState>,
    cx: &mut Context<WorkspaceEditor>,
) {
    let source_y = source.read(cx).scroll_offset().y;
    let mut offset = target.read(cx).scroll_offset();
    let Some(target_y) = scroll_sync_target(source_y, offset.y) else {
        return;
    };
    offset.y = target_y;
    target.update(cx, |state, cx| state.set_scroll_offset(offset, cx));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_sync_ignores_subpixel_drift_but_follows_real_moves() {
        // clamp 造成的亚像素差不该触发写入，否则两栏会互相推。
        assert_eq!(None, scroll_sync_target(px(120.0), px(120.4)));
        assert_eq!(None, scroll_sync_target(px(120.0), px(120.0)));
        // 真正的滚动必须跟随，向上向下两个方向都算。
        assert_eq!(Some(px(240.0)), scroll_sync_target(px(240.0), px(120.0)));
        assert_eq!(Some(px(0.0)), scroll_sync_target(px(0.0), px(120.0)));
    }
}
