use crate::{MarkdownEditorTheme, MarkdownProjection};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle, Subscription, Window,
};
use gpui_component::{VirtualListScrollHandle, input::InputState};
use markdown_source::{
    PatchError, SourceEdit, SourceEditOrigin, SourceHistory, SourceMarkdownDocument,
    SourceSelection, SourceTransaction,
};

mod media_operations;
mod operations;
mod render;
mod setup;
mod sync;
mod text_diff;
mod types;
use setup::{apply_projection_styles, create_input, create_property_input, subscribe_to_input};
use sync::projection_highlights;
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
    theme: MarkdownEditorTheme,
    dirty: bool,
    syncing_input: bool,
    source_mode_selection: SourceSelection,
    pending_newline: Option<usize>,
    block_scroll: VirtualListScrollHandle,
    document_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl MarkdownEditor {
    pub fn new(
        source: impl Into<String>,
        theme: MarkdownEditorTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Self, MarkdownEditorError> {
        let document = SourceMarkdownDocument::parse(source.into())?;
        let projection = MarkdownProjection::build(&document, None);
        let input = create_input(&projection.text, window, cx);
        let image_alt_input = create_property_input(window, cx);
        let image_destination_input = create_property_input(window, cx);
        apply_projection_styles(&input, &projection, &theme, cx);
        let subscriptions = subscribe_to_input(&input, window, cx);
        Ok(Self {
            input,
            image_alt_input,
            image_destination_input,
            history: SourceHistory::new(document),
            projection,
            active_block: None,
            active_table_cell: None,
            theme,
            dirty: false,
            syncing_input: false,
            source_mode_selection: SourceSelection::default(),
            pending_newline: None,
            block_scroll: VirtualListScrollHandle::new(),
            document_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        })
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

    pub fn projected_text(&self) -> &str {
        &self.projection.text
    }

    pub fn edit_projected_value(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, PatchError> {
        let Some(edit) = self.projection.edit_for_value(value) else {
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
        self.history = SourceHistory::new(document);
        self.active_block = None;
        self.active_table_cell = None;
        self.dirty = false;
        self.sync_projection(0, window, cx);
        Ok(())
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn set_theme(&mut self, theme: MarkdownEditorTheme, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            apply_projection_styles(&self.input, &self.projection, &self.theme, cx);
            cx.notify();
        }
    }

    pub fn undo(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, PatchError> {
        self.apply_history_change(true, window, cx)
    }

    pub fn redo(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, PatchError> {
        self.apply_history_change(false, window, cx)
    }

    pub fn undo_source_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Option<SourceSelection>, PatchError> {
        self.apply_source_history_change(true, window, cx)
    }

    pub fn redo_source_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Option<SourceSelection>, PatchError> {
        self.apply_source_history_change(false, window, cx)
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

    fn apply_history_change(
        &mut self,
        undo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, PatchError> {
        let selection = if undo {
            self.history.undo()?
        } else {
            self.history.redo()?
        };
        if let Some(selection) = selection {
            self.dirty = true;
            self.sync_selection(selection, window, cx);
            self.emit_changed(cx);
            return Ok(true);
        }
        Ok(false)
    }

    fn apply_source_history_change(
        &mut self,
        undo: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Option<SourceSelection>, PatchError> {
        let selection = if undo {
            self.history.undo()?
        } else {
            self.history.redo()?
        };
        if let Some(selection) = selection {
            self.source_mode_selection = selection;
            self.dirty = true;
            self.sync_projection(selection.head, window, cx);
            self.emit_changed(cx);
            return Ok(Some(selection));
        }
        Ok(None)
    }
}

impl EventEmitter<MarkdownEditorEvent> for MarkdownEditor {}

impl Focusable for MarkdownEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}
