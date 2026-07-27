use crate::notes_notifications::notify_operation_error;
use crate::theme_provider::MarkdownEditorTheme;
use crate::{DocumentDescriptor, DocumentFormat, NodeKind, NotesStorage, TreeRow, TreeState};
use anyhow::{Context as _, bail};
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Window,
};
use gpui_component::input::InputState;
use one_core::tab_container::TabContentEvent;
use rust_i18n::t;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub(crate) enum NotesLoadState {
    NeedsLocation,
    Ready,
}

#[derive(Clone, Debug)]
pub enum NotesViewEvent {
    FileSaved(PathBuf),
}

pub struct NotesView {
    pub(crate) storage: Option<NotesStorage>,
    pub(crate) load_state: NotesLoadState,
    pub(crate) tree: TreeState,
    pub(crate) rows: Vec<TreeRow>,
    pub(crate) markdown_sessions: HashMap<String, crate::markdown_session::MarkdownSession>,
    pub(crate) active_document_id: Option<String>,
    pub(crate) current_directory: PathBuf,
    pub(crate) selected_sidebar_path: Option<PathBuf>,
    pub(crate) context_menu_path: Option<PathBuf>,
    pub(crate) notebook_name: SharedString,
    pub(crate) setup_path: Entity<InputState>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) standalone_markdown: bool,
    pub(crate) sidebar_collapsed: bool,
    pub(crate) editor_theme: Option<MarkdownEditorTheme>,
}

impl NotesView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self::base(
            "Notes".into(),
            NotesLoadState::NeedsLocation,
            false,
            None,
            window,
            cx,
        );
        let should_prompt = match view.initialize_configured_notes(window, cx) {
            Ok(configured) => !configured,
            Err(error) => {
                notify_operation_error(window, cx, error);
                true
            }
        };
        if should_prompt {
            crate::notes_setup::defer_location_dialog(view.setup_path.clone(), window, cx);
        }
        view
    }

    /// Opens an arbitrary Markdown file in-place without copying it into the Notes notebook.
    /// Both source-mode and WYSIWYG saves continue to target the supplied file path.
    pub fn new_for_markdown_file(
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = markdown_title(&path);
        let mut view = Self::base(title.into(), NotesLoadState::Ready, true, None, window, cx);
        view.open_standalone_markdown(path, window, cx);
        view
    }

    pub fn new_for_markdown_file_with_theme(
        path: PathBuf,
        theme: MarkdownEditorTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = markdown_title(&path);
        let mut view = Self::base(
            title.into(),
            NotesLoadState::Ready,
            true,
            Some(theme),
            window,
            cx,
        );
        view.open_standalone_markdown(path, window, cx);
        view
    }

    fn open_standalone_markdown(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match standalone_markdown_descriptor(&path)
            .and_then(|descriptor| self.open_markdown_document(descriptor, window, cx))
        {
            Ok(()) => {}
            Err(error) => notify_operation_error(window, cx, error),
        }
    }

    pub fn focus_active_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(document_id) = self.active_document_id.as_ref()
            && let Some(session) = self.markdown_sessions.get(document_id)
        {
            match session.state.mode {
                crate::MarkdownViewMode::Source => {
                    session
                        .source_editor
                        .update(cx, |input, cx| input.focus(window, cx));
                }
                crate::MarkdownViewMode::Wysiwyg => {
                    session
                        .preview
                        .update(cx, |editor, cx| editor.focus(window, cx));
                }
            }
            return;
        }
    }

    fn base(
        notebook_name: SharedString,
        load_state: NotesLoadState,
        standalone_markdown: bool,
        editor_theme: Option<MarkdownEditorTheme>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let default_root = NotesStorage::default_root().unwrap_or_default();
        let initial_root = NotesStorage::configured_root().unwrap_or(default_root);
        let setup_path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("Notes.notebook_path_placeholder").to_string())
                .default_value(initial_root.to_string_lossy())
        });
        Self {
            storage: None,
            load_state,
            tree: TreeState::default(),
            rows: Vec::new(),
            markdown_sessions: HashMap::new(),
            active_document_id: None,
            current_directory: PathBuf::new(),
            selected_sidebar_path: None,
            context_menu_path: None,
            notebook_name,
            setup_path: setup_path.clone(),
            focus_handle: cx.focus_handle(),
            standalone_markdown,
            sidebar_collapsed: false,
            editor_theme,
        }
    }

    pub fn set_editor_theme(&mut self, theme: MarkdownEditorTheme, cx: &mut Context<Self>) {
        self.editor_theme = Some(theme);
        cx.notify();
    }

    pub(crate) fn resolved_editor_theme(&self, cx: &App) -> MarkdownEditorTheme {
        self.editor_theme
            .clone()
            .unwrap_or_else(|| MarkdownEditorTheme::from_app(cx))
    }

    pub(crate) fn refresh_tree(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let nodes = self.storage()?.scan_tree()?;
        self.rows = self.tree.project(&nodes);
        self.tree.select_fallback(&self.rows);
        if self.selected_sidebar_path.is_none() {
            self.selected_sidebar_path = self.tree.selected_document.clone();
        }
        self.storage()?.save_state(&self.tree.to_ui_state())?;
        if let Some(path) = self.tree.selected_document.clone() {
            self.open_document(&path, window, cx)?;
        } else {
            self.active_document_id = None;
        }
        Ok(())
    }

    pub(crate) fn open_document(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let descriptor = self.storage()?.descriptor(path)?;
        self.open_markdown_document(descriptor, window, cx)
    }

    pub(crate) fn select_row(
        &mut self,
        path: PathBuf,
        kind: NodeKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_sidebar_path = Some(path.clone());
        self.context_menu_path = None;
        let result = if kind == NodeKind::Directory {
            self.current_directory = path.clone();
            self.tree.toggle_directory(&path);
            self.refresh_tree(window, cx)
        } else {
            self.current_directory = path.parent().unwrap_or(Path::new("")).to_path_buf();
            self.tree.selected_document = Some(path.clone());
            self.open_document(&path, window, cx)
                .and_then(|_| self.storage()?.save_state(&self.tree.to_ui_state()))
        };
        if let Err(error) = result {
            notify_operation_error(window, cx, error);
        }
        cx.notify();
    }

    pub(crate) fn storage(&self) -> anyhow::Result<&NotesStorage> {
        self.storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("notes storage is unavailable"))
    }
}

fn markdown_title(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn standalone_markdown_descriptor(path: &Path) -> anyhow::Result<DocumentDescriptor> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    {
        bail!("unsupported Markdown file extension: {}", path.display());
    }
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("read Markdown metadata: {}", path.display()))?;
    if !metadata.is_file() {
        bail!("Markdown path is not a file: {}", path.display());
    }
    std::fs::read_to_string(path)
        .with_context(|| format!("read Markdown file as UTF-8: {}", path.display()))?;
    let file_name = path.file_name().context("Markdown file has no file name")?;
    Ok(DocumentDescriptor {
        document_id: uuid::Uuid::new_v4().to_string(),
        format: DocumentFormat::Markdown,
        relative_path: PathBuf::from(file_name),
        absolute_path: path.to_path_buf(),
    })
}

impl EventEmitter<TabContentEvent> for NotesView {}
impl EventEmitter<NotesViewEvent> for NotesView {}

impl Focusable for NotesView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod external_markdown_tests {
    use super::*;
    use crate::markdown_session::MarkdownSyncState;
    use gpui::{TestAppContext, VisualTestContext, WindowOptions};
    use gpui_component::Root;

    #[test]
    fn standalone_markdown_descriptor_preserves_the_external_file() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("Release Notes.md");
        std::fs::write(&path, "# Release Notes")?;

        let descriptor = standalone_markdown_descriptor(&path)?;

        assert_eq!(DocumentFormat::Markdown, descriptor.format);
        assert_eq!(PathBuf::from("Release Notes.md"), descriptor.relative_path);
        assert_eq!(path, descriptor.absolute_path);
        assert!(!descriptor.document_id.is_empty());
        Ok(())
    }

    #[test]
    fn standalone_markdown_descriptor_rejects_other_files() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("notes.txt");
        std::fs::write(&path, "notes")?;

        assert!(standalone_markdown_descriptor(&path).is_err());
        Ok(())
    }

    #[test]
    fn standalone_markdown_store_saves_back_to_the_opened_path() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("direct.md");
        std::fs::write(&path, "before")?;
        let descriptor = standalone_markdown_descriptor(&path)?;
        let store = crate::markdown_file_store::MarkdownFileStore::new(descriptor.absolute_path);

        assert_eq!("before", store.load()?.source);
        let outcome = store.save("after")?;

        assert!(matches!(
            outcome,
            crate::markdown_file_store::MarkdownSaveOutcome::Saved(_)
        ));
        assert_eq!("after", std::fs::read_to_string(path)?);
        Ok(())
    }

    #[gpui::test]
    fn standalone_source_preserving_preview_and_mode_switch_keep_bytes(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("round-trip.md");
        let source = concat!(
            "> <https://example.com/path_(item)>\n\n",
            "[README](README_CN.md) and `snake_case(value)`\n\n",
            "2. second\n\n_italic_\n",
        );
        std::fs::write(&path, source).unwrap();
        let (window, view) = cx.update(|cx| {
            let mut view = None;
            let window = cx
                .open_window(WindowOptions::default(), |window, cx| {
                    let entity =
                        cx.new(|cx| NotesView::new_for_markdown_file(path.clone(), window, cx));
                    view = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                })
                .unwrap();
            (window, view.unwrap())
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();

        let document_id = view.read_with(&cx, |view, cx| {
            let id = view.active_document_id.clone().unwrap();
            let session = view.markdown_sessions.get(&id).unwrap();
            assert_eq!(
                source,
                session.source_document.lock().unwrap().source.as_str()
            );
            assert_eq!(
                concat!(
                    "<https://example.com/path_(item)>\n\n",
                    "README and snake_case(value)\n\n",
                    "second\n\nitalic\n",
                ),
                session.preview.read(cx).projected_text()
            );
            assert!(!session.preview.read(cx).is_dirty());
            id
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.toggle_markdown_mode(document_id.clone(), window, cx);
            });
        });
        cx.run_until_parked();

        assert_eq!(source, std::fs::read_to_string(path).unwrap());
    }

    #[gpui::test]
    fn source_toggle_is_visible_in_preview_and_returns_to_preview(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source-toggle.md");
        std::fs::write(&path, "# Title\n\nBody\n").unwrap();
        let (window, view) = cx.update(|cx| {
            let mut view = None;
            let window = cx
                .open_window(WindowOptions::default(), |window, cx| {
                    let entity =
                        cx.new(|cx| NotesView::new_for_markdown_file(path.clone(), window, cx));
                    view = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                })
                .unwrap();
            (window, view.unwrap())
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();

        let toolbar_height = cx
            .debug_bounds("markdown-mode-toolbar")
            .expect("mode toolbar must be visible in preview")
            .size
            .height;
        let source_toggle = cx
            .debug_bounds("markdown-source-mode")
            .expect("source toggle must be visible in preview");
        cx.simulate_click(source_toggle.center(), gpui::Modifiers::default());
        cx.run_until_parked();

        assert_eq!(
            crate::MarkdownViewMode::Source,
            view.read_with(&cx, |view, _| view
                .markdown_sessions
                .values()
                .next()
                .unwrap()
                .state
                .mode)
        );
        assert_eq!(
            toolbar_height,
            cx.debug_bounds("markdown-mode-toolbar")
                .expect("mode toolbar must remain visible in source mode")
                .size
                .height
        );
        let preview_toggle = cx
            .debug_bounds("markdown-source-mode")
            .expect("preview toggle must replace the source toggle");
        cx.simulate_click(preview_toggle.center(), gpui::Modifiers::default());
        cx.run_until_parked();

        assert_eq!(
            crate::MarkdownViewMode::Wysiwyg,
            view.read_with(&cx, |view, _| view
                .markdown_sessions
                .values()
                .next()
                .unwrap()
                .state
                .mode)
        );
    }

    #[gpui::test]
    fn standalone_markdown_preview_uses_the_editable_wysiwyg_editor(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("editable.md");
        let source = "# Title\n\nBody\n";
        std::fs::write(&path, source).unwrap();
        let body_id = markdown_source::SourceMarkdownDocument::parse(source)
            .unwrap()
            .blocks[1]
            .id;
        let (window, view) = cx.update(|cx| {
            let mut view = None;
            let window = cx
                .open_window(WindowOptions::default(), |window, cx| {
                    let entity =
                        cx.new(|cx| NotesView::new_for_markdown_file(path.clone(), window, cx));
                    view = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                })
                .unwrap();
            (window, view.unwrap())
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();

        assert!(
            cx.debug_bounds("markdown-readonly-preview").is_none(),
            "the legacy read-only preview must not remain mounted"
        );
        cx.debug_bounds("markdown-wysiwyg-editor")
            .expect("the editable WYSIWYG editor must be rendered");
        let body = cx
            .debug_bounds(Box::leak(
                format!("markdown-preview-block-{}", body_id.0).into_boxed_str(),
            ))
            .expect("the Markdown editor must expose its rendered body block");
        cx.simulate_click(body.center(), gpui::Modifiers::default());
        cx.run_until_parked();

        let editor = view.read_with(&cx, |view, _| {
            let id = view.active_document_id.as_ref().unwrap();
            view.markdown_sessions.get(id).unwrap().preview.clone()
        });
        assert_eq!(
            Some(body_id),
            editor.read_with(&cx, |editor, _| editor.active_block())
        );
        let input = editor.read_with(&cx, |editor, _| editor.input_state());
        input.update_in(&mut cx, |input, window, cx| {
            input.set_selected_range(4..4, false, window, cx);
        });
        cx.simulate_keystrokes("X");
        cx.run_until_parked();

        view.read_with(&cx, |view, cx| {
            let id = view.active_document_id.as_ref().unwrap();
            let session = view.markdown_sessions.get(id).unwrap();
            assert_eq!(
                "# Title\n\nBodyX\n",
                session.source_document.lock().unwrap().source.as_str()
            );
            assert_eq!("# Title\n\nBodyX\n", session.preview.read(cx).source());
            assert_eq!(
                "# Title\n\nBodyX\n",
                session.source_editor.read(cx).value().as_ref()
            );
            assert!(session.preview.read(cx).is_dirty());
        });
    }

    #[gpui::test]
    fn standalone_open_and_mode_switch_preserve_file_byte_boundaries(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let cases = [
            ("empty.md", ""),
            ("bom-crlf.md", "\u{feff}# 标题\r\n\r\nBody  \r\nnext\r\n"),
            ("no-trailing-newline.md", "_italic_\n\n\nend"),
        ];
        for (name, source) in cases {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join(name);
            std::fs::write(&path, source.as_bytes()).unwrap();
            let (window, view) = cx.update(|cx| {
                let mut view = None;
                let window = cx
                    .open_window(WindowOptions::default(), |window, cx| {
                        let entity =
                            cx.new(|cx| NotesView::new_for_markdown_file(path.clone(), window, cx));
                        view = Some(entity.clone());
                        cx.new(|cx| Root::new(entity, window, cx))
                    })
                    .unwrap();
                (window, view.unwrap())
            });
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            visual.run_until_parked();
            let id = view.read_with(&visual, |view, _| view.active_document_id.clone().unwrap());
            view.update_in(&mut visual, |view, window, cx| {
                view.toggle_markdown_mode(id.clone(), window, cx);
                view.toggle_markdown_mode(id.clone(), window, cx);
            });
            visual.run_until_parked();
            assert_eq!(source.as_bytes(), std::fs::read(&path).unwrap());
        }
    }

    #[gpui::test]
    fn source_mode_undo_uses_the_shared_markdown_history(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("source-undo.md");
        std::fs::write(&path, "before").unwrap();
        let (window, view) = cx.update(|cx| {
            let mut view = None;
            let window = cx
                .open_window(WindowOptions::default(), |window, cx| {
                    let entity =
                        cx.new(|cx| NotesView::new_for_markdown_file(path.clone(), window, cx));
                    view = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                })
                .unwrap();
            (window, view.unwrap())
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        let id = view.read_with(&cx, |view, _| view.active_document_id.clone().unwrap());
        view.update_in(&mut cx, |view, window, cx| {
            view.toggle_markdown_mode(id, window, cx);
        });
        cx.run_until_parked();
        let source_editor = view.read_with(&cx, |view, _| {
            view.markdown_sessions
                .values()
                .next()
                .unwrap()
                .source_editor
                .clone()
        });
        view.update_in(&mut cx, |view, window, cx| {
            let session = view.markdown_sessions.values().next().unwrap();
            session.preview.update(cx, |editor, cx| {
                editor
                    .apply_source_value(
                        "after",
                        markdown_source::SourceSelection { anchor: 5, head: 5 },
                        window,
                        cx,
                    )
                    .unwrap();
            });
            session.source_editor.update(cx, |input, cx| {
                input.set_value("after", window, cx);
                input.focus(window, cx);
            });
        });
        cx.run_until_parked();
        view.read_with(&cx, |view, cx| {
            let session = view.markdown_sessions.values().next().unwrap();
            assert_eq!("after", session.preview.read(cx).source());
        });
        #[cfg(target_os = "macos")]
        cx.simulate_keystrokes("cmd-z");
        #[cfg(not(target_os = "macos"))]
        cx.simulate_keystrokes("ctrl-z");
        cx.run_until_parked();
        assert_eq!(
            "before",
            source_editor.read_with(&cx, |input, _| input.value().to_string())
        );
        view.read_with(&cx, |view, cx| {
            let session = view.markdown_sessions.values().next().unwrap();
            assert_eq!("before", session.preview.read(cx).source());
        });
    }

    #[gpui::test]
    fn clean_external_reload_updates_both_markdown_modes(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("reload.md");
        std::fs::write(&path, "before").unwrap();
        let (window, view) = cx.update(|cx| {
            let mut view = None;
            let window = cx
                .open_window(WindowOptions::default(), |window, cx| {
                    let entity =
                        cx.new(|cx| NotesView::new_for_markdown_file(path.clone(), window, cx));
                    view = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                })
                .unwrap();
            (window, view.unwrap())
        });
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        std::fs::write(&path, "external _change_").unwrap();
        view.update_in(&mut visual, |view, window, cx| {
            view.reload_active_markdown_from_disk(window, cx);
        });
        view.read_with(&visual, |view, cx| {
            let session = view
                .markdown_sessions
                .get(view.active_document_id.as_ref().unwrap())
                .unwrap();
            assert_eq!("external _change_", session.preview.read(cx).source());
            assert_eq!(
                "external _change_",
                session.source_editor.read(cx).value().as_ref()
            );
            assert_eq!(
                crate::markdown_session::MarkdownSyncState::Clean,
                session.state.sync_state
            );
        });
    }

    #[gpui::test]
    fn external_change_event_reloads_clean_session(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("watched.md");
        std::fs::write(&path, "before").unwrap();
        let (window, view) = cx.update(|cx| {
            let mut view = None;
            let window = cx
                .open_window(WindowOptions::default(), |window, cx| {
                    let entity =
                        cx.new(|cx| NotesView::new_for_markdown_file(path.clone(), window, cx));
                    view = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                })
                .unwrap();
            (window, view.unwrap())
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        std::fs::write(&path, "external").unwrap();
        let id = view.read_with(&cx, |view, _| view.active_document_id.clone().unwrap());
        view.update_in(&mut cx, |view, window, cx| {
            view.markdown_file_changed_on_disk(&id, window, cx);
        });
        view.read_with(&cx, |view, cx| {
            let session = view.markdown_sessions.values().next().unwrap();
            assert_eq!("external", session.preview.read(cx).source());
            assert!(matches!(session.state.sync_state, MarkdownSyncState::Clean));
        });
    }

    #[gpui::test]
    fn external_change_event_marks_dirty_session_as_conflicted(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dirty-watched.md");
        std::fs::write(&path, "before").unwrap();
        let (window, view) = cx.update(|cx| {
            let mut view = None;
            let window = cx
                .open_window(WindowOptions::default(), |window, cx| {
                    let entity =
                        cx.new(|cx| NotesView::new_for_markdown_file(path.clone(), window, cx));
                    view = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                })
                .unwrap();
            (window, view.unwrap())
        });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let id = view.read_with(&cx, |view, _| view.active_document_id.clone().unwrap());
        view.update_in(&mut cx, |view, window, cx| {
            let session = view.markdown_sessions.get(&id).unwrap();
            session.preview.update(cx, |editor, cx| {
                editor
                    .apply_source_value(
                        "local",
                        markdown_source::SourceSelection { anchor: 5, head: 5 },
                        window,
                        cx,
                    )
                    .unwrap();
            });
        });
        std::fs::write(&path, "external").unwrap();
        view.update_in(&mut cx, |view, window, cx| {
            view.markdown_file_changed_on_disk(&id, window, cx);
        });
        view.read_with(&cx, |view, cx| {
            let session = view.markdown_sessions.get(&id).unwrap();
            assert_eq!("local", session.preview.read(cx).source());
            assert!(matches!(
                session.state.sync_state,
                MarkdownSyncState::Conflict
            ));
        });
    }
}
