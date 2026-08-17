//! Editor context menus and native table insertion dialog.

use super::table_menu::{
    TableMenuAction, TableMenuEntry, TableMenuTarget, table_menu_entries, table_menu_move_delta,
};
use super::{Editor, ViewMode};
use crate::components::{
    BoldSelection, CodeSelection, Copy, Cut, DeleteBlock, DismissTransientUi, DuplicateBlock,
    IndentBlock, ItalicSelection, MoveBlockDown, MoveBlockUp, OutdentBlock, Paste, Redo, SelectAll,
    SetHeading1, SetHeading2, SetHeading3, SetHeading4, SetHeading5, SetHeading6, SetParagraph,
    StrikethroughSelection, TableColumnAlignment, TableData, ToggleBulletList, ToggleCodeBlock,
    ToggleOrderedList, ToggleQuote, ToggleTaskList, ToggleViewMode, UnderlineSelection, Undo,
    serialize_table_markdown_lines,
};
use crate::theme::Theme;
use gpui::*;
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use rust_i18n::t;

/// Target block position for inserting a native table.
#[derive(Clone, Copy)]
pub(super) enum TableInsertTarget {
    /// Insert the table immediately after the referenced block.
    After(EntityId),
    /// Append the table to the end of the current root list.
    Append,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContextSubmenu {
    Format,
    BlockType,
    Block,
    Insert,
    Table,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContextMenuAction {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Bold,
    Italic,
    Underline,
    Strikethrough,
    InlineCode,
    Paragraph,
    Heading1,
    Heading2,
    Heading3,
    Heading4,
    Heading5,
    Heading6,
    BulletList,
    OrderedList,
    TaskList,
    Quote,
    CodeBlock,
    MoveBlockUp,
    MoveBlockDown,
    DuplicateBlock,
    DeleteBlock,
    IndentBlock,
    OutdentBlock,
    InsertTable,
    ToggleViewMode,
}

#[derive(Clone, Copy)]
enum ContextMenuEntry {
    Action(ContextMenuAction),
    Submenu(ContextSubmenu),
    Separator,
}

/// Target captured during the right-click event before gpui-component builds
/// the popup menu in its deferred frame.
#[derive(Clone, Copy, Default)]
pub(super) struct ContextMenuTargetState {
    pub(super) block_target: Option<EntityId>,
    pub(super) insert_target: Option<TableInsertTarget>,
    pub(super) table_target: Option<TableMenuTarget>,
}

/// State for the table insertion dialog opened from the context menu.
pub(super) struct TableInsertDialogState {
    pub target: TableInsertTarget,
    pub body_rows: usize,
    pub columns: usize,
}

impl Editor {
    pub(super) fn root_ancestor_entity_id(&self, entity_id: EntityId) -> EntityId {
        let mut current = entity_id;
        while let Some(location) = self.document.find_block_location(current) {
            let Some(parent) = location.parent else {
                break;
            };
            current = parent.entity_id();
        }
        current
    }

    pub(super) fn open_table_context_menu(
        &mut self,
        _position: Point<Pixels>,
        block_target: Option<EntityId>,
        target: TableMenuTarget,
        _cx: &mut Context<Self>,
    ) {
        if self.view_mode != ViewMode::Rendered {
            return;
        }

        self.context_menu_target = ContextMenuTargetState {
            block_target,
            insert_target: None,
            table_target: Some(target),
        };
    }

    pub(super) fn clear_context_menu_target(&mut self) {
        self.context_menu_target = ContextMenuTargetState::default();
    }

    pub(super) fn set_block_context_menu_target(&mut self, entity_id: EntityId, cx: &App) {
        if let Some(binding) = self.table_cell_binding(entity_id) {
            self.context_menu_target = ContextMenuTargetState {
                block_target: Some(entity_id),
                insert_target: None,
                table_target: Some(TableMenuTarget {
                    table_block_id: binding.table_block.entity_id(),
                    row: binding.position.row,
                    column: binding.position.column,
                }),
            };
            return;
        }

        let insert_target = (self.view_mode == ViewMode::Rendered
            && self
                .focusable_entity_by_id(entity_id)
                .is_none_or(|block| block.read(cx).kind().allows_context_table_insert()))
        .then(|| TableInsertTarget::After(self.root_ancestor_entity_id(entity_id)));
        self.context_menu_target = ContextMenuTargetState {
            block_target: Some(entity_id),
            insert_target,
            table_target: None,
        };
    }

    pub(super) fn close_table_insert_dialog(&mut self, cx: &mut Context<Self>) {
        if self.table_insert_dialog.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn dismiss_contextual_overlays(&mut self, cx: &mut Context<Self>) {
        let had_dialog = self.table_insert_dialog.take().is_some();
        let had_enlarged = self.enlarged_block.take().is_some();
        if had_dialog || had_enlarged {
            cx.notify();
        }
    }

    pub(super) fn on_dismiss_context_menu_overlay(
        &mut self,
        _event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_contextual_overlays(cx);
    }

    pub(super) fn on_dismiss_transient_ui(
        &mut self,
        _: &DismissTransientUi,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_contextual_overlays(cx);
    }

    fn open_table_insert_dialog(
        &mut self,
        target: Option<TableInsertTarget>,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = target else {
            return;
        };
        self.table_insert_dialog = Some(TableInsertDialogState {
            target,
            body_rows: 2,
            columns: 2,
        });
        cx.notify();
    }

    pub(super) fn on_table_rows_decrement(
        &mut self,
        _event: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(dialog) = self.table_insert_dialog.as_mut() {
            dialog.body_rows = dialog.body_rows.saturating_sub(1).max(1);
            cx.notify();
        }
    }

    pub(super) fn on_table_rows_increment(
        &mut self,
        _event: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(dialog) = self.table_insert_dialog.as_mut() {
            dialog.body_rows += 1;
            cx.notify();
        }
    }

    pub(super) fn on_table_columns_decrement(
        &mut self,
        _event: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(dialog) = self.table_insert_dialog.as_mut() {
            dialog.columns = dialog.columns.saturating_sub(1).max(1);
            cx.notify();
        }
    }

    pub(super) fn on_table_columns_increment(
        &mut self,
        _event: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(dialog) = self.table_insert_dialog.as_mut() {
            dialog.columns += 1;
            cx.notify();
        }
    }

    pub(super) fn on_cancel_table_insert_dialog(
        &mut self,
        _event: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_table_insert_dialog(cx);
    }

    pub(super) fn on_confirm_table_insert_dialog(
        &mut self,
        _event: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.table_insert_dialog.take() else {
            return;
        };

        self.prepare_undo_capture(crate::components::UndoCaptureKind::NonCoalescible, cx);
        let table = TableData::new_empty(dialog.body_rows, dialog.columns);
        let new_block = Self::new_table_block(cx, table);

        match dialog.target {
            TableInsertTarget::After(entity_id) => {
                if let Some(location) = self.document.find_block_location(entity_id) {
                    self.document.insert_blocks_at(
                        location.parent,
                        location.index + 1,
                        vec![new_block.clone()],
                        cx,
                    );
                } else {
                    self.document.insert_blocks_at(
                        None,
                        self.document.root_count(),
                        vec![new_block.clone()],
                        cx,
                    );
                }
            }
            TableInsertTarget::Append => {
                self.document.insert_blocks_at(
                    None,
                    self.document.root_count(),
                    vec![new_block.clone()],
                    cx,
                );
            }
        }

        // A table inserted as the last block in its container leaves no line
        // below it, so in rendered mode the caret cannot move past the table.
        // Add a trailing empty paragraph to land on when nothing follows it.
        self.ensure_trailing_paragraph_after_structural(&new_block, cx);

        self.rebuild_table_runtimes(cx);
        if let Some(first_cell) = new_block
            .read(cx)
            .table_runtime
            .as_ref()
            .and_then(|runtime| runtime.header.first())
        {
            self.focus_block(first_cell.entity_id());
        }
        self.mark_dirty(cx);
        self.finalize_pending_undo_capture(cx);
        self.request_active_block_scroll_into_view(cx);
        cx.notify();
    }

    fn apply_context_menu_action(
        &mut self,
        action: ContextMenuAction,
        block_target: Option<EntityId>,
        insert_target: Option<TableInsertTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action == ContextMenuAction::InsertTable {
            self.open_table_insert_dialog(insert_target, cx);
            return;
        }

        if let Some(entity_id) = block_target
            && let Some(block) = self.focusable_entity_by_id(entity_id)
        {
            self.active_entity_id = Some(entity_id);
            block.read(cx).focus_handle.clone().focus(window, cx);
        }
        match action {
            ContextMenuAction::Undo => window.dispatch_action(Box::new(Undo), cx),
            ContextMenuAction::Redo => window.dispatch_action(Box::new(Redo), cx),
            ContextMenuAction::Cut => window.dispatch_action(Box::new(Cut), cx),
            ContextMenuAction::Copy => window.dispatch_action(Box::new(Copy), cx),
            ContextMenuAction::Paste => window.dispatch_action(Box::new(Paste), cx),
            ContextMenuAction::SelectAll => window.dispatch_action(Box::new(SelectAll), cx),
            ContextMenuAction::Bold => window.dispatch_action(Box::new(BoldSelection), cx),
            ContextMenuAction::Italic => window.dispatch_action(Box::new(ItalicSelection), cx),
            ContextMenuAction::Underline => {
                window.dispatch_action(Box::new(UnderlineSelection), cx)
            }
            ContextMenuAction::Strikethrough => {
                window.dispatch_action(Box::new(StrikethroughSelection), cx)
            }
            ContextMenuAction::InlineCode => window.dispatch_action(Box::new(CodeSelection), cx),
            ContextMenuAction::Paragraph => window.dispatch_action(Box::new(SetParagraph), cx),
            ContextMenuAction::Heading1 => window.dispatch_action(Box::new(SetHeading1), cx),
            ContextMenuAction::Heading2 => window.dispatch_action(Box::new(SetHeading2), cx),
            ContextMenuAction::Heading3 => window.dispatch_action(Box::new(SetHeading3), cx),
            ContextMenuAction::Heading4 => window.dispatch_action(Box::new(SetHeading4), cx),
            ContextMenuAction::Heading5 => window.dispatch_action(Box::new(SetHeading5), cx),
            ContextMenuAction::Heading6 => window.dispatch_action(Box::new(SetHeading6), cx),
            ContextMenuAction::BulletList => window.dispatch_action(Box::new(ToggleBulletList), cx),
            ContextMenuAction::OrderedList => {
                window.dispatch_action(Box::new(ToggleOrderedList), cx)
            }
            ContextMenuAction::TaskList => window.dispatch_action(Box::new(ToggleTaskList), cx),
            ContextMenuAction::Quote => window.dispatch_action(Box::new(ToggleQuote), cx),
            ContextMenuAction::CodeBlock => window.dispatch_action(Box::new(ToggleCodeBlock), cx),
            ContextMenuAction::MoveBlockUp => window.dispatch_action(Box::new(MoveBlockUp), cx),
            ContextMenuAction::MoveBlockDown => window.dispatch_action(Box::new(MoveBlockDown), cx),
            ContextMenuAction::DuplicateBlock => {
                window.dispatch_action(Box::new(DuplicateBlock), cx)
            }
            ContextMenuAction::DeleteBlock => window.dispatch_action(Box::new(DeleteBlock), cx),
            ContextMenuAction::IndentBlock => window.dispatch_action(Box::new(IndentBlock), cx),
            ContextMenuAction::OutdentBlock => window.dispatch_action(Box::new(OutdentBlock), cx),
            ContextMenuAction::ToggleViewMode => {
                window.dispatch_action(Box::new(ToggleViewMode), cx)
            }
            ContextMenuAction::InsertTable => unreachable!("handled before action dispatch"),
        }
    }

    fn apply_table_menu_action(
        &mut self,
        action: TableMenuAction,
        target: TableMenuTarget,
        cx: &mut Context<Self>,
    ) {
        let Some(table_block) = self.table_block_by_id(target.table_block_id, cx) else {
            return;
        };
        match action {
            TableMenuAction::InsertRowAbove => {
                self.insert_table_row(&table_block, target.row, false, cx)
            }
            TableMenuAction::InsertRowBelow => {
                self.insert_table_row(&table_block, target.row, true, cx)
            }
            TableMenuAction::InsertColumnLeft => {
                self.insert_table_column(&table_block, target.column, false, cx)
            }
            TableMenuAction::InsertColumnRight => {
                self.insert_table_column(&table_block, target.column, true, cx)
            }
            TableMenuAction::AlignColumnLeft => self.set_table_column_alignment(
                &table_block,
                target.column,
                TableColumnAlignment::Left,
                cx,
            ),
            TableMenuAction::AlignColumnCenter => self.set_table_column_alignment(
                &table_block,
                target.column,
                TableColumnAlignment::Center,
                cx,
            ),
            TableMenuAction::AlignColumnRight => self.set_table_column_alignment(
                &table_block,
                target.column,
                TableColumnAlignment::Right,
                cx,
            ),
            TableMenuAction::DeleteRow => self.delete_table_menu_row(&table_block, target.row, cx),
            TableMenuAction::DeleteColumn => {
                self.delete_table_menu_column(&table_block, target.column, cx)
            }
            TableMenuAction::CopyTable => self.copy_table_markdown(&table_block, cx),
            TableMenuAction::FormatTableSource => self.format_table_source(&table_block, cx),
            TableMenuAction::DeleteTable => self.remove_table_block(&table_block, cx),
            TableMenuAction::MoveTableRowUp | TableMenuAction::MoveTableRowDown => {
                let delta = table_menu_move_delta(action).expect("row move action has a delta");
                self.move_table_row(&table_block, target.row, delta, cx);
            }
            TableMenuAction::MoveTableColumnLeft | TableMenuAction::MoveTableColumnRight => {
                let delta = table_menu_move_delta(action).expect("column move action has a delta");
                self.move_table_column(&table_block, target.column, delta, cx);
            }
        }
    }

    fn delete_table_menu_row(
        &mut self,
        table_block: &Entity<crate::components::Block>,
        visual_row: usize,
        cx: &mut Context<Self>,
    ) {
        let body_rows = table_block
            .read(cx)
            .record
            .table
            .as_ref()
            .map(|table| table.rows.len());
        if visual_row == 0 && body_rows == Some(0) {
            self.remove_table_block(table_block, cx);
        } else if visual_row == 0 {
            self.delete_table_header_row(table_block, cx);
        } else {
            self.delete_table_row(table_block, visual_row - 1, cx);
        }
    }

    fn delete_table_menu_column(
        &mut self,
        table_block: &Entity<crate::components::Block>,
        column: usize,
        cx: &mut Context<Self>,
    ) {
        let columns = table_block
            .read(cx)
            .record
            .table
            .as_ref()
            .map(TableData::column_count);
        if columns == Some(1) {
            self.remove_table_block(table_block, cx);
        } else {
            self.delete_table_column(table_block, column, cx);
        }
    }

    fn copy_table_markdown(
        &mut self,
        table_block: &Entity<crate::components::Block>,
        cx: &mut Context<Self>,
    ) {
        self.sync_table_record_from_runtime(table_block, cx);
        let markdown = table_block
            .read(cx)
            .record
            .table
            .as_ref()
            .map(serialize_table_markdown_lines)
            .map(|lines| lines.join("\n"));
        if let Some(markdown) = markdown {
            cx.write_to_clipboard(ClipboardItem::new_string(markdown));
        }
    }

    fn format_table_source(
        &mut self,
        table_block: &Entity<crate::components::Block>,
        cx: &mut Context<Self>,
    ) {
        self.sync_table_record_from_runtime(table_block, cx);
        self.mark_dirty(cx);
        cx.notify();
    }

    fn table_menu_label(action: TableMenuAction) -> String {
        match action {
            TableMenuAction::InsertRowAbove => t!("MarkdownEditor.table_menu_insert_row_above"),
            TableMenuAction::InsertRowBelow => t!("MarkdownEditor.table_menu_insert_row_below"),
            TableMenuAction::InsertColumnLeft => t!("MarkdownEditor.table_menu_insert_column_left"),
            TableMenuAction::InsertColumnRight => {
                t!("MarkdownEditor.table_menu_insert_column_right")
            }
            TableMenuAction::AlignColumnLeft => t!("MarkdownEditor.table_axis_align_column_left"),
            TableMenuAction::AlignColumnCenter => {
                t!("MarkdownEditor.table_axis_align_column_center")
            }
            TableMenuAction::AlignColumnRight => t!("MarkdownEditor.table_axis_align_column_right"),
            TableMenuAction::MoveTableRowUp => t!("MarkdownEditor.table_axis_move_row_up"),
            TableMenuAction::MoveTableRowDown => t!("MarkdownEditor.table_axis_move_row_down"),
            TableMenuAction::MoveTableColumnLeft => {
                t!("MarkdownEditor.table_axis_move_column_left")
            }
            TableMenuAction::MoveTableColumnRight => {
                t!("MarkdownEditor.table_axis_move_column_right")
            }
            TableMenuAction::DeleteRow => t!("MarkdownEditor.table_menu_delete_row"),
            TableMenuAction::DeleteColumn => t!("MarkdownEditor.table_menu_delete_column"),
            TableMenuAction::CopyTable => t!("MarkdownEditor.table_menu_copy_table"),
            TableMenuAction::FormatTableSource => t!("MarkdownEditor.table_menu_format_source"),
            TableMenuAction::DeleteTable => t!("MarkdownEditor.table_menu_delete_table"),
        }
        .to_string()
    }

    fn context_submenu_entries(submenu: ContextSubmenu) -> Vec<ContextMenuEntry> {
        match submenu {
            ContextSubmenu::Format => vec![
                ContextMenuEntry::Action(ContextMenuAction::Bold),
                ContextMenuEntry::Action(ContextMenuAction::Italic),
                ContextMenuEntry::Action(ContextMenuAction::Underline),
                ContextMenuEntry::Action(ContextMenuAction::Strikethrough),
                ContextMenuEntry::Action(ContextMenuAction::InlineCode),
            ],
            ContextSubmenu::BlockType => vec![
                ContextMenuEntry::Action(ContextMenuAction::Paragraph),
                ContextMenuEntry::Action(ContextMenuAction::Heading1),
                ContextMenuEntry::Action(ContextMenuAction::Heading2),
                ContextMenuEntry::Action(ContextMenuAction::Heading3),
                ContextMenuEntry::Action(ContextMenuAction::Heading4),
                ContextMenuEntry::Action(ContextMenuAction::Heading5),
                ContextMenuEntry::Action(ContextMenuAction::Heading6),
                ContextMenuEntry::Separator,
                ContextMenuEntry::Action(ContextMenuAction::BulletList),
                ContextMenuEntry::Action(ContextMenuAction::OrderedList),
                ContextMenuEntry::Action(ContextMenuAction::TaskList),
                ContextMenuEntry::Action(ContextMenuAction::Quote),
                ContextMenuEntry::Action(ContextMenuAction::CodeBlock),
            ],
            ContextSubmenu::Block => vec![
                ContextMenuEntry::Action(ContextMenuAction::MoveBlockUp),
                ContextMenuEntry::Action(ContextMenuAction::MoveBlockDown),
                ContextMenuEntry::Action(ContextMenuAction::DuplicateBlock),
                ContextMenuEntry::Action(ContextMenuAction::DeleteBlock),
                ContextMenuEntry::Separator,
                ContextMenuEntry::Action(ContextMenuAction::IndentBlock),
                ContextMenuEntry::Action(ContextMenuAction::OutdentBlock),
            ],
            ContextSubmenu::Insert => {
                vec![ContextMenuEntry::Action(ContextMenuAction::InsertTable)]
            }
            ContextSubmenu::Table => Vec::new(),
        }
    }

    fn context_menu_action_label(action: ContextMenuAction) -> String {
        match action {
            ContextMenuAction::Undo => t!("MarkdownEditor.context_menu_undo").to_string(),
            ContextMenuAction::Redo => t!("MarkdownEditor.context_menu_redo").to_string(),
            ContextMenuAction::Cut => t!("MarkdownEditor.context_menu_cut").to_string(),
            ContextMenuAction::Copy => t!("MarkdownEditor.context_menu_copy").to_string(),
            ContextMenuAction::Paste => t!("MarkdownEditor.context_menu_paste").to_string(),
            ContextMenuAction::SelectAll => {
                t!("MarkdownEditor.context_menu_select_all").to_string()
            }
            ContextMenuAction::Bold => t!("MarkdownEditor.context_menu_bold").to_string(),
            ContextMenuAction::Italic => t!("MarkdownEditor.context_menu_italic").to_string(),
            ContextMenuAction::Underline => t!("MarkdownEditor.context_menu_underline").to_string(),
            ContextMenuAction::Strikethrough => {
                t!("MarkdownEditor.context_menu_strikethrough").to_string()
            }
            ContextMenuAction::InlineCode => {
                t!("MarkdownEditor.context_menu_inline_code").to_string()
            }
            ContextMenuAction::Paragraph => t!("MarkdownEditor.context_menu_paragraph").to_string(),
            ContextMenuAction::Heading1 => t!("MarkdownEditor.context_menu_heading_1").to_string(),
            ContextMenuAction::Heading2 => t!("MarkdownEditor.context_menu_heading_2").to_string(),
            ContextMenuAction::Heading3 => t!("MarkdownEditor.context_menu_heading_3").to_string(),
            ContextMenuAction::Heading4 => t!("MarkdownEditor.context_menu_heading_4").to_string(),
            ContextMenuAction::Heading5 => t!("MarkdownEditor.context_menu_heading_5").to_string(),
            ContextMenuAction::Heading6 => t!("MarkdownEditor.context_menu_heading_6").to_string(),
            ContextMenuAction::BulletList => {
                t!("MarkdownEditor.context_menu_bullet_list").to_string()
            }
            ContextMenuAction::OrderedList => {
                t!("MarkdownEditor.context_menu_ordered_list").to_string()
            }
            ContextMenuAction::TaskList => t!("MarkdownEditor.context_menu_task_list").to_string(),
            ContextMenuAction::Quote => t!("MarkdownEditor.context_menu_quote").to_string(),
            ContextMenuAction::CodeBlock => {
                t!("MarkdownEditor.context_menu_code_block").to_string()
            }
            ContextMenuAction::MoveBlockUp => {
                t!("MarkdownEditor.context_menu_move_block_up").to_string()
            }
            ContextMenuAction::MoveBlockDown => {
                t!("MarkdownEditor.context_menu_move_block_down").to_string()
            }
            ContextMenuAction::DuplicateBlock => {
                t!("MarkdownEditor.context_menu_duplicate_block").to_string()
            }
            ContextMenuAction::DeleteBlock => {
                t!("MarkdownEditor.context_menu_delete_block").to_string()
            }
            ContextMenuAction::IndentBlock => {
                t!("MarkdownEditor.context_menu_indent_block").to_string()
            }
            ContextMenuAction::OutdentBlock => {
                t!("MarkdownEditor.context_menu_outdent_block").to_string()
            }
            ContextMenuAction::InsertTable => t!("MarkdownEditor.context_menu_table").to_string(),
            ContextMenuAction::ToggleViewMode => {
                t!("MarkdownEditor.context_menu_toggle_view_mode").to_string()
            }
        }
    }

    fn context_submenu_label(submenu: ContextSubmenu) -> String {
        match submenu {
            ContextSubmenu::Format => t!("MarkdownEditor.context_menu_format").to_string(),
            ContextSubmenu::BlockType => t!("MarkdownEditor.context_menu_block_type").to_string(),
            ContextSubmenu::Block => t!("MarkdownEditor.context_menu_block").to_string(),
            ContextSubmenu::Insert => t!("MarkdownEditor.context_menu_insert").to_string(),
            ContextSubmenu::Table => t!("MarkdownEditor.context_menu_table").to_string(),
        }
    }

    fn append_popup_actions(
        mut menu: PopupMenu,
        entries: impl IntoIterator<Item = ContextMenuEntry>,
        editor: &Entity<Self>,
        block_target: Option<EntityId>,
        insert_target: Option<TableInsertTarget>,
        table_target: Option<TableMenuTarget>,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        for entry in entries {
            match entry {
                ContextMenuEntry::Action(action) => {
                    let editor = editor.clone();
                    menu = menu.item(
                        PopupMenuItem::new(Self::context_menu_action_label(action)).on_click(
                            window.listener_for(&editor, move |editor, _, window, cx| {
                                editor.apply_context_menu_action(
                                    action,
                                    block_target,
                                    insert_target,
                                    window,
                                    cx,
                                );
                            }),
                        ),
                    );
                }
                ContextMenuEntry::Separator => {
                    menu = menu.separator();
                }
                ContextMenuEntry::Submenu(submenu) => {
                    let editor = editor.clone();
                    menu = menu.submenu(
                        Self::context_submenu_label(submenu),
                        window,
                        cx,
                        move |submenu_menu, window, cx| {
                            if submenu == ContextSubmenu::Table {
                                return Self::append_popup_table_actions(
                                    submenu_menu,
                                    table_target,
                                    &editor,
                                    window,
                                    cx,
                                );
                            }
                            Self::append_popup_actions(
                                submenu_menu,
                                Self::context_submenu_entries(submenu),
                                &editor,
                                block_target,
                                insert_target,
                                table_target,
                                window,
                                cx,
                            )
                        },
                    );
                }
            }
        }
        menu
    }

    fn append_popup_table_actions(
        mut menu: PopupMenu,
        target: Option<TableMenuTarget>,
        editor: &Entity<Self>,
        window: &mut Window,
        _cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let Some(target) = target else {
            return menu;
        };
        for entry in table_menu_entries() {
            match entry {
                TableMenuEntry::Action(action) => {
                    let editor = editor.clone();
                    menu = menu.item(
                        PopupMenuItem::new(Self::table_menu_label(*action)).on_click(
                            window.listener_for(&editor, move |editor, _, _, cx| {
                                editor.apply_table_menu_action(*action, target, cx);
                            }),
                        ),
                    );
                }
                TableMenuEntry::Separator => menu = menu.separator(),
            }
        }
        menu
    }

    /// Build the native gpui-component context menu for a block or editor
    /// surface. Targets are captured at the point where the user right-clicks,
    /// so the menu does not depend on a separate editor-owned overlay state.
    pub(super) fn build_popup_context_menu(
        editor: Entity<Self>,
        block_target: Option<EntityId>,
        insert_target: Option<TableInsertTarget>,
        table_target: Option<TableMenuTarget>,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let rendered = editor.read(cx).view_mode == ViewMode::Rendered;
        let mut entries = vec![
            ContextMenuEntry::Action(ContextMenuAction::Undo),
            ContextMenuEntry::Action(ContextMenuAction::Redo),
            ContextMenuEntry::Separator,
            ContextMenuEntry::Action(ContextMenuAction::Cut),
            ContextMenuEntry::Action(ContextMenuAction::Copy),
            ContextMenuEntry::Action(ContextMenuAction::Paste),
            ContextMenuEntry::Action(ContextMenuAction::SelectAll),
        ];
        if block_target.is_some() {
            entries.push(ContextMenuEntry::Separator);
            entries.push(ContextMenuEntry::Submenu(ContextSubmenu::Format));
            if rendered {
                entries.extend([
                    ContextMenuEntry::Submenu(ContextSubmenu::BlockType),
                    ContextMenuEntry::Submenu(ContextSubmenu::Block),
                ]);
            }
        }
        if rendered {
            if insert_target.is_some() {
                if !matches!(entries.last(), Some(ContextMenuEntry::Separator)) {
                    entries.push(ContextMenuEntry::Separator);
                }
                entries.push(ContextMenuEntry::Submenu(ContextSubmenu::Insert));
            }
            if table_target.is_some() {
                if !matches!(entries.last(), Some(ContextMenuEntry::Separator)) {
                    entries.push(ContextMenuEntry::Separator);
                }
                entries.push(ContextMenuEntry::Submenu(ContextSubmenu::Table));
            }
        }
        entries.extend([
            ContextMenuEntry::Separator,
            ContextMenuEntry::Action(ContextMenuAction::ToggleViewMode),
        ]);

        // The table submenu is the only submenu whose contents are target
        // dependent; build it separately so row/column actions keep the exact
        // cell that was clicked.
        let menu = Self::append_popup_actions(
            menu,
            entries.into_iter(),
            &editor,
            block_target,
            insert_target,
            table_target,
            window,
            cx,
        );
        menu
    }

    pub(super) fn render_table_insert_dialog_overlay(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let dialog = self.table_insert_dialog.as_ref()?;
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;

        let stepper =
            |id_prefix: &'static str,
             label: String,
             value: usize,
             on_dec: fn(&mut Editor, &ClickEvent, &mut Window, &mut Context<Editor>),
             on_inc: fn(&mut Editor, &ClickEvent, &mut Window, &mut Context<Editor>)| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(d.table_insert_stepper_gap))
                    .child(
                        div()
                            .text_size(px(t.dialog_body_size))
                            .font_weight(t.dialog_button_weight.to_font_weight())
                            .text_color(c.dialog_body)
                            .child(label),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(d.table_insert_stepper_gap))
                            .child(
                                div()
                                    .id((id_prefix, 0usize))
                                    .size(px(d.table_insert_stepper_button_size))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(d.table_insert_stepper_radius))
                                    .border(px(d.dialog_border_width))
                                    .border_color(c.dialog_border)
                                    .bg(c.dialog_secondary_button_bg)
                                    .hover(|this| this.bg(c.dialog_secondary_button_hover))
                                    .cursor_pointer()
                                    .text_color(c.dialog_secondary_button_text)
                                    .on_click(cx.listener(on_dec))
                                    .child("-"),
                            )
                            .child(
                                div()
                                    .min_w(px(d.table_insert_stepper_value_min_width))
                                    .h(px(d.table_insert_stepper_button_size))
                                    .px(px(d.table_insert_stepper_value_padding_x))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(d.table_insert_stepper_radius))
                                    .border(px(d.dialog_border_width))
                                    .border_color(c.dialog_border)
                                    .bg(c.dialog_surface)
                                    .text_size(px(t.dialog_body_size))
                                    .text_color(c.dialog_title)
                                    .child(value.to_string()),
                            )
                            .child(
                                div()
                                    .id((id_prefix, 1usize))
                                    .size(px(d.table_insert_stepper_button_size))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(d.table_insert_stepper_radius))
                                    .border(px(d.dialog_border_width))
                                    .border_color(c.dialog_border)
                                    .bg(c.dialog_secondary_button_bg)
                                    .hover(|this| this.bg(c.dialog_secondary_button_hover))
                                    .cursor_pointer()
                                    .text_color(c.dialog_secondary_button_text)
                                    .on_click(cx.listener(on_inc))
                                    .child("+"),
                            ),
                    )
            };

        Some(
            div()
                .id("table-insert-dialog-overlay")
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(c.dialog_backdrop)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(Self::on_dismiss_context_menu_overlay),
                )
                .child(
                    div()
                        .w_full()
                        .px(px(d.editor_padding))
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .id("table-insert-dialog")
                                .w(px(d.dialog_width.min(d.table_insert_dialog_width)))
                                .max_w(relative(1.0))
                                .p(px(d.dialog_padding))
                                .flex()
                                .flex_col()
                                .gap(px(d.dialog_gap))
                                .bg(c.dialog_surface)
                                .border(px(d.dialog_border_width))
                                .border_color(c.dialog_border)
                                .rounded(px(d.dialog_radius))
                                .shadow_lg()
                                .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                                    cx.stop_propagation()
                                })
                                .child(
                                    div()
                                        .text_size(px(t.dialog_title_size))
                                        .font_weight(t.dialog_title_weight.to_font_weight())
                                        .text_color(c.dialog_title)
                                        .child(t!("MarkdownEditor.table_insert_title").to_string()),
                                )
                                .child(
                                    div()
                                        .text_size(px(t.dialog_body_size))
                                        .font_weight(t.dialog_body_weight.to_font_weight())
                                        .text_color(c.dialog_body)
                                        .child(
                                            t!("MarkdownEditor.table_insert_description")
                                                .to_string(),
                                        ),
                                )
                                .child(stepper(
                                    "table-body-rows",
                                    t!("MarkdownEditor.table_insert_body_rows").to_string(),
                                    dialog.body_rows,
                                    Self::on_table_rows_decrement,
                                    Self::on_table_rows_increment,
                                ))
                                .child(stepper(
                                    "table-columns",
                                    t!("MarkdownEditor.table_insert_columns").to_string(),
                                    dialog.columns,
                                    Self::on_table_columns_decrement,
                                    Self::on_table_columns_increment,
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .justify_end()
                                        .gap(px(d.dialog_button_gap))
                                        .child(
                                            div()
                                                .id("cancel-table-insert-dialog")
                                                .h(px(d.dialog_button_height))
                                                .px(px(d.dialog_button_padding_x))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px((d.dialog_radius - 4.0).max(0.0)))
                                                .border(px(d.dialog_border_width))
                                                .border_color(c.dialog_border)
                                                .bg(c.dialog_secondary_button_bg)
                                                .hover(|this| {
                                                    this.bg(c.dialog_secondary_button_hover)
                                                })
                                                .cursor_pointer()
                                                .text_size(px(t.dialog_button_size))
                                                .font_weight(
                                                    t.dialog_button_weight.to_font_weight(),
                                                )
                                                .text_color(c.dialog_secondary_button_text)
                                                .on_click(
                                                    cx.listener(
                                                        Self::on_cancel_table_insert_dialog,
                                                    ),
                                                )
                                                .child(
                                                    t!("MarkdownEditor.table_insert_cancel")
                                                        .to_string(),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .id("confirm-table-insert-dialog")
                                                .h(px(d.dialog_button_height))
                                                .px(px(d.dialog_button_padding_x))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .rounded(px((d.dialog_radius - 4.0).max(0.0)))
                                                .bg(c.dialog_primary_button_bg)
                                                .hover(|this| {
                                                    this.bg(c.dialog_primary_button_hover)
                                                })
                                                .cursor_pointer()
                                                .text_size(px(t.dialog_button_size))
                                                .font_weight(
                                                    t.dialog_button_weight.to_font_weight(),
                                                )
                                                .text_color(c.dialog_primary_button_text)
                                                .on_click(
                                                    cx.listener(
                                                        Self::on_confirm_table_insert_dialog,
                                                    ),
                                                )
                                                .child(
                                                    t!("MarkdownEditor.table_insert_confirm")
                                                        .to_string(),
                                                ),
                                        ),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Editor;
    use crate::components::BlockKind;
    use gpui::{AppContext, TestAppContext};

    #[gpui::test]
    async fn block_context_target_tracks_the_exact_table_cell(cx: &mut TestAppContext) {
        let editor = cx.new(|cx| {
            Editor::from_markdown(cx, "| A | B |\n| --- | --- |\n| 1 | 2 |".to_string(), None)
        });
        let (table_block_id, cell_id) = editor.read_with(cx, |editor, cx| {
            let table = editor
                .document
                .visible_blocks()
                .into_iter()
                .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
                .expect("the markdown table becomes a table block")
                .entity
                .clone();
            let cell_id = table
                .read(cx)
                .table_runtime
                .as_ref()
                .and_then(|runtime| runtime.rows.first())
                .and_then(|row| row.get(1))
                .expect("the table has a second-column body cell")
                .entity_id();
            (table.entity_id(), cell_id)
        });

        editor.update(cx, |editor, cx| {
            editor.set_block_context_menu_target(cell_id, cx);
            let target = editor
                .context_menu_target
                .table_target
                .expect("table target should be captured");
            assert_eq!(target.table_block_id, table_block_id);
            assert_eq!(target.row, 1);
            assert_eq!(target.column, 1);

            editor.clear_context_menu_target();
            assert!(editor.context_menu_target.block_target.is_none());
            assert!(editor.context_menu_target.table_target.is_none());
        });
    }
}
