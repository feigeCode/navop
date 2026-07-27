use crate::{
    MarkdownBlockRenderArtifact, MarkdownBlockRenderProvider, MarkdownEditorTheme,
    MarkdownProjection,
};
use gpui::{App, Context, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle, Window};
use gpui_component::{VirtualListScrollHandle, input::InputState};
use markdown_source::{
    PatchError, SourceEdit, SourceEditOrigin, SourceHistory, SourceMarkdownDocument,
    SourceSelection, SourceTransaction,
};
use std::collections::{HashMap, HashSet};

mod activation;
mod history_operations;
mod media_operations;
mod operations;
mod projection_styles;
mod render;
mod setup;
mod surface;
mod sync;
mod table_operations;
mod text_diff;
mod types;
use projection_styles::projection_highlights;
use setup::{apply_projection_styles, create_input, create_property_input};
use surface::{MarkdownEditSurface, MarkdownSurfaceKey};
use text_diff::{common_prefix, common_suffix};
pub use types::{MarkdownEditorError, MarkdownEditorEvent};

pub struct MarkdownEditor {
    input: Entity<InputState>,
    image_alt_input: Entity<InputState>,
    image_destination_input: Entity<InputState>,
    history: SourceHistory,
    projection: MarkdownProjection,
    active_block: Option<markdown_source::SourceNodeId>,
    active_table_cell: Option<markdown_source::TableCellAddress>,
    table_grid_hover: Option<(usize, usize)>,
    theme: MarkdownEditorTheme,
    dirty: bool,
    syncing_input: bool,
    surfaces: HashMap<MarkdownSurfaceKey, MarkdownEditSurface>,
    active_surface: Option<MarkdownSurfaceKey>,
    empty_surface_range: std::ops::Range<usize>,
    source_mode_selection: SourceSelection,
    pending_newline: Option<(MarkdownSurfaceKey, usize)>,
    block_scroll: VirtualListScrollHandle,
    document_scroll: ScrollHandle,
    block_render_provider: Option<MarkdownBlockRenderProvider>,
    block_render_artifacts: HashMap<markdown_source::SourceNodeId, MarkdownBlockRenderArtifact>,
    block_render_sources: HashMap<markdown_source::SourceNodeId, String>,
    pending_block_renders: HashMap<markdown_source::SourceNodeId, String>,
    block_render_errors: HashMap<markdown_source::SourceNodeId, String>,
    block_render_cache:
        HashMap<render::block_renderer::RenderCacheKey, render::block_renderer::CachedRender>,
    pending_shared_renders:
        HashMap<render::block_renderer::RenderCacheKey, Vec<render::block_renderer::RenderWaiter>>,
    block_render_generation: u64,
    inline_math_artifacts: HashMap<String, MarkdownBlockRenderArtifact>,
    pending_inline_math_renders: HashSet<String>,
    failed_inline_math_renders: HashSet<String>,
    measured_block_heights: HashMap<markdown_source::SourceNodeId, gpui::Pixels>,
}

impl MarkdownEditor {
    pub fn new(
        source: impl Into<String>,
        theme: MarkdownEditorTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Self, MarkdownEditorError> {
        let document = SourceMarkdownDocument::parse(source.into())?;
        Ok(Self::from_document(document, theme, window, cx))
    }

    pub fn from_document(
        document: SourceMarkdownDocument,
        theme: MarkdownEditorTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let source_len = document.source.len();
        let projection = MarkdownProjection::build(&document, None);
        let input = create_input(&projection.text, window, cx);
        let image_alt_input = create_property_input(window, cx);
        let image_destination_input = create_property_input(window, cx);
        apply_projection_styles(&input, &projection, &theme, cx);
        let mut editor = Self {
            input,
            image_alt_input,
            image_destination_input,
            history: SourceHistory::new(document),
            projection,
            active_block: None,
            active_table_cell: None,
            table_grid_hover: None,
            theme,
            dirty: false,
            syncing_input: false,
            surfaces: HashMap::new(),
            active_surface: None,
            empty_surface_range: 0..source_len,
            source_mode_selection: SourceSelection::default(),
            pending_newline: None,
            block_scroll: VirtualListScrollHandle::new(),
            document_scroll: ScrollHandle::new(),
            block_render_provider: None,
            block_render_artifacts: HashMap::new(),
            block_render_sources: HashMap::new(),
            pending_block_renders: HashMap::new(),
            block_render_errors: HashMap::new(),
            block_render_cache: HashMap::new(),
            pending_shared_renders: HashMap::new(),
            block_render_generation: 0,
            inline_math_artifacts: HashMap::new(),
            pending_inline_math_renders: HashSet::new(),
            failed_inline_math_renders: HashSet::new(),
            measured_block_heights: HashMap::new(),
        };
        editor.initialize_surfaces(window, cx);
        editor
    }

    pub fn source(&self) -> &str {
        &self.history.document().source
    }

    pub fn revision(&self) -> u64 {
        self.history.document().revision
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn input_state(&self) -> Entity<InputState> {
        self.input.clone()
    }

    pub fn uses_virtual_layout(&self) -> bool {
        render::layout_metrics::should_virtualize(&self.history.document().blocks)
    }

    pub fn vertical_scroll_range(&self) -> gpui::Pixels {
        if self.uses_virtual_layout() {
            self.block_scroll.max_offset().y
        } else {
            self.document_scroll.max_offset().y
        }
    }

    pub fn vertical_scroll_offset(&self) -> gpui::Pixels {
        if self.uses_virtual_layout() {
            self.block_scroll.offset().y
        } else {
            self.document_scroll.offset().y
        }
    }

    pub fn table_grid_hover(&self) -> Option<(usize, usize)> {
        self.table_grid_hover
    }

    pub fn set_block_render_provider(
        &mut self,
        provider: Option<MarkdownBlockRenderProvider>,
        cx: &mut Context<Self>,
    ) {
        self.block_render_provider = provider;
        self.reset_block_renders();
        cx.notify();
    }

    /// Retry a failed math or Mermaid block render without disturbing editor
    /// selection, layout or scroll position.
    pub fn retry_block_render(
        &mut self,
        block_id: markdown_source::SourceNodeId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(block) = self.history.document().block_by_id(block_id) else {
            return false;
        };
        let Some((_, source, request)) = self.block_render_request(block) else {
            return false;
        };
        let key = render::block_renderer::RenderCacheKey::from_request(&request);
        self.block_render_cache.remove(&key);
        self.block_render_sources.remove(&block_id);
        self.block_render_artifacts.remove(&block_id);
        self.block_render_errors.remove(&block_id);
        self.pending_block_renders.remove(&block_id);
        self.enqueue_block_render(block_id, source, request, cx);
        cx.notify();
        true
    }

    pub fn projected_text(&self) -> &str {
        &self.projection.text
    }

    pub fn active_inline_math_preview_count(&self) -> usize {
        self.history
            .document()
            .blocks
            .iter()
            .flat_map(|block| block.inline_nodes.iter())
            .filter(|node| {
                matches!(
                    node.kind,
                    markdown_source::SourceInlineKind::InlineMath { .. }
                ) && self.projection.active_inline != Some(node.id)
                    && node.content_range.as_ref().is_some_and(|range| {
                        self.inline_math_artifacts
                            .contains_key(&self.history.document().source[range.clone()])
                    })
            })
            .count()
    }

    pub fn edit_projected_value(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, PatchError> {
        let projection = self.projection.clone();
        self.edit_value_with_projection(&projection, value, window, cx)
    }

    fn edit_value_with_projection(
        &mut self,
        projection: &MarkdownProjection,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, PatchError> {
        let Some(edit) = projection.edit_for_value(value) else {
            return Ok(false);
        };
        let selection_before = self.source_selection(cx);
        let source_cursor = edit.source_range.start + edit.replacement.len();
        let selection_after = SourceSelection {
            anchor: source_cursor,
            head: source_cursor,
        };
        let active_table_cell = self.active_table_cell;
        self.apply_projection_edit(edit, selection_before, selection_after)?;
        self.dirty = true;
        if let Some(address) = active_table_cell {
            self.sync_table_cell(address, source_cursor, window, cx);
        } else {
            self.sync_projection(source_cursor, window, cx);
        }
        self.emit_changed(cx);
        Ok(true)
    }

    pub fn set_source_mode_selection(&mut self, selection: SourceSelection) {
        self.source_mode_selection = selection;
    }

    pub fn apply_source_value(
        &mut self,
        value: &str,
        selection_after: SourceSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, PatchError> {
        if value == self.source() {
            self.source_mode_selection = selection_after;
            return Ok(false);
        }
        let prefix = common_prefix(self.source(), value);
        let suffix = common_suffix(&self.source()[prefix..], &value[prefix..]);
        let source_end = self.source().len().saturating_sub(suffix);
        let value_end = value.len().saturating_sub(suffix);
        let revision = self.revision();
        let range = prefix..source_end;
        self.history.apply(&SourceTransaction {
            edits: vec![SourceEdit::new(
                range.clone(),
                value[prefix..value_end].to_owned(),
                revision,
            )],
            origin: SourceEditOrigin::SourceEditor,
            allowed_ranges: vec![range],
            selection_before: self.source_mode_selection,
            selection_after,
        })?;
        self.source_mode_selection = selection_after;
        self.dirty = true;
        self.sync_projection(selection_after.head, window, cx);
        self.emit_changed(cx);
        Ok(true)
    }

    pub fn replace_source(
        &mut self,
        source: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), MarkdownEditorError> {
        let document = SourceMarkdownDocument::parse(source.into())?;
        window.blur();
        self.history = SourceHistory::new(document);
        self.reset_block_renders();
        self.active_block = None;
        self.active_table_cell = None;
        self.active_surface = None;
        self.empty_surface_range = 0..self.source().len();
        self.pending_newline = None;
        self.dirty = false;
        self.reconcile_surfaces(window, cx);
        let _ = self.set_active_surface(MarkdownSurfaceKey::Empty);
        self.collapse_surface_projection(
            MarkdownSurfaceKey::Empty,
            SourceSelection::default(),
            window,
            cx,
        );
        self.sync_image_property_inputs(window, cx);
        Ok(())
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn set_theme(&mut self, theme: MarkdownEditorTheme, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            self.reset_block_renders();
            self.refresh_projection_highlights(cx);
            cx.notify();
        }
    }

    fn apply_projection_edit(
        &mut self,
        edit: crate::ProjectionEdit,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
    ) -> Result<(), PatchError> {
        let revision = self.history.document().revision;
        let allowed = edit.source_range.clone();
        self.history.apply(&SourceTransaction {
            edits: vec![SourceEdit::new(
                edit.source_range,
                edit.replacement,
                revision,
            )],
            origin: SourceEditOrigin::RichTextTyping,
            allowed_ranges: vec![allowed],
            selection_before,
            selection_after,
        })
    }

    fn reset_block_renders(&mut self) {
        self.block_render_generation = self.block_render_generation.wrapping_add(1);
        self.block_render_artifacts.clear();
        self.block_render_sources.clear();
        self.pending_block_renders.clear();
        self.block_render_errors.clear();
        self.block_render_cache.clear();
        self.pending_shared_renders.clear();
        self.inline_math_artifacts.clear();
        self.pending_inline_math_renders.clear();
        self.failed_inline_math_renders.clear();
        self.measured_block_heights.clear();
    }
}

impl EventEmitter<MarkdownEditorEvent> for MarkdownEditor {}

impl Focusable for MarkdownEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}
