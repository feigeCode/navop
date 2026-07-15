use crate::{FileDocumentPersistence, NodeKind, NotesStorage, TreeRow, TreeState};
use cditor_app::{Editor, EditorHandle, EditorSaveState};
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, SharedString,
    Subscription, Task, Window,
};
use gpui_component::{Icon, IconName, Sizable, Size, input::InputState};
use one_core::tab_container::{TabContent, TabContentEvent};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(1);
const CLOSE_POLL_INTERVAL: Duration = Duration::from_millis(50);
const CLOSE_POLL_ATTEMPTS: usize = 100;

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
    pub(crate) active_editor: Option<EditorHandle>,
    pub(crate) current_directory: PathBuf,
    pub(crate) notebook_name: SharedString,
    pub(crate) setup_name: Entity<InputState>,
    pub(crate) setup_description: Entity<InputState>,
    pub(crate) error: Option<SharedString>,
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
            active_editor: None,
            current_directory: PathBuf::new(),
            notebook_name: "Notes".into(),
            setup_name,
            setup_description,
            error: None,
            dialog_subscription: None,
            focus_handle: cx.focus_handle(),
        };
        if let Err(error) = view.initialize(cx) {
            view.load_state = NotesLoadState::Failed(error.to_string().into());
        }
        view
    }

    fn initialize(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let storage = NotesStorage::open(NotesStorage::default_root()?)?;
        let metadata = storage.load_notebook()?;
        self.storage = Some(storage);
        let Some(metadata) = metadata else {
            return Ok(());
        };
        self.notebook_name = metadata.name.into();
        self.load_state = NotesLoadState::Ready;
        self.tree = TreeState::from_ui_state(self.storage()?.load_state()?);
        self.refresh_tree(cx)
    }

    pub(crate) fn create_notebook(&mut self, cx: &mut Context<Self>) {
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
                if let Err(error) = self.refresh_tree(cx) {
                    self.set_error(error);
                }
            }
            Err(error) => self.set_error(error),
        }
        cx.notify();
    }

    pub(crate) fn refresh_tree(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let nodes = self.storage()?.scan_tree()?;
        self.rows = self.tree.project(&nodes);
        self.tree.select_fallback(&self.rows);
        self.storage()?.save_state(&self.tree.to_ui_state())?;
        if let Some(path) = self.tree.selected_document.clone() {
            self.open_document(&path, cx)?;
        } else {
            self.active_editor = None;
        }
        Ok(())
    }

    fn open_document(&mut self, path: &Path, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let descriptor = self.storage()?.descriptor(path)?;
        let handle = if let Some(cached) = self.editors.get(&descriptor.document_id) {
            cached.handle.clone()
        } else {
            let persistence = FileDocumentPersistence::new(descriptor.absolute_path);
            let handle = Editor::builder()
                .document_id(descriptor.document_id.clone())
                .persistence(persistence.clone())
                .autosave(AUTOSAVE_INTERVAL)
                .build(cx)?;
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
        self.active_editor = Some(handle);
        Ok(())
    }

    pub(crate) fn select_row(&mut self, path: PathBuf, kind: NodeKind, cx: &mut Context<Self>) {
        let result = if kind == NodeKind::Directory {
            self.current_directory = path.clone();
            self.tree.toggle_directory(&path);
            self.refresh_tree(cx)
        } else {
            self.current_directory = path.parent().unwrap_or(Path::new("")).to_path_buf();
            self.tree.selected_document = Some(path.clone());
            self.open_document(&path, cx)
                .and_then(|_| self.storage()?.save_state(&self.tree.to_ui_state()))
        };
        if let Err(error) = result {
            self.set_error(error);
        }
        cx.notify();
    }

    pub(crate) fn storage(&self) -> anyhow::Result<&NotesStorage> {
        self.storage
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("notes storage is unavailable"))
    }

    pub(crate) fn set_error(&mut self, error: impl std::fmt::Display) {
        self.error = Some(error.to_string().into());
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

impl TabContent for NotesView {
    fn content_key(&self) -> &'static str {
        "Notes"
    }

    fn title(&self, _cx: &App) -> SharedString {
        self.notebook_name.clone()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::BookOpen.color().with_size(Size::Medium))
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let dirty = self
            .editors
            .values()
            .filter(|cached| cached.handle.is_dirty(cx))
            .map(|cached| cached.handle.clone())
            .collect::<Vec<_>>();
        if dirty.is_empty() {
            return Task::ready(true);
        }
        for handle in &dirty {
            if let Err(error) = handle.save(cx) {
                self.set_error(error);
                return Task::ready(false);
            }
        }
        let executor = cx.background_executor().clone();
        cx.spawn(async move |_view, cx| {
            for _ in 0..CLOSE_POLL_ATTEMPTS {
                executor.timer(CLOSE_POLL_INTERVAL).await;
                let states = cx.update(|cx| {
                    dirty
                        .iter()
                        .map(|handle| handle.save_state(cx))
                        .collect::<Vec<_>>()
                });
                if states
                    .iter()
                    .any(|state| matches!(state, EditorSaveState::SaveFailed { .. }))
                {
                    return false;
                }
                if states.iter().all(|state| {
                    matches!(state, EditorSaveState::Clean | EditorSaveState::Disabled)
                }) {
                    return true;
                }
            }
            false
        })
    }
}
