use crate::document_rendering::NavopDocumentRendererProvider;
use crate::notes_notifications::notify_operation_error;
use crate::syntax_highlighting::NavopSyntaxHighlightProvider;
use crate::theme_provider::{MarkdownEditorTheme, NavopThemeProvider, cditor_theme};
use crate::{
    DocumentDescriptor, DocumentFormat, FileDocumentPersistence, NodeKind, NotesStorage, TreeRow,
    TreeState,
};
use anyhow::{Context as _, bail};
use cditor_app::{AiProvider, Editor, EditorHandle};
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Window,
};
use gpui_component::input::InputState;
use one_core::settings::AppSettings;
use one_core::tab_container::TabContentEvent;
use rust_i18n::t;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) enum NotesLoadState {
    NeedsLocation,
    Ready,
}

pub(crate) struct CachedEditor {
    pub relative_path: PathBuf,
    pub handle: EditorHandle,
    pub persistence: FileDocumentPersistence,
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
    pub(crate) editors: HashMap<String, CachedEditor>,
    pub(crate) markdown_sessions: HashMap<String, crate::markdown_session::MarkdownSession>,
    pub(crate) active_editor: Option<EditorHandle>,
    pub(crate) active_document_id: Option<String>,
    pub(crate) current_directory: PathBuf,
    pub(crate) selected_sidebar_path: Option<PathBuf>,
    pub(crate) context_menu_path: Option<PathBuf>,
    pub(crate) notebook_name: SharedString,
    pub(crate) ai_provider: Option<Arc<dyn AiProvider>>,
    pub(crate) syntax_highlight_provider: Arc<NavopSyntaxHighlightProvider>,
    pub(crate) document_renderer_provider: Option<Arc<NavopDocumentRendererProvider>>,
    pub(crate) theme_provider: Arc<NavopThemeProvider>,
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
                    let _ = session.preview.focus(cx);
                }
            }
            return;
        }
        if let Some(editor) = self.active_editor.as_ref() {
            let _ = editor.focus(cx);
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
        let ai_provider = match crate::ai_provider::build_provider(cx) {
            Ok(provider) => provider,
            Err(error) => {
                notify_operation_error(window, cx, error);
                None
            }
        };
        let resolved_theme = editor_theme
            .clone()
            .unwrap_or_else(|| MarkdownEditorTheme::from_app(cx));
        let syntax_highlight_provider = Arc::new(NavopSyntaxHighlightProvider::new(
            resolved_theme.highlight_theme.clone(),
            resolved_theme.background,
            resolved_theme.foreground,
        ));
        let document_renderer_provider = cx
            .try_global::<extension_runtime::GlobalExtensionRuntimeCatalog>()
            .cloned()
            .map(NavopDocumentRendererProvider::new)
            .map(Arc::new);
        let theme_provider = Arc::new(NavopThemeProvider::new(cditor_theme(
            resolved_theme.background,
            resolved_theme.foreground,
            resolved_theme.muted_foreground,
            resolved_theme.border,
            resolved_theme.primary,
            resolved_theme.danger,
        )));
        Self {
            storage: None,
            load_state,
            tree: TreeState::default(),
            rows: Vec::new(),
            editors: HashMap::new(),
            markdown_sessions: HashMap::new(),
            active_editor: None,
            active_document_id: None,
            current_directory: PathBuf::new(),
            selected_sidebar_path: None,
            context_menu_path: None,
            notebook_name,
            ai_provider,
            syntax_highlight_provider,
            document_renderer_provider,
            theme_provider,
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
            self.active_editor = None;
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
        if descriptor.format == DocumentFormat::Markdown {
            return self.open_markdown_document(descriptor, window, cx);
        }
        let document_id = descriptor.document_id.clone();
        let handle = if let Some(cached) = self.editors.get(&descriptor.document_id) {
            cached.handle.clone()
        } else {
            let persistence = FileDocumentPersistence::new(descriptor.absolute_path);
            let (event_sender, events) = smol::channel::unbounded();
            let mut builder = Editor::builder()
                .document_id(descriptor.document_id.clone())
                .persistence(persistence.clone())
                .autosave(AUTOSAVE_INTERVAL)
                .on_event(move |event| {
                    let _ = event_sender.try_send(event);
                });
            builder = match self.ai_provider.clone() {
                Some(provider) => builder.ai_provider_arc(provider),
                None => builder.without_ai(),
            };
            builder = builder.syntax_highlight_provider_arc(self.syntax_highlight_provider.clone());
            builder = builder.theme_provider_arc(self.theme_provider.clone());
            builder = builder.source_editor_provider_arc(Arc::new(
                crate::source_editor_provider::NotesSourceEditorProvider,
            ));
            if let Some(provider) = self.document_renderer_provider.clone() {
                builder = builder.document_renderer_provider_arc(provider);
            }
            let handle = builder.build(cx)?;
            self.restore_ai_model(&handle, cx);
            self.observe_editor_events(events, window, cx);
            self.editors.insert(
                descriptor.document_id,
                CachedEditor {
                    relative_path: descriptor.relative_path,
                    handle: handle.clone(),
                    persistence,
                },
            );
            handle
        };
        self.active_document_id = Some(document_id);
        self.active_editor = Some(handle);
        Ok(())
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

    pub(crate) fn restore_ai_model(&self, handle: &EditorHandle, cx: &mut App) {
        let Some(model_id) = cx
            .try_global::<AppSettings>()
            .and_then(|settings| settings.ai_chat.notes_model_id.clone())
        else {
            return;
        };
        if let Err(error) = handle.select_ai_model(&model_id, cx) {
            tracing::debug!(%error, model_id, "saved Notes AI model is unavailable");
        }
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
    fn standalone_preview_and_mode_switch_preserve_source(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::init(cx);
        });
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("round-trip.md");
        let source = concat!(
            "> <https://example.com/path_(item)>\n\n",
            "[README](README_CN.md) and `snake_case(value)`\n",
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
            assert!(session.source_authoritative);
            assert!(session.preview.is_readonly(cx));
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
}
