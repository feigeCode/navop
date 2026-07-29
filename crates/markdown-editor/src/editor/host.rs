//! Host-facing API for embedding the Velotype editor in another GPUI view.

use super::*;

/// Document mutations emitted to the editor's host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorEvent {
    /// The canonical Markdown document changed.
    Changed { revision: u64 },
}

impl EventEmitter<EditorEvent> for Editor {}

impl Editor {
    fn replacement_roots(
        &self,
        normalized: &str,
        cx: &mut Context<Self>,
    ) -> Option<Vec<Entity<Block>>> {
        let current_source = self.current_document_source(cx);

        match self.view_mode {
            ViewMode::Rendered => {
                let roots = Self::build_rendered_roots(cx, normalized);
                let candidate_source = DocumentTree::new(roots.clone()).markdown_text(cx);
                (candidate_source != current_source).then_some(roots)
            }
            ViewMode::Source => {
                if normalized == current_source {
                    return None;
                }
                let block = Self::new_block(cx, BlockRecord::paragraph(normalized));
                block.update(cx, |block, _cx| block.set_source_document_mode());
                Some(vec![block])
            }
        }
    }

    fn install_replacement_roots(
        &mut self,
        replacement: Vec<Entity<Block>>,
        cx: &mut Context<Self>,
    ) {
        self.end_block_pointer_selection_sessions(cx);
        self.clear_cross_block_selection(cx);
        self.rendered_select_all_cycle = None;
        self.document.replace_roots(replacement, cx);
        if self.view_mode == ViewMode::Rendered {
            self.rebuild_table_runtimes(cx);
        } else {
            self.table_cells.clear();
        }
        self.rebuild_image_runtimes(cx);

        self.pending_focus = self.first_focusable_entity_id(cx);
        self.active_entity_id = self.pending_focus;
        self.pending_scroll_active_block_into_view = true;
        self.pending_scroll_recheck_after_layout = true;
        self.last_scroll_viewport_size = None;
        self.sync_table_axis_visuals(cx);
        self.dismiss_contextual_overlays(cx);
    }

    /// Returns the monotonically increasing document revision.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Replaces host-only rendering services without changing the document,
    /// selection, history, focus, or dirty state.
    pub fn set_host_services(&mut self, services: EditorHostServices, cx: &mut Context<Self>) {
        let services = Arc::new(services);
        self.effective_theme = Self::derive_effective_theme(&services, cx);
        self.host_services = services;
        self.host_services_revision = self.host_services_revision.saturating_add(1);
        self.sync_host_services_for_all_blocks(cx);
        cx.notify();
    }

    pub fn host_services_revision(&self) -> u64 {
        self.host_services_revision
    }

    pub fn has_block_render_provider(&self) -> bool {
        self.host_services.block_renderer().is_some()
    }

    pub fn has_code_highlight_provider(&self) -> bool {
        self.host_services.code_highlighter().is_some()
    }

    /// Replaces the current Markdown through Velotype's real document tree and
    /// records the replacement in the same history used by interactive edits.
    pub fn replace_markdown(&mut self, markdown: String, cx: &mut Context<Self>) -> bool {
        let normalized = Self::normalize_markdown(&markdown);
        let Some(replacement) = self.replacement_roots(&normalized, cx) else {
            return false;
        };

        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.install_replacement_roots(replacement, cx);
        self.mark_dirty(cx);
        self.finalize_pending_undo_capture(cx);
        cx.notify();
        true
    }

    /// Reloads Markdown as a new clean baseline, discarding the current
    /// undo/redo chain.  This is intended for hosts accepting an external
    /// file change; unlike [`Self::replace_markdown`], the reload itself is
    /// never undoable.
    pub fn reload_markdown(&mut self, markdown: String, cx: &mut Context<Self>) -> bool {
        let normalized = Self::normalize_markdown(&markdown);
        let replacement = self.replacement_roots(&normalized, cx);
        let changed = replacement.is_some();

        if let Some(replacement) = replacement {
            self.install_replacement_roots(replacement, cx);
        }

        self.reset_document_history(cx);
        self.document_dirty = false;
        if changed {
            self.record_document_changed(cx);
        }
        cx.notify();
        changed
    }

    /// Marks the host-managed document as saved without altering history.
    pub fn mark_saved(&mut self, cx: &mut Context<Self>) {
        if !self.document_dirty {
            return;
        }

        self.document_dirty = false;
        cx.notify();
    }

    /// Undoes one document mutation using Velotype's native history.
    pub fn undo(&mut self, cx: &mut Context<Self>) -> bool {
        self.undo_document(cx)
    }

    /// Redoes one document mutation using Velotype's native history.
    pub fn redo(&mut self, cx: &mut Context<Self>) -> bool {
        self.redo_document(cx)
    }

    /// Returns the active editor view.
    pub fn view_mode(&self) -> ViewMode {
        self.view_mode
    }

    /// Switches views without reporting a document mutation.
    pub fn set_view_mode(&mut self, view_mode: ViewMode, cx: &mut Context<Self>) -> bool {
        self.switch_to_view_mode(view_mode, cx)
    }

    /// Focuses a real editable block, including a table cell when appropriate.
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(entity_id) = self.current_edit_target_entity_id_from_state(cx) else {
            return false;
        };
        let Some(block) = self.focusable_entity_by_id(entity_id) else {
            return false;
        };

        self.pending_focus = None;
        self.active_entity_id = Some(entity_id);
        self.pending_scroll_active_block_into_view = true;
        block.read(cx).focus_handle.clone().focus(window, cx);
        true
    }

    /// Returns whether any real block editor owned by this editor has focus.
    pub fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focused_edit_target_entity_id(window, cx).is_some()
    }

    pub(super) fn record_document_changed(&mut self, cx: &mut Context<Self>) {
        self.revision = self.revision.saturating_add(1);
        cx.emit(EditorEvent::Changed {
            revision: self.revision,
        });
    }
}
