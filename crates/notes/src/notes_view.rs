use crate::notes_notifications::notify_operation_error;
use crate::{DocumentFormat, FileDocumentPersistence, NodeKind, NotesStorage, TreeRow, TreeState};
use cditor_app::{Editor, EditorHandle};
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Window,
};
use gpui_component::input::InputState;
use one_core::tab_container::TabContentEvent;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) enum NotesLoadState {
    NeedsSetup,
    Ready,
    Failed(SharedString),
}

pub(crate) struct CachedEditor {
    pub relative_path: PathBuf,
    pub handle: EditorHandle,
    pub persistence: FileDocumentPersistence,
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
    pub(crate) notebook_name: SharedString,
    pub(crate) setup_name: Entity<InputState>,
    pub(crate) setup_description: Entity<InputState>,
    pub(crate) dialog_subscription: Option<Subscription>,
    pub(crate) focus_handle: FocusHandle,
}

impl NotesView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let setup_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("笔记本名称")
                .default_value("我的笔记")
        });
        let setup_description = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("描述（可选）")
                .default_value("Navop 本地笔记")
        });
        let mut view = Self {
            storage: None,
            load_state: NotesLoadState::NeedsSetup,
            tree: TreeState::default(),
            rows: Vec::new(),
            editors: HashMap::new(),
            markdown_sessions: HashMap::new(),
            active_editor: None,
            active_document_id: None,
            current_directory: PathBuf::new(),
            notebook_name: "Notes".into(),
            setup_name,
            setup_description,
            dialog_subscription: None,
            focus_handle: cx.focus_handle(),
        };
        if let Err(error) = view.initialize(window, cx) {
            view.load_state = NotesLoadState::Failed(error.to_string().into());
        }
        view
    }

    fn initialize(&mut self, window: &mut Window, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let storage = NotesStorage::open(NotesStorage::default_root()?)?;
        let metadata = storage.load_notebook()?;
        self.storage = Some(storage);
        let Some(metadata) = metadata else {
            return Ok(());
        };
        self.notebook_name = metadata.name.into();
        self.load_state = NotesLoadState::Ready;
        self.tree = TreeState::from_ui_state(self.storage()?.load_state()?);
        self.refresh_tree(window, cx)
    }

    pub(crate) fn create_notebook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.setup_name.read(cx).value().trim().to_owned();
        let description = self.setup_description.read(cx).value().trim().to_owned();
        let result = self.storage().and_then(|storage| {
            storage
                .create_notebook(&name, &description)
                .map(|metadata| metadata.name)
        });
        match result {
            Ok(name) => {
                self.notebook_name = name.into();
                self.load_state = NotesLoadState::Ready;
                if let Err(error) = self.refresh_tree(window, cx) {
                    notify_operation_error(window, cx, error);
                }
            }
            Err(error) => notify_operation_error(window, cx, error),
        }
        cx.notify();
    }

    pub(crate) fn refresh_tree(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let nodes = self.storage()?.scan_tree()?;
        self.rows = self.tree.project(&nodes);
        self.tree.select_fallback(&self.rows);
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
            let handle = Editor::builder()
                .document_id(descriptor.document_id.clone())
                .persistence(persistence.clone())
                .autosave(AUTOSAVE_INTERVAL)
                .on_event(move |event| {
                    let _ = event_sender.try_send(event);
                })
                .build(cx)?;
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

impl EventEmitter<TabContentEvent> for NotesView {}

impl Focusable for NotesView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.load_state {
            NotesLoadState::NeedsSetup => self.setup_name.read(cx).focus_handle(cx),
            NotesLoadState::Ready => self.focus_handle.clone(),
            NotesLoadState::Failed(_) => self.focus_handle.clone(),
        }
    }
}
