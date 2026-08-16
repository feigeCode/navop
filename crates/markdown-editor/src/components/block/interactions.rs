//! Action handlers dispatched by GPUI's action system when bound keys are
//! pressed on a focused block.  Each handler maps to a named action declared
//! in [`crate::components::actions`] and delegates structural changes to the
//! parent editor via `BlockEvent` emissions.

use gpui::*;

use super::CollapsedCaretAffinity;
use super::{
    Block, BlockEvent, BlockKind, InlineFormat, InlineTextTree, PastedImageSource, UndoCaptureKind,
};
use crate::components::markdown::paste::should_split_plain_multiline_paste;
use crate::components::{
    BlockDown, BlockUp, BoldSelection, CodeSelection, Copy, Cut, Delete, DeleteBack, DeleteBlock,
    DuplicateBlock, End, ExitCodeBlock, FocusNext, FocusPrev, Home, IndentBlock, ItalicSelection,
    MoveBlockDown, MoveBlockUp, MoveLeft, MoveRight, Newline, OutdentBlock, Paste, SelectAll,
    SelectEnd, SelectHome, SelectLeft, SelectRight, SetHeading1, SetHeading2, SetHeading3,
    SetHeading4, SetHeading5, SetHeading6, SetParagraph, StrikethroughSelection, ToggleBulletList,
    ToggleCodeBlock, ToggleOrderedList, ToggleQuote, ToggleTaskList, UnderlineSelection,
    WordDeleteBack, WordDeleteForward, WordMoveLeft, WordMoveRight, WordSelectLeft,
    WordSelectRight,
};

impl Block {
    fn pasted_image_source_from_clipboard(item: &ClipboardItem) -> Option<PastedImageSource> {
        item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(PastedImageSource::ClipboardImage(image.clone())),
            ClipboardEntry::String(_) | ClipboardEntry::ExternalPaths(_) => None,
        })
    }

    fn pasted_image_source_from_text(text: &str) -> Option<PastedImageSource> {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.contains('\n') || trimmed.contains('\r') {
            return None;
        }

        Self::pasted_image_path_from_text_item(trimmed).map(PastedImageSource::LocalPath)
    }

    /// Parses a single clipboard text item as a local image path.
    ///
    /// Windows file-copy paste reaches GPUI as a plain drive-letter path; that
    /// must be tested as a path before URL parsing, because `url::Url` treats
    /// the drive letter as a URL scheme.
    fn pasted_image_path_from_text_item(text: &str) -> Option<std::path::PathBuf> {
        let unquoted = text
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or(text);
        let direct_path = std::path::PathBuf::from(unquoted);
        let path = if Self::is_supported_local_image_path(&direct_path) {
            direct_path
        } else if let Ok(url) = url::Url::parse(unquoted) {
            if url.scheme() == "file" {
                url.to_file_path().ok()?
            } else {
                return None;
            }
        } else {
            return None;
        };
        if !Self::is_supported_local_image_path(&path) {
            return None;
        }
        Some(path)
    }

    fn is_supported_local_image_path(path: &std::path::Path) -> bool {
        if !path.is_file() {
            return false;
        }
        let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
            return false;
        };
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff"
        )
    }

    fn paste_image_split(&self) -> (InlineTextTree, InlineTextTree) {
        let clean_selected = self.selection_clean_range();
        let (leading, tail) = self.record.title.split_at(clean_selected.start);
        let (_, trailing) = tail.split_at(clean_selected.end.saturating_sub(clean_selected.start));
        (leading, trailing)
    }

    fn is_leaf_quote(&self) -> bool {
        self.kind() == BlockKind::Quote
            && self.children.is_empty()
            && !self.display_text().contains('\n')
    }

    fn is_leaf_callout(&self) -> bool {
        matches!(self.kind(), BlockKind::Callout(_)) && self.children.is_empty()
    }

    fn is_empty_leaf_quote(&self) -> bool {
        self.is_leaf_quote() && self.selected_range.is_empty() && self.is_empty()
    }

    fn downgrade_leaf_callout_to_quote_at_start(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.is_leaf_callout() || !self.selected_range.is_empty() || self.cursor_offset() != 0 {
            return false;
        }

        let BlockKind::Callout(variant) = self.kind() else {
            return false;
        };
        let header_markdown = variant.header_markdown(&self.record.title.serialize_markdown());
        self.record.kind = BlockKind::Quote;
        self.record
            .set_title(InlineTextTree::from_markdown(&header_markdown));
        self.sync_edit_mode_from_kind();
        self.sync_render_cache();
        self.assign_collapsed_selection_offset(0, CollapsedCaretAffinity::Default, None);
        self.marked_range = None;
        self.cursor_blink_epoch = std::time::Instant::now();
        cx.emit(BlockEvent::Changed);
        cx.notify();
        true
    }

    fn downgrade_empty_leaf_quote_to_paragraph(&mut self, cx: &mut Context<Self>) -> bool {
        if self.is_empty_leaf_quote() {
            self.convert_to_paragraph(cx);
            return true;
        }
        false
    }

    /// If the code block's last line is a bare fence (three or more backticks
    /// or tildes, no info string), returns the byte offset to cut from so the
    /// whole line is removed; otherwise `None`.
    fn trailing_code_fence_line_start(&self) -> Option<usize> {
        let text = self.display_text();
        let line_start = text.rfind('\n').map(|idx| idx + 1).unwrap_or(0);
        let is_bare_fence = BlockKind::parse_code_fence_opening(&text[line_start..])
            .is_some_and(|fence| fence.language.is_none());
        // Cut from the preceding newline too, unless the fence is the only line.
        is_bare_fence.then(|| line_start.saturating_sub(1))
    }

    pub(crate) fn on_newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        // The ATX prefix is a one-shot typing affordance. Once Enter is used
        // to leave or split the heading, hide it permanently before focus
        // transfer and the corresponding blur notification complete.
        self.clear_heading_shortcut_marker(cx);

        // Enter is ordered from special editors to rich-text splitting:
        // table/source/code/quote-like blocks keep local newline semantics,
        // while normal rendered blocks emit an editor-level split request.
        if self.is_table_cell() {
            cx.emit(BlockEvent::RequestTableCellMoveVertical { delta: 1 });
            cx.stop_propagation();
            return;
        }

        if self.editor_selection_range.is_some() {
            cx.emit(BlockEvent::RequestReplaceCrossBlockSelection {
                text: "\n".to_string(),
                selected_range_relative: None,
                mark_inserted_text: false,
                undo_kind: UndoCaptureKind::NonCoalescible,
            });
            return;
        }

        if self.is_source_raw_mode() {
            if !self.selected_range.is_empty() {
                self.replace_text_in_range(None, "", window, cx);
            }
            self.replace_text_in_range(None, "\n", window, cx);
            return;
        }

        if self.kind() == BlockKind::Paragraph
            && self.selected_range.is_empty()
            && self.cursor_offset() == self.visible_len()
            && BlockKind::parse_separator_line(self.display_text())
            // A dash run is also a setext underline; defer it to the editor so a
            // preceding paragraph can become a heading (the editor falls back to
            // a separator when there is no heading target).
            && BlockKind::parse_setext_underline(self.display_text()).is_none()
        {
            self.convert_to_separator(cx);
            cx.emit(BlockEvent::RequestNewline {
                trailing: InlineTextTree::plain(String::new()),
                source_already_mutated: true,
            });
            return;
        }

        // `$$` then Enter opens a display-math block. Keying off the caret sitting
        // right after a leading `$$` (rather than the line being exactly `$$`)
        // means it also fires after pressing Home on an existing line and typing
        // the fence in front of a formula: the rest of the line becomes the math
        // body instead of being split off into a new paragraph.
        if self.kind() == BlockKind::Paragraph
            && self.selected_range.is_empty()
            && self.cursor_offset() == "$$".len()
            && self.display_text().starts_with("$$")
        {
            let body = self.display_text()["$$".len()..].to_string();
            self.enter_math_block(&body, cx);
            return;
        }

        if self.kind() == BlockKind::Paragraph
            && self.selected_range.is_empty()
            && self.cursor_offset() == self.visible_len()
            && let Some(fence) = BlockKind::parse_code_fence_opening(self.display_text())
        {
            self.enter_code_block(fence.language, cx);
            return;
        }

        if self.kind().is_separator() {
            cx.emit(BlockEvent::RequestNewline {
                trailing: InlineTextTree::plain(String::new()),
                source_already_mutated: false,
            });
            return;
        }

        if self.kind().is_list_item() && self.selected_range.is_empty() && self.is_empty() {
            cx.emit(BlockEvent::RequestOutdent);
            return;
        }

        if self.kind() == BlockKind::Quote {
            if !self.selected_range.is_empty() {
                self.replace_text_in_range(None, "", window, cx);
            }
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            self.replace_text_in_range(None, "\n", window, cx);
            return;
        }

        if matches!(self.kind(), BlockKind::Callout(_)) {
            cx.emit(BlockEvent::RequestEnterCalloutBody);
            return;
        }

        // In a code block, Enter inserts a newline into the block content
        // rather than splitting the block.  Pressing Enter on an empty
        // code block exits back to a paragraph.
        if self.kind().is_code_block() {
            if self.selected_range.is_empty() && self.is_empty() {
                self.convert_to_paragraph(cx);
                return;
            }
            // Typing a bare closing fence on the last line and pressing Enter
            // leaves the block, matching source mode.
            if self.selected_range.is_empty()
                && self.cursor_offset() == self.visible_len()
                && let Some(fence_start) = self.trailing_code_fence_line_start()
            {
                let fence_end = self.visible_len();
                self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
                self.replace_text_in_visible_range(fence_start..fence_end, "", None, false, cx);
                cx.emit(BlockEvent::RequestNewline {
                    trailing: InlineTextTree::plain(String::new()),
                    source_already_mutated: true,
                });
                return;
            }
            // A blank line at the end is the code-block exit affordance:
            // the first Enter creates that line, while the second Enter
            // transfers editing to the real paragraph after the block.
            // Do not append a second newline here, otherwise the editor-level
            // RequestNewline handler never gets a chance to focus/create the
            // following document node.
            if self.selected_range.is_empty()
                && self.cursor_offset() == self.visible_len()
                && self.display_text().ends_with('\n')
            {
                cx.emit(BlockEvent::RequestNewline {
                    trailing: InlineTextTree::plain(String::new()),
                    source_already_mutated: false,
                });
                return;
            }
            if !self.selected_range.is_empty() {
                self.replace_text_in_range(None, "", window, cx);
            }
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            self.replace_text_in_range(None, "\n", window, cx);
            return;
        }

        if self.collapsed_caret_inherits_inline_code_style() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            self.replace_text_in_range(None, "\n", window, cx);
            return;
        }

        if !self.selected_range.is_empty() {
            self.replace_text_in_range(None, "", window, cx);
        }

        let cursor = self.cursor_offset();
        let (leading, trailing) = self.split_title(cursor);
        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        self.record.set_title(leading);
        self.mark_changed(cx);
        let cursor = self.visible_len();
        self.assign_collapsed_selection_offset(cursor, CollapsedCaretAffinity::Default, None);
        self.marked_range = None;
        cx.emit(BlockEvent::RequestNewline {
            trailing,
            source_already_mutated: true,
        });
    }

    pub(crate) fn on_delete_back(
        &mut self,
        _: &DeleteBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_table_cell() {
            if self.selected_range.is_empty() {
                let previous = self.previous_boundary(self.cursor_offset());
                if previous == self.cursor_offset() {
                    return;
                }
                self.select_to(previous, cx);
            }
            self.replace_text_in_range(None, "", window, cx);
            return;
        }

        if self.is_source_raw_mode() {
            if self.selected_range.is_empty() {
                self.select_to(self.previous_boundary(self.cursor_offset()), cx);
            }
            self.replace_text_in_range(None, "", window, cx);
            return;
        }

        if self.selected_range.is_empty() && self.cursor_offset() == 0 {
            if self.kind() == BlockKind::Paragraph && self.is_direct_list_child() && self.is_empty()
            {
                cx.emit(BlockEvent::RequestOutdent);
                return;
            }
            if self.is_nested_list_item() {
                cx.emit(BlockEvent::RequestDowngradeNestedListItemToChildParagraph);
                return;
            }
            match self.kind() {
                BlockKind::BulletedListItem
                | BlockKind::TaskListItem { .. }
                | BlockKind::NumberedListItem => {
                    cx.emit(BlockEvent::RequestOutdent);
                    return;
                }
                BlockKind::Heading { .. } => {
                    self.convert_to_paragraph(cx);
                    return;
                }
                BlockKind::Quote => {
                    if self.is_leaf_quote() {
                        self.convert_to_paragraph(cx);
                    }
                    return;
                }
                BlockKind::Callout(_) => {
                    if self.downgrade_leaf_callout_to_quote_at_start(cx) {
                        return;
                    }
                    return;
                }
                BlockKind::Separator => {
                    self.convert_to_paragraph(cx);
                    return;
                }
                BlockKind::CodeBlock { .. } => {
                    self.convert_to_paragraph(cx);
                    return;
                }
                _ => {}
            }
        }

        if self.downgrade_leaf_callout_to_quote_at_start(cx)
            || self.downgrade_empty_leaf_quote_to_paragraph(cx)
        {
            return;
        }

        if self.selected_range.is_empty() && self.display_text().is_empty() {
            cx.emit(BlockEvent::RequestDelete);
            return;
        }

        if self.selected_range.is_empty() && self.cursor_offset() == 0 {
            cx.emit(BlockEvent::RequestMergeIntoPrev {
                content: self.record.title.clone(),
            });
            return;
        }

        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn on_delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_table_cell() {
            if self.selected_range.is_empty() {
                let next = self.next_boundary(self.cursor_offset());
                if next == self.cursor_offset() {
                    return;
                }
                self.select_to(next, cx);
            }
            self.replace_text_in_range(None, "", window, cx);
            return;
        }

        if self.is_source_raw_mode() {
            if self.selected_range.is_empty() {
                self.select_to(self.next_boundary(self.cursor_offset()), cx);
            }
            self.replace_text_in_range(None, "", window, cx);
            return;
        }

        if self.downgrade_leaf_callout_to_quote_at_start(cx)
            || self.downgrade_empty_leaf_quote_to_paragraph(cx)
        {
            return;
        }

        if self.kind().is_separator() {
            self.convert_to_paragraph(cx);
            return;
        }

        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn on_word_delete_back(
        &mut self,
        _: &WordDeleteBack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            if self.cursor_offset() == 0 {
                // Nothing to the left in this block; defer to grapheme
                // backspace, which handles block merge and downgrades.
                self.on_delete_back(&DeleteBack, window, cx);
                return;
            }
            self.select_to(self.previous_word_start(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn on_word_delete_forward(
        &mut self,
        _: &WordDeleteForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            if self.cursor_offset() == self.visible_len() {
                // Nothing to the right in this block; defer to grapheme
                // delete, which handles block merge and separator removal.
                self.on_delete(&Delete, window, cx);
                return;
            }
            self.select_to(self.next_word_start(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    pub(crate) fn on_indent_block(
        &mut self,
        _: &IndentBlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_table_cell() {
            cx.emit(BlockEvent::RequestTableCellMoveHorizontal { delta: 1 });
            return;
        }
        if self.can_adjust_list_nesting() {
            cx.emit(BlockEvent::RequestIndent);
            return;
        }
        if self.kind() == BlockKind::Paragraph || self.kind().is_code_block() {
            self.replace_text_in_range(None, "    ", window, cx);
        }
    }

    pub(crate) fn on_outdent_block(
        &mut self,
        _: &OutdentBlock,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_table_cell() {
            cx.emit(BlockEvent::RequestTableCellMoveHorizontal { delta: -1 });
            return;
        }
        if self.can_outdent_list_nesting() {
            cx.emit(BlockEvent::RequestOutdent);
        }
    }

    pub(crate) fn on_focus_prev(
        &mut self,
        _: &FocusPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preferred_x = self.vertical_anchor_x();
        if !self.move_cursor_vertically(-1, preferred_x, cx) {
            if self.is_table_cell() {
                cx.emit(BlockEvent::RequestTableCellMoveVertical { delta: -1 });
                return;
            }
            cx.emit(BlockEvent::RequestFocusPrev {
                preferred_x: Some(f32::from(preferred_x)),
            });
        }
    }

    pub(crate) fn on_focus_next(
        &mut self,
        _: &FocusNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preferred_x = self.vertical_anchor_x();
        if !self.move_cursor_vertically(1, preferred_x, cx) {
            if self.is_table_cell() {
                cx.emit(BlockEvent::RequestTableCellMoveVertical { delta: 1 });
                return;
            }
            cx.emit(BlockEvent::RequestFocusNext {
                preferred_x: Some(f32::from(preferred_x)),
            });
        }
    }

    pub(crate) fn on_move_left(
        &mut self,
        _: &MoveLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            if let Some((target, affinity)) = self.projected_move_left_target(self.cursor_offset())
            {
                self.assign_collapsed_selection_offset(target, affinity, None);
                self.cursor_blink_epoch = std::time::Instant::now();
                cx.notify();
            } else {
                let previous = self.previous_boundary(self.cursor_offset());
                // At the start of a table cell, step into the previous cell
                // rather than stalling at the edge (same path as Shift+Tab).
                if previous == self.cursor_offset() && self.is_table_cell() {
                    cx.emit(BlockEvent::RequestTableCellMoveHorizontal { delta: -1 });
                    return;
                }
                self.move_to(previous, cx);
            }
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    pub(crate) fn on_move_right(
        &mut self,
        _: &MoveRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            if let Some((target, affinity)) =
                self.projected_move_right_target(self.selected_range.end)
            {
                self.assign_collapsed_selection_offset(target, affinity, None);
                self.cursor_blink_epoch = std::time::Instant::now();
                cx.notify();
            } else {
                let next = self.next_boundary(self.selected_range.end);
                // At the end of a table cell, step into the next cell rather
                // than stalling at the edge (same path as Tab).
                if next == self.selected_range.end && self.is_table_cell() {
                    cx.emit(BlockEvent::RequestTableCellMoveHorizontal { delta: 1 });
                    return;
                }
                self.move_to(next, cx);
            }
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    pub(crate) fn on_home(&mut self, _: &Home, _window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    pub(crate) fn on_end(&mut self, _: &End, _window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.visible_len(), cx);
    }

    pub(crate) fn on_select_left(
        &mut self,
        _: &SelectLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((target, _)) = self.projected_move_left_target(self.cursor_offset()) {
            self.select_to(target, cx);
        } else {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
    }

    pub(crate) fn on_select_right(
        &mut self,
        _: &SelectRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((target, _)) = self.projected_move_right_target(self.cursor_offset()) {
            self.select_to(target, cx);
        } else {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
    }

    pub(crate) fn on_word_move_left(
        &mut self,
        _: &WordMoveLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_to(self.previous_word_start(self.cursor_offset()), cx);
    }

    pub(crate) fn on_word_move_right(
        &mut self,
        _: &WordMoveRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_to(self.next_word_start(self.cursor_offset()), cx);
    }

    pub(crate) fn on_word_select_left(
        &mut self,
        _: &WordSelectLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.previous_word_start(self.cursor_offset()), cx);
    }

    pub(crate) fn on_word_select_right(
        &mut self,
        _: &WordSelectRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.next_word_start(self.cursor_offset()), cx);
    }

    pub(crate) fn on_block_up(
        &mut self,
        _: &BlockUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(BlockEvent::RequestBlockUp);
    }

    pub(crate) fn on_block_down(
        &mut self,
        _: &BlockDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(BlockEvent::RequestBlockDown);
    }

    fn select_all_text(&mut self, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.visible_len(), cx);
    }

    pub(crate) fn on_select_all(
        &mut self,
        _: &SelectAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.show_source_line_numbers() {
            self.select_all_text(cx);
        } else {
            cx.emit(BlockEvent::RequestRenderedSelectAll);
        }
    }

    pub(crate) fn on_select_home(
        &mut self,
        _: &SelectHome,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(0, cx);
    }

    pub(crate) fn on_select_end(
        &mut self,
        _: &SelectEnd,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.visible_len(), cx);
    }

    pub(crate) fn on_copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.display_text()[self.selected_range.clone()].to_string(),
            ));
        }
    }

    pub(crate) fn on_cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.display_text()[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx);
        }
    }

    pub(crate) fn on_paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.kind().is_separator() && !self.uses_raw_text_editing() {
            return;
        }

        if let Some(item) = cx.read_from_clipboard() {
            if let Some(source) = Self::pasted_image_source_from_clipboard(&item) {
                let (leading, trailing) = self.paste_image_split();
                cx.emit(BlockEvent::RequestPasteImage {
                    leading,
                    source,
                    trailing,
                });
                return;
            }

            let Some(text) = item.text() else {
                return;
            };
            if let Some(source) = Self::pasted_image_source_from_text(&text) {
                let (leading, trailing) = self.paste_image_split();
                cx.emit(BlockEvent::RequestPasteImage {
                    leading,
                    source,
                    trailing,
                });
                return;
            }

            // Only rendered rich-text blocks apply paste correction. Raw/code
            // contexts preserve bytes, and table cells flatten newlines so the
            // surrounding table structure is not accidentally split.
            if self.editor_selection_range.is_some() {
                cx.emit(BlockEvent::RequestReplaceCrossBlockSelection {
                    text,
                    selected_range_relative: None,
                    mark_inserted_text: false,
                    undo_kind: UndoCaptureKind::NonCoalescible,
                });
                return;
            }

            if self.is_table_cell() {
                let flattened = text.replace("\r\n", " ").replace(['\r', '\n'], " ");
                self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
                self.replace_text_in_range(None, &flattened, window, cx);
                return;
            }

            if self.uses_raw_text_editing() {
                self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
                self.replace_text_in_range(None, &text, window, cx);
                return;
            }

            if text.contains('\n') || text.contains('\r') {
                let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                if self.quote_depth > 0 {
                    self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
                    self.replace_text_in_range(None, &normalized, window, cx);
                    return;
                }
                let clean_selected = self.selection_clean_range();
                let (leading, tail) = self.record.title.split_at(clean_selected.start);
                let (_, trailing) =
                    tail.split_at(clean_selected.end.saturating_sub(clean_selected.start));
                let lines = normalized
                    .split('\n')
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();
                let split_physical_lines = should_split_plain_multiline_paste(&lines);
                cx.emit(BlockEvent::RequestPasteMultiline {
                    leading,
                    lines,
                    trailing,
                    split_physical_lines,
                });
                return;
            }

            self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    pub(crate) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.showing_rendered_image() {
            self.is_selecting = false;
            self.request_image_edit_expansion();
            if self.focus_handle.is_focused(window) {
                if self.sync_image_focus_state(true) {
                    cx.notify();
                }
            } else {
                cx.emit(BlockEvent::RequestFocus);
            }
            cx.stop_propagation();
            return;
        }

        let offset = self.index_for_mouse_position(event.position);
        let was_focused = self.focus_handle.is_focused(window);

        // Cmd/Ctrl+click follows a rendered link instead of editing it, so the
        // block is neither focused nor selected; the link opens on mouse-up.
        if event.modifiers.secondary() && self.pointer_link_hit(event.position).is_some() {
            self.is_selecting = false;
            cx.stop_propagation();
            return;
        }

        if was_focused {
            self.is_selecting = true;
            if event.modifiers.shift {
                self.select_to(offset, cx);
            } else {
                self.move_to(offset, cx);
            }
        } else {
            self.is_selecting = false;
            self.move_to(offset, cx);
            cx.emit(BlockEvent::RequestFocus);
        }
    }

    /// Resolve the inline link under a pointer position against the most recent
    /// rendered text layout, if any. Returns `None` while the block shows raw
    /// source or when the pointer is not over a link.
    pub(crate) fn pointer_link_hit(&self, position: Point<Pixels>) -> Option<super::InlineLinkHit> {
        self.last_layout
            .as_ref()
            .zip(self.last_bounds)
            .and_then(|(lines, bounds)| {
                super::element::link_at_position(
                    self,
                    lines,
                    bounds,
                    self.last_line_height,
                    position,
                )
            })
            .cloned()
    }

    /// Handle mouse-down on a rendered inline link (in a mixed inline-visual
    /// block). A Cmd/Ctrl+click is claimed here so it follows the link instead
    /// of focusing the block; the destination opens on the matching mouse-up.
    pub(crate) fn on_rendered_link_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Only Cmd/Ctrl+click follows the link; a plain click falls through so
        // the block focuses for editing like any other inline text.
        if event.modifiers.secondary() {
            cx.stop_propagation();
        }
    }

    /// Open a rendered inline link's destination through the editor prompt.
    pub(crate) fn open_rendered_link(
        &mut self,
        link: &super::InlineLinkHit,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        cx.emit(BlockEvent::RequestOpenLink {
            prompt_target: link.prompt_target.clone(),
            open_target: link.open_target.clone(),
        });
    }

    pub(crate) fn on_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.is_selecting = false;

        // Cmd/Ctrl+click follows a rendered link, using the same open-link
        // prompt as the double-click gesture below.
        if event.modifiers.secondary()
            && let Some(link) = self.pointer_link_hit(event.position)
        {
            self.open_rendered_link(&link, cx);
            return;
        }

        if event.click_count >= 2 {
            let footnote = self
                .last_layout
                .as_ref()
                .zip(self.last_bounds)
                .and_then(|(lines, bounds)| {
                    super::element::footnote_at_position(
                        self,
                        lines,
                        bounds,
                        self.last_line_height,
                        event.position,
                    )
                })
                .cloned();
            if let Some(footnote) = footnote {
                cx.stop_propagation();
                cx.emit(BlockEvent::RequestJumpToFootnoteDefinition { id: footnote.id });
                return;
            }

            if let Some(link) = self.pointer_link_hit(event.position) {
                self.open_rendered_link(&link, cx);
            }
        }
    }

    pub(crate) fn on_footnote_backref_mouse_down(
        &mut self,
        _: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        if !self.focus_handle.is_focused(window) {
            cx.emit(BlockEvent::RequestFocus);
        }
    }

    pub(crate) fn on_footnote_backref_mouse_up(
        &mut self,
        _: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.footnote_definition_id() else {
            return;
        };
        cx.stop_propagation();
        cx.emit(BlockEvent::RequestJumpToFootnoteBackref { id });
    }

    pub(crate) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_selecting {
            // A stale selecting flag can survive a missed mouse-up. Only extend
            // the selection while the platform still reports an active drag.
            if !event.dragging() {
                self.is_selecting = false;
                cx.notify();
                return;
            }
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    pub(crate) fn on_task_checkbox_mouse_down(
        &mut self,
        _: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        if !self.focus_handle.is_focused(window) {
            cx.emit(BlockEvent::RequestFocus);
        }
    }

    pub(crate) fn on_task_checkbox_mouse_up(
        &mut self,
        _: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.kind().is_task_list_item() || self.is_source_raw_mode() {
            return;
        }

        cx.stop_propagation();
        cx.emit(BlockEvent::ToggleTaskChecked);
    }

    pub(crate) fn on_table_cell_context_menu_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_table_cell() {
            cx.stop_propagation();
            cx.emit(BlockEvent::RequestOpenTableContextMenu {
                position: event.position,
            });
        }
    }

    pub(crate) fn on_bold_selection(
        &mut self,
        _: &BoldSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_inline_format(InlineFormat::Bold, cx);
    }

    pub(crate) fn on_italic_selection(
        &mut self,
        _: &ItalicSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_inline_format(InlineFormat::Italic, cx);
    }

    pub(crate) fn on_underline_selection(
        &mut self,
        _: &UnderlineSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_inline_format(InlineFormat::Underline, cx);
    }

    pub(crate) fn on_strikethrough_selection(
        &mut self,
        _: &StrikethroughSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_inline_format(InlineFormat::Strikethrough, cx);
    }

    pub(crate) fn on_code_selection(
        &mut self,
        _: &CodeSelection,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_inline_format(InlineFormat::Code, cx);
    }

    fn request_block_kind(&mut self, kind: BlockKind, cx: &mut Context<Self>) {
        if !self.is_table_cell() && !self.is_source_raw_mode() {
            cx.emit(BlockEvent::RequestSetBlockKind { kind });
        }
    }

    fn request_toggle_block_kind(
        &mut self,
        target: BlockKind,
        is_active: impl FnOnce(&BlockKind) -> bool,
        cx: &mut Context<Self>,
    ) {
        let current = self.kind();
        self.request_block_kind(
            if is_active(&current) {
                BlockKind::Paragraph
            } else {
                target
            },
            cx,
        );
    }

    pub(crate) fn on_set_paragraph(
        &mut self,
        _: &SetParagraph,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_block_kind(BlockKind::Paragraph, cx);
    }

    pub(crate) fn on_set_heading_1(
        &mut self,
        _: &SetHeading1,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_block_kind(BlockKind::Heading { level: 1 }, cx);
    }

    pub(crate) fn on_set_heading_2(
        &mut self,
        _: &SetHeading2,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_block_kind(BlockKind::Heading { level: 2 }, cx);
    }

    pub(crate) fn on_set_heading_3(
        &mut self,
        _: &SetHeading3,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_block_kind(BlockKind::Heading { level: 3 }, cx);
    }

    pub(crate) fn on_set_heading_4(
        &mut self,
        _: &SetHeading4,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_block_kind(BlockKind::Heading { level: 4 }, cx);
    }

    pub(crate) fn on_set_heading_5(
        &mut self,
        _: &SetHeading5,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_block_kind(BlockKind::Heading { level: 5 }, cx);
    }

    pub(crate) fn on_set_heading_6(
        &mut self,
        _: &SetHeading6,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_block_kind(BlockKind::Heading { level: 6 }, cx);
    }

    pub(crate) fn on_toggle_bullet_list(
        &mut self,
        _: &ToggleBulletList,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_toggle_block_kind(
            BlockKind::BulletedListItem,
            |kind| *kind == BlockKind::BulletedListItem,
            cx,
        );
    }

    pub(crate) fn on_toggle_ordered_list(
        &mut self,
        _: &ToggleOrderedList,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_toggle_block_kind(
            BlockKind::NumberedListItem,
            |kind| *kind == BlockKind::NumberedListItem,
            cx,
        );
    }

    pub(crate) fn on_toggle_task_list(
        &mut self,
        _: &ToggleTaskList,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_toggle_block_kind(
            BlockKind::TaskListItem { checked: false },
            BlockKind::is_task_list_item,
            cx,
        );
    }

    pub(crate) fn on_toggle_quote(
        &mut self,
        _: &ToggleQuote,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_toggle_block_kind(BlockKind::Quote, |kind| *kind == BlockKind::Quote, cx);
    }

    pub(crate) fn on_toggle_code_block(
        &mut self,
        _: &ToggleCodeBlock,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_toggle_block_kind(
            BlockKind::CodeBlock { language: None },
            BlockKind::is_code_block,
            cx,
        );
    }

    pub(crate) fn on_move_block_up(
        &mut self,
        _: &MoveBlockUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_table_cell() && !self.is_source_raw_mode() {
            cx.emit(BlockEvent::RequestMoveBlockUp);
        }
    }

    pub(crate) fn on_move_block_down(
        &mut self,
        _: &MoveBlockDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_table_cell() && !self.is_source_raw_mode() {
            cx.emit(BlockEvent::RequestMoveBlockDown);
        }
    }

    pub(crate) fn on_duplicate_block(
        &mut self,
        _: &DuplicateBlock,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_table_cell() && !self.is_source_raw_mode() {
            cx.emit(BlockEvent::RequestDuplicateBlock);
        }
    }

    pub(crate) fn on_delete_block(
        &mut self,
        _: &DeleteBlock,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_table_cell() && !self.is_source_raw_mode() {
            cx.emit(BlockEvent::RequestDelete);
        }
    }

    pub(crate) fn on_exit_code_block(
        &mut self,
        _: &ExitCodeBlock,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let exits_multiline_block = self.is_table_cell() || self.kind().is_multiline_text_block();

        if exits_multiline_block {
            cx.emit(BlockEvent::RequestNewline {
                trailing: InlineTextTree::plain(String::new()),
                source_already_mutated: false,
            });
        } else if self.callout_depth > 0 {
            cx.emit(BlockEvent::RequestCalloutBreak);
        } else if self.quote_depth > 0 {
            cx.emit(BlockEvent::RequestQuoteBreak);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Block;
    use crate::components::{BlockKind, BlockRecord, InlineTextTree, PastedImageSource};
    use gpui::{AppContext, TestAppContext};
    use std::fs;

    fn temp_image_path(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "velotype-paste-image-path-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).expect("temp image dir should exist");
        let path = root.join(name);
        fs::write(
            &path,
            b"not a real image; extension is enough for paste routing",
        )
        .expect("temp image should be written");
        path
    }

    fn remove_temp_image(path: &std::path::Path) {
        let _ = path.parent().map(|parent| fs::remove_dir_all(parent));
    }

    #[test]
    fn paste_image_text_accepts_plain_local_image_path() {
        let path = temp_image_path("copied.png");
        let text = path.to_string_lossy().to_string();
        #[cfg(target_os = "windows")]
        assert!(
            text.contains(':'),
            "test should exercise Windows drive-letter paths"
        );

        let source = Block::pasted_image_source_from_text(&text);

        assert_eq!(source, Some(PastedImageSource::LocalPath(path.clone())));
        remove_temp_image(&path);
    }

    #[test]
    fn paste_image_text_accepts_quoted_local_image_path() {
        let path = temp_image_path("quoted image.png");
        let text = format!("\"{}\"", path.display());

        let source = Block::pasted_image_source_from_text(&text);

        assert_eq!(source, Some(PastedImageSource::LocalPath(path.clone())));
        remove_temp_image(&path);
    }

    #[test]
    fn paste_image_text_accepts_file_url() {
        let path = temp_image_path("url image.png");
        let url = url::Url::from_file_path(&path).expect("temp image path should form file URL");

        let source = Block::pasted_image_source_from_text(url.as_str());

        assert_eq!(source, Some(PastedImageSource::LocalPath(path.clone())));
        remove_temp_image(&path);
    }

    #[test]
    fn paste_image_text_rejects_non_image_path() {
        let path = temp_image_path("notes.txt");
        let text = path.to_string_lossy().to_string();

        let source = Block::pasted_image_source_from_text(&text);

        assert_eq!(source, None);
        remove_temp_image(&path);
    }

    #[gpui::test]
    async fn multiline_quote_is_not_treated_as_leaf(cx: &mut TestAppContext) {
        let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

        block.update(cx, |block, cx| {
            block.record.kind = BlockKind::Quote;
            block.record.set_title(InlineTextTree::plain("first\n"));
            block.sync_edit_mode_from_kind();
            block.sync_render_cache();
            cx.notify();

            assert!(!block.is_leaf_quote());
        });
    }
}
