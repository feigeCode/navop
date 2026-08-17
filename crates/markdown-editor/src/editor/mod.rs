//! Top-level editor controller and window state.
//!
//! [`Editor`] owns window-level concerns such as view mode, save/close flow,
//! scroll state, and focus deferral. The runtime block tree itself lives in
//! [`DocumentTree`], which centralizes structural mutations and cached visible
//! order metadata.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::*;

use self::context_menu::{ContextMenuTargetState, TableInsertDialogState};
use self::enlarged::EnlargedBlockState;
use self::tree::DocumentTree;
use crate::EditorHostServices;
use crate::components::{
    Block, BlockKind, BlockRecord, FootnoteDefinitionBinding, FootnoteReferenceLocation,
    FootnoteRegistry, FootnoteResolvedOccurrence, ImageReferenceDefinitions, InlineTextTree,
    LinkReferenceDefinitions, parse_image_reference_definitions, parse_link_reference_definitions,
};
use crate::components::{
    TableAxisHighlight, TableAxisKind, TableAxisMarker, TableCellPosition, TableColumnAlignment,
    TableData, TableRuntime, UndoCaptureKind, serialize_table_cell_markdown,
};
use crate::theme::{Theme, ThemeManager};
mod context_menu;
mod document;
mod enlarged;
mod events;
mod history;
mod host;
mod persistence;
mod render;
mod runtime_context;
mod selection;
mod source_mapping;
mod table_edit;
mod table_menu;
mod tree;
mod window_state;

pub use self::host::EditorEvent;

/// Link navigation request deferred until a `Window` is available.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingOpenLink {
    pub(crate) prompt_target: String,
    pub(crate) open_target: String,
}

/// Top-level controller that owns editor-wide state and delegates tree
/// mutations to [`DocumentTree`].
///
/// The editor subscribes to every [`BlockEvent`](crate::components::BlockEvent)
/// emitted by child blocks. Structural changes are handled centrally so focus,
/// scrolling, dirty tracking, and serialization stay synchronized.
pub struct Editor {
    document: DocumentTree,
    table_cells: HashMap<EntityId, TableCellBinding>,
    /// Which view the editor is currently presenting.
    pub(crate) view_mode: ViewMode,
    /// Deferred focus target applied during render when a [`Window`] is
    /// available.
    pending_focus: Option<EntityId>,
    active_entity_id: Option<EntityId>,
    pending_scroll_active_block_into_view: bool,
    pending_scroll_recheck_after_layout: bool,
    pending_open_link: Option<PendingOpenLink>,
    document_dirty: bool,
    revision: u64,
    file_path: Option<PathBuf>,
    scroll_handle: ScrollHandle,
    /// Last painted bounds of this editor in window coordinates. Pointer
    /// events also use window coordinates, while editor overlays are positioned
    /// relative to this root.
    root_bounds: Bounds<Pixels>,
    last_scroll_viewport_size: Option<Size<Pixels>>,
    /// Last frame's visible block ids, to detect structural edits so the height
    /// cache is refreshed only when the row/block mapping is unchanged.
    prev_visible_block_ids: Vec<EntityId>,
    /// Per-row footprint (height plus trailing gap), keyed by the row's first
    /// block. Scroll-invariant, unlike raw painted positions, so windowing from
    /// their running sum stays correct as the document scrolls. Filled as rows
    /// paint; unknown rows use a minimum-height estimate.
    row_stride_cache: HashMap<EntityId, f32>,
    /// Content column the cached footprints were measured at. Rows rewrap when it
    /// changes, so entries from another width are discarded rather than reused.
    row_stride_width: Option<f32>,
    /// Where last frame's run sat among the scroll container's children.
    prev_mounted_run: Option<MountedRun>,
    context_menu_target: ContextMenuTargetState,
    table_insert_dialog: Option<TableInsertDialogState>,
    enlarged_block: Option<EnlargedBlockState>,
    table_axis_preview: Option<TableAxisSelection>,
    table_axis_selection: Option<TableAxisSelection>,
    cross_block_selection: Option<CrossBlockSelection>,
    cross_block_drag: Option<CrossBlockDrag>,
    rendered_select_all_cycle: Option<RenderedSelectAllCycle>,
    scrollbar_hovered: bool,
    scrollbar_visible_until: Instant,
    scrollbar_fade_task: Option<Task<()>>,
    /// Forces a repaint shortly after a pending scroll-into-view that could
    /// not be satisfied yet (the target block has no measured bounds), so the
    /// scroll lands on the next frame instead of waiting for the cursor blink.
    scroll_recheck_task: Option<Task<()>>,
    scrollbar_drag: Option<ScrollbarDragSession>,
    undo_history: Vec<HistoryEntry>,
    redo_history: Vec<HistoryEntry>,
    pending_undo_capture: Option<PendingUndoCapture>,
    last_selection_snapshot: UndoSelectionSnapshot,
    last_stable_source_text: String,
    history_restore_in_progress: bool,
    image_reference_definitions: Arc<ImageReferenceDefinitions>,
    link_reference_definitions: Arc<LinkReferenceDefinitions>,
    footnote_registry: Arc<FootnoteRegistry>,
    host_services: Arc<EditorHostServices>,
    effective_theme: Option<Arc<Theme>>,
    host_services_revision: u64,
}

/// Runtime binding between a table block and one cell editor.
#[derive(Clone)]
struct TableCellBinding {
    table_block: Entity<Block>,
    cell: Entity<Block>,
    position: TableCellPosition,
}

/// Selected row or column in a rendered native table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TableAxisSelection {
    table_block_id: EntityId,
    kind: TableAxisKind,
    index: usize,
}

/// Pixel geometry for the custom editor scrollbar.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScrollbarGeometry {
    track_height: f32,
    thumb_height: f32,
    thumb_top: f32,
    max_scroll_y: f32,
}

/// Windowing result: the run of rows to mount, plus the spacer heights standing
/// in for the culled rows. `top_h` is the spacer directly above the run and
/// `bottom_h` the one closing out the document.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RenderWindow {
    run_start: usize,
    run_end: usize,
    top_h: f32,
    bottom_h: f32,
    focus_island: Option<FocusIsland>,
}

/// Where a frame's mounted run sat among the scroll container's children, so the
/// next frame can read its recorded bounds back by index. `child_count` is what
/// makes that mapping checkable rather than assumed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct MountedRun {
    row_start: usize,
    row_end: usize,
    child_base: usize,
    child_count: usize,
}

/// Focused row mounted on its own, away from the run. Its position relative to
/// the run follows from `row`, and `lead_h` is the spacer directly above it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FocusIsland {
    row: usize,
    lead_h: f32,
}

/// Active drag session for the custom scrollbar thumb.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScrollbarDragSession {
    pointer_offset_y: f32,
    track_height: f32,
    thumb_height: f32,
    max_scroll_y: f32,
}

/// Source-mode selection snapshot stored with undo history.
#[derive(Clone, Debug, PartialEq, Eq)]
struct UndoSelectionSnapshot {
    range: std::ops::Range<usize>,
    reversed: bool,
}

/// One undo history entry containing source text and selection state.
#[derive(Clone, Debug)]
struct HistoryEntry {
    source_text: String,
    selection: UndoSelectionSnapshot,
    timestamp: Instant,
    kind: UndoCaptureKind,
}

/// Deferred undo capture used to coalesce adjacent typing edits.
#[derive(Clone, Debug)]
struct PendingUndoCapture {
    snapshot: HistoryEntry,
}

/// Cross-block selection endpoint in visible block order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CrossBlockSelectionEndpoint {
    pub(super) entity_id: EntityId,
    pub(super) offset: usize,
}

/// Editor-level selection spanning two visible block endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CrossBlockSelection {
    pub(super) anchor: CrossBlockSelectionEndpoint,
    pub(super) focus: CrossBlockSelectionEndpoint,
}

/// Drag state while creating or extending a cross-block selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CrossBlockDrag {
    pub(super) anchor: CrossBlockSelectionEndpoint,
}

/// Short-lived Ctrl/Cmd+A press counter for rendered-mode selection upgrade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RenderedSelectAllCycle {
    entity_id: EntityId,
    count: u8,
    last_pressed_at: Instant,
}

/// Mapping from one visible block's text range to canonical Markdown offsets.
#[derive(Clone)]
pub(super) struct SourceTargetMapping {
    entity: Entity<Block>,
    full_source_range: std::ops::Range<usize>,
    content_to_source: Vec<usize>,
    source_to_content: Vec<usize>,
}

/// The two editing views the editor can present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    /// Rich rendered view where each block is styled by its semantic kind.
    Rendered,
    /// Plain source view where the full Markdown document is edited as a
    /// single raw buffer.
    Source,
}

impl Editor {
    const HISTORY_LIMIT: usize = 200;
    const HISTORY_COALESCE_WINDOW: Duration = Duration::from_millis(1_000);
    const RENDERED_SELECT_ALL_CYCLE_WINDOW: Duration = Duration::from_millis(750);

    /// Builds an editor intended to be hosted inside another GPUI view.
    pub fn from_markdown_embedded(cx: &mut Context<Self>, markdown: String) -> Self {
        Self::from_markdown_embedded_with_host(cx, markdown, EditorHostServices::default())
    }

    /// Test compatibility constructor for the editor-core tests migrated from
    /// Velotype. Production callers must use the embedded constructors above.
    #[cfg(test)]
    pub(crate) fn from_markdown(
        cx: &mut Context<Self>,
        markdown: String,
        file_path: Option<PathBuf>,
    ) -> Self {
        let mut editor = Self::from_markdown_with_host_options(
            cx,
            markdown,
            EditorHostServices::default(),
            false,
        );
        editor.file_path = file_path;
        editor
    }

    /// Builds an embedded editor with host rendering services installed before
    /// its initial block tree is synchronized.
    pub fn from_markdown_embedded_with_host(
        cx: &mut Context<Self>,
        markdown: String,
        host_services: EditorHostServices,
    ) -> Self {
        Self::from_markdown_with_host_options(cx, markdown, host_services, true)
    }

    fn from_markdown_with_host_options(
        cx: &mut Context<Self>,
        markdown: String,
        host_services: EditorHostServices,
        ensure_trailing_structural_paragraph: bool,
    ) -> Self {
        let normalized = Self::normalize_markdown(&markdown);
        let roots = Self::build_rendered_roots_with_options(
            cx,
            &normalized,
            ensure_trailing_structural_paragraph,
        );

        let mut document = DocumentTree::new(roots);
        document.rebuild_metadata_and_snapshot(cx);
        let pending_focus = document.first_root().map(|block| block.entity_id());
        let host_services = Arc::new(host_services);
        let effective_theme = Self::derive_effective_theme(&host_services, cx);

        let mut editor = Self {
            document,
            table_cells: HashMap::new(),
            view_mode: ViewMode::Rendered,
            pending_focus,
            active_entity_id: pending_focus,
            pending_scroll_active_block_into_view: true,
            pending_scroll_recheck_after_layout: true,
            pending_open_link: None,
            document_dirty: false,
            revision: 0,
            file_path: None,
            scroll_handle: ScrollHandle::new(),
            root_bounds: Bounds::default(),
            last_scroll_viewport_size: None,
            prev_visible_block_ids: Vec::new(),
            row_stride_cache: HashMap::new(),
            row_stride_width: None,
            prev_mounted_run: None,
            context_menu_target: ContextMenuTargetState::default(),
            table_insert_dialog: None,
            enlarged_block: None,
            table_axis_preview: None,
            table_axis_selection: None,
            cross_block_selection: None,
            cross_block_drag: None,
            rendered_select_all_cycle: None,
            scrollbar_hovered: false,
            scrollbar_visible_until: Instant::now(),
            scrollbar_fade_task: None,
            scroll_recheck_task: None,
            scrollbar_drag: None,
            undo_history: Vec::new(),
            redo_history: Vec::new(),
            pending_undo_capture: None,
            last_selection_snapshot: Self::empty_selection_snapshot(),
            last_stable_source_text: normalized,
            history_restore_in_progress: false,
            image_reference_definitions: Arc::default(),
            link_reference_definitions: Arc::default(),
            footnote_registry: Arc::default(),
            host_services,
            effective_theme,
            host_services_revision: 0,
        };
        editor.rebuild_table_runtimes(cx);
        editor.rebuild_image_runtimes(cx);
        editor.pending_focus = editor.first_focusable_entity_id(cx);
        editor.active_entity_id = editor.pending_focus;
        editor.refresh_stable_document_snapshot(cx);
        editor
    }

    fn derive_effective_theme(host_services: &EditorHostServices, cx: &App) -> Option<Arc<Theme>> {
        host_services.theme_override().map(|host_theme| {
            let base = cx.global::<ThemeManager>().current();
            Arc::new(Theme::with_host_palette(base, host_theme))
        })
    }

    pub(crate) fn effective_theme(&self, cx: &App) -> Arc<Theme> {
        self.effective_theme
            .clone()
            .unwrap_or_else(|| cx.global::<ThemeManager>().current_arc())
    }

    fn last_root_strands_editor(roots: &[Entity<Block>], cx: &App) -> bool {
        roots.last().is_some_and(|block| {
            let block = block.read(cx);
            let kind = block.kind();
            kind.is_atomic_structural()
                || kind.is_quote_container()
                || kind.is_footnote_definition()
                || block.renders_as_standalone_image()
        })
    }

    pub(super) fn normalize_markdown(markdown: &str) -> String {
        markdown.replace("\r\n", "\n").replace('\r', "\n")
    }

    pub(super) fn build_rendered_roots(
        cx: &mut Context<Self>,
        markdown: &str,
    ) -> Vec<Entity<Block>> {
        Self::build_rendered_roots_with_options(cx, markdown, true)
    }

    fn build_rendered_roots_with_options(
        cx: &mut Context<Self>,
        markdown: &str,
        ensure_trailing_structural_paragraph: bool,
    ) -> Vec<Entity<Block>> {
        let mut roots = Self::build_root_blocks_from_markdown(cx, markdown);
        if roots.is_empty() {
            roots.push(Self::new_block(cx, BlockRecord::paragraph(String::new())));
        }
        if ensure_trailing_structural_paragraph && Self::last_root_strands_editor(&roots, cx) {
            roots.push(Self::new_block(cx, BlockRecord::paragraph(String::new())));
        }
        roots
    }

    /// Returns the current canonical Markdown serialization.
    pub fn markdown(&self, cx: &App) -> String {
        self.current_document_source(cx)
    }

    /// Returns the number of real root blocks in the document tree.
    pub fn root_block_count(&self) -> usize {
        self.document.root_count()
    }

    /// Returns whether the last root is a real paragraph block.
    pub fn trailing_block_is_paragraph(&self, cx: &App) -> bool {
        self.document
            .root_blocks()
            .last()
            .is_some_and(|block| block.read(cx).kind() == BlockKind::Paragraph)
    }

    /// Returns whether the document has unsaved edits.
    pub fn is_dirty(&self) -> bool {
        self.document_dirty
    }
}
