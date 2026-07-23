use super::MarkdownEditor;
use gpui::{Context, Window};
use markdown_source::{PatchError, SourceSelection};

impl MarkdownEditor {
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
        let Some(selection) = selection else {
            return Ok(false);
        };
        self.dirty = true;
        self.sync_selection(selection, window, cx);
        self.emit_changed(cx);
        Ok(true)
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
        let Some(selection) = selection else {
            return Ok(None);
        };
        self.source_mode_selection = selection;
        self.dirty = true;
        self.sync_projection(selection.head, window, cx);
        self.emit_changed(cx);
        Ok(Some(selection))
    }
}
