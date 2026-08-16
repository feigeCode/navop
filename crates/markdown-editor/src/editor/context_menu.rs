//! Editor context menus and native table insertion dialog.

use std::time::Duration;

use super::table_menu::{
    TableMenuAction, TableMenuEntry, TableMenuTarget, place_context_menu, table_menu_entries,
    table_menu_move_delta,
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
use rust_i18n::t;

const CONTEXT_MENU_VIEWPORT_MARGIN: f32 = 8.0;

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

/// Context menu currently open in the editor.
pub(super) struct ContextMenuState {
    position: Point<Pixels>,
    block_target: Option<EntityId>,
    insert_target: Option<TableInsertTarget>,
    pub(super) table_target: Option<TableMenuTarget>,
    trigger_hovered: bool,
    submenu_hovered: bool,
    open_submenu: Option<ContextSubmenu>,
}

/// State for the table insertion dialog opened from the context menu.
pub(super) struct TableInsertDialogState {
    pub target: TableInsertTarget,
    pub body_rows: usize,
    pub columns: usize,
}

macro_rules! table_menu_handler {
    ($name:ident, $action:expr) => {
        fn $name(&mut self, _event: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
            self.apply_table_menu_action($action, cx);
        }
    };
}

impl Editor {
    fn context_menu_position_in_editor(&self, position: Point<Pixels>) -> Point<Pixels> {
        point(
            (position.x - self.root_bounds.origin.x).max(px(0.0)),
            (position.y - self.root_bounds.origin.y).max(px(0.0)),
        )
    }

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

    fn open_context_menu(
        &mut self,
        position: Point<Pixels>,
        block_target: Option<EntityId>,
        insert_target: Option<TableInsertTarget>,
        table_target: Option<TableMenuTarget>,
        cx: &mut Context<Self>,
    ) {
        self.context_menu_submenu_close_task = None;
        let position = self.context_menu_position_in_editor(position);
        self.context_menu = Some(ContextMenuState {
            position,
            block_target,
            insert_target,
            table_target,
            trigger_hovered: false,
            submenu_hovered: false,
            open_submenu: None,
        });
        cx.notify();
    }

    pub(super) fn open_table_context_menu(
        &mut self,
        position: Point<Pixels>,
        block_target: Option<EntityId>,
        target: TableMenuTarget,
        cx: &mut Context<Self>,
    ) {
        if self.view_mode != ViewMode::Rendered {
            return;
        }

        self.open_context_menu(position, block_target, None, Some(target), cx);
    }

    pub(super) fn close_table_insert_dialog(&mut self, cx: &mut Context<Self>) {
        if self.table_insert_dialog.take().is_some() {
            cx.notify();
        }
    }

    fn close_context_menu(&mut self, cx: &mut Context<Self>) {
        let had_menu = self.context_menu.take().is_some();
        let had_submenu_close = self.context_menu_submenu_close_task.take().is_some();
        if had_menu || had_submenu_close {
            cx.notify();
        }
    }

    pub(super) fn dismiss_contextual_overlays(&mut self, cx: &mut Context<Self>) {
        let had_menu = self.context_menu.take().is_some();
        let had_dialog = self.table_insert_dialog.take().is_some();
        let had_enlarged = self.enlarged_block.take().is_some();
        let had_submenu_close = self.context_menu_submenu_close_task.take().is_some();
        if had_menu || had_dialog || had_enlarged || had_submenu_close {
            cx.notify();
        }
    }

    fn schedule_context_menu_submenu_close(&mut self, cx: &mut Context<Self>) {
        let expected_submenu = self
            .context_menu
            .as_ref()
            .and_then(|menu| menu.open_submenu);
        if expected_submenu.is_none() {
            return;
        }

        let weak_editor = cx.entity().downgrade();
        self.context_menu_submenu_close_task = Some(cx.spawn(
            async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let _ = weak_editor.update(cx, |editor, cx| {
                    editor.context_menu_submenu_close_task = None;
                    let Some(menu) = editor.context_menu.as_mut() else {
                        return;
                    };
                    if !menu.trigger_hovered
                        && !menu.submenu_hovered
                        && menu.open_submenu == expected_submenu
                    {
                        menu.open_submenu = None;
                        cx.notify();
                    }
                });
            },
        ));
    }

    fn set_context_menu_hover_state(
        &mut self,
        hovered: bool,
        submenu: Option<ContextSubmenu>,
        cx: &mut Context<Self>,
    ) {
        let mut changed = false;
        let mut should_clear_close = false;
        let mut should_schedule_close = false;

        if let Some(menu) = self.context_menu.as_mut() {
            if submenu.is_none() {
                if menu.submenu_hovered != hovered {
                    menu.submenu_hovered = hovered;
                    changed = true;
                }
            } else if menu.trigger_hovered != hovered {
                menu.trigger_hovered = hovered;
                changed = true;
            }

            if hovered {
                should_clear_close = true;
                if let Some(submenu) = submenu
                    && menu.open_submenu != Some(submenu)
                {
                    menu.open_submenu = Some(submenu);
                    changed = true;
                }
            } else {
                if !menu.trigger_hovered && !menu.submenu_hovered {
                    should_schedule_close = true;
                }
            }
        }

        if should_clear_close {
            self.context_menu_submenu_close_task = None;
        }
        if should_schedule_close {
            self.schedule_context_menu_submenu_close(cx);
        }
        if changed {
            cx.notify();
        }
    }

    pub(super) fn on_editor_context_menu_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.open_context_menu(
            event.position,
            None,
            (self.view_mode == ViewMode::Rendered).then_some(TableInsertTarget::Append),
            None,
            cx,
        );
    }

    pub(super) fn on_block_context_menu_mouse_down(
        &mut self,
        entity_id: EntityId,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        if let Some(binding) = self.table_cell_binding(entity_id) {
            self.open_table_context_menu(
                event.position,
                Some(entity_id),
                TableMenuTarget {
                    table_block_id: binding.table_block.entity_id(),
                    row: binding.position.row,
                    column: binding.position.column,
                },
                cx,
            );
            return;
        }
        let allows_insert = self
            .focusable_entity_by_id(entity_id)
            .is_none_or(|block| block.read(cx).kind().allows_context_table_insert());
        let insert_target = (self.view_mode == ViewMode::Rendered && allows_insert)
            .then(|| TableInsertTarget::After(self.root_ancestor_entity_id(entity_id)));
        self.open_context_menu(event.position, Some(entity_id), insert_target, None, cx);
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

    pub(super) fn on_context_menu_submenu_hover(
        &mut self,
        hovered: &bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_context_menu_hover_state(*hovered, None, cx);
    }

    fn open_table_insert_dialog_from_context_menu(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.context_menu.take().and_then(|menu| menu.insert_target) else {
            return;
        };
        self.context_menu_submenu_close_task = None;
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

    fn active_table_menu_target(&self) -> Option<TableMenuTarget> {
        self.context_menu.as_ref()?.table_target
    }

    fn apply_context_menu_action(
        &mut self,
        action: ContextMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action == ContextMenuAction::InsertTable {
            self.open_table_insert_dialog_from_context_menu(cx);
            return;
        }

        let block_target = self
            .context_menu
            .as_ref()
            .and_then(|menu| menu.block_target);
        if let Some(entity_id) = block_target
            && let Some(block) = self.focusable_entity_by_id(entity_id)
        {
            self.active_entity_id = Some(entity_id);
            block.read(cx).focus_handle.clone().focus(window, cx);
        }
        self.close_context_menu(cx);

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

    fn apply_table_menu_action(&mut self, action: TableMenuAction, cx: &mut Context<Self>) {
        let Some(target) = self.active_table_menu_target() else {
            return;
        };
        let Some(table_block) = self.table_block_by_id(target.table_block_id, cx) else {
            return;
        };
        self.close_context_menu(cx);
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

    table_menu_handler!(on_insert_table_row_above, TableMenuAction::InsertRowAbove);
    table_menu_handler!(on_insert_table_row_below, TableMenuAction::InsertRowBelow);
    table_menu_handler!(
        on_insert_table_column_left,
        TableMenuAction::InsertColumnLeft
    );
    table_menu_handler!(
        on_insert_table_column_right,
        TableMenuAction::InsertColumnRight
    );
    table_menu_handler!(on_align_table_column_left, TableMenuAction::AlignColumnLeft);
    table_menu_handler!(
        on_align_table_column_center,
        TableMenuAction::AlignColumnCenter
    );
    table_menu_handler!(
        on_align_table_column_right,
        TableMenuAction::AlignColumnRight
    );
    table_menu_handler!(on_move_table_row_up, TableMenuAction::MoveTableRowUp);
    table_menu_handler!(on_move_table_row_down, TableMenuAction::MoveTableRowDown);
    table_menu_handler!(
        on_move_table_column_left,
        TableMenuAction::MoveTableColumnLeft
    );
    table_menu_handler!(
        on_move_table_column_right,
        TableMenuAction::MoveTableColumnRight
    );
    table_menu_handler!(on_delete_table_menu_row, TableMenuAction::DeleteRow);
    table_menu_handler!(on_delete_table_menu_column, TableMenuAction::DeleteColumn);
    table_menu_handler!(on_copy_table, TableMenuAction::CopyTable);
    table_menu_handler!(on_format_table_source, TableMenuAction::FormatTableSource);
    table_menu_handler!(on_delete_table, TableMenuAction::DeleteTable);

    fn render_table_menu_item(
        theme: &Theme,
        id: &'static str,
        label: String,
        enabled: bool,
        danger: bool,
        on_click: fn(&mut Editor, &ClickEvent, &mut Window, &mut Context<Editor>),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        if enabled {
            div()
                .id(id)
                .h(px(d.menu_item_height))
                .px(px(d.menu_item_padding_x))
                .flex()
                .items_center()
                .rounded(px(d.menu_item_radius))
                .bg(c.dialog_surface)
                .text_size(px(d.menu_text_size))
                .font_weight(t.dialog_body_weight.to_font_weight())
                .text_color(if danger {
                    c.dialog_danger_button_bg
                } else {
                    c.dialog_secondary_button_text
                })
                .child(label)
                .hover(|this| {
                    this.bg(c.dialog_primary_button_bg)
                        .text_color(c.dialog_primary_button_text)
                })
                .cursor_pointer()
                .on_click(cx.listener(on_click))
                .into_any_element()
        } else {
            div()
                .id(id)
                .h(px(d.menu_item_height))
                .px(px(d.menu_item_padding_x))
                .flex()
                .items_center()
                .rounded(px(d.menu_item_radius))
                .bg(c.dialog_surface)
                .text_size(px(d.menu_text_size))
                .font_weight(t.dialog_body_weight.to_font_weight())
                .text_color(if danger {
                    c.dialog_danger_button_bg
                } else {
                    c.dialog_muted
                })
                .child(label)
                .into_any_element()
        }
    }

    fn render_table_menu_action(
        theme: &Theme,
        action: TableMenuAction,
        label: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (id, danger, handler) = match action {
            TableMenuAction::InsertRowAbove => (
                "table-menu-insert-row-above",
                false,
                Self::on_insert_table_row_above as _,
            ),
            TableMenuAction::InsertRowBelow => (
                "table-menu-insert-row-below",
                false,
                Self::on_insert_table_row_below as _,
            ),
            TableMenuAction::InsertColumnLeft => (
                "table-menu-insert-column-left",
                false,
                Self::on_insert_table_column_left as _,
            ),
            TableMenuAction::InsertColumnRight => (
                "table-menu-insert-column-right",
                false,
                Self::on_insert_table_column_right as _,
            ),
            TableMenuAction::AlignColumnLeft => (
                "table-menu-align-column-left",
                false,
                Self::on_align_table_column_left as _,
            ),
            TableMenuAction::AlignColumnCenter => (
                "table-menu-align-column-center",
                false,
                Self::on_align_table_column_center as _,
            ),
            TableMenuAction::AlignColumnRight => (
                "table-menu-align-column-right",
                false,
                Self::on_align_table_column_right as _,
            ),
            TableMenuAction::MoveTableRowUp => (
                "table-menu-move-row-up",
                false,
                Self::on_move_table_row_up as _,
            ),
            TableMenuAction::MoveTableRowDown => (
                "table-menu-move-row-down",
                false,
                Self::on_move_table_row_down as _,
            ),
            TableMenuAction::MoveTableColumnLeft => (
                "table-menu-move-column-left",
                false,
                Self::on_move_table_column_left as _,
            ),
            TableMenuAction::MoveTableColumnRight => (
                "table-menu-move-column-right",
                false,
                Self::on_move_table_column_right as _,
            ),
            TableMenuAction::DeleteRow => (
                "table-menu-delete-row",
                false,
                Self::on_delete_table_menu_row as _,
            ),
            TableMenuAction::DeleteColumn => (
                "table-menu-delete-column",
                false,
                Self::on_delete_table_menu_column as _,
            ),
            TableMenuAction::CopyTable => {
                ("table-menu-copy-table", false, Self::on_copy_table as _)
            }
            TableMenuAction::FormatTableSource => (
                "table-menu-format-source",
                false,
                Self::on_format_table_source as _,
            ),
            TableMenuAction::DeleteTable => {
                ("table-menu-delete-table", true, Self::on_delete_table as _)
            }
        };
        Self::render_table_menu_item(theme, id, label, true, danger, handler, cx)
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

    fn render_menu_separator(theme: &Theme) -> AnyElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        div()
            .mx(px(d.menu_separator_margin_x))
            .my(px(d.menu_separator_margin_y))
            .h(px(d.menu_separator_height))
            .bg(c.dialog_border)
            .into_any_element()
    }

    fn context_menu_entries(&self, menu: &ContextMenuState) -> Vec<ContextMenuEntry> {
        let mut entries = vec![
            ContextMenuEntry::Action(ContextMenuAction::Undo),
            ContextMenuEntry::Action(ContextMenuAction::Redo),
            ContextMenuEntry::Separator,
            ContextMenuEntry::Action(ContextMenuAction::Cut),
            ContextMenuEntry::Action(ContextMenuAction::Copy),
            ContextMenuEntry::Action(ContextMenuAction::Paste),
            ContextMenuEntry::Action(ContextMenuAction::SelectAll),
        ];

        if menu.block_target.is_some() {
            entries.push(ContextMenuEntry::Separator);
            entries.push(ContextMenuEntry::Submenu(ContextSubmenu::Format));
            if self.view_mode == ViewMode::Rendered {
                entries.extend([
                    ContextMenuEntry::Submenu(ContextSubmenu::BlockType),
                    ContextMenuEntry::Submenu(ContextSubmenu::Block),
                ]);
            }
        }

        if self.view_mode == ViewMode::Rendered {
            if menu.insert_target.is_some() {
                if !matches!(entries.last(), Some(ContextMenuEntry::Separator)) {
                    entries.push(ContextMenuEntry::Separator);
                }
                entries.push(ContextMenuEntry::Submenu(ContextSubmenu::Insert));
            }
            if menu.table_target.is_some() {
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
        entries
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

    fn context_menu_action_id(action: ContextMenuAction) -> &'static str {
        match action {
            ContextMenuAction::Undo => "editor-context-menu-undo",
            ContextMenuAction::Redo => "editor-context-menu-redo",
            ContextMenuAction::Cut => "editor-context-menu-cut",
            ContextMenuAction::Copy => "editor-context-menu-copy",
            ContextMenuAction::Paste => "editor-context-menu-paste",
            ContextMenuAction::SelectAll => "editor-context-menu-select-all",
            ContextMenuAction::Bold => "editor-context-menu-bold",
            ContextMenuAction::Italic => "editor-context-menu-italic",
            ContextMenuAction::Underline => "editor-context-menu-underline",
            ContextMenuAction::Strikethrough => "editor-context-menu-strikethrough",
            ContextMenuAction::InlineCode => "editor-context-menu-inline-code",
            ContextMenuAction::Paragraph => "editor-context-menu-paragraph",
            ContextMenuAction::Heading1 => "editor-context-menu-heading-1",
            ContextMenuAction::Heading2 => "editor-context-menu-heading-2",
            ContextMenuAction::Heading3 => "editor-context-menu-heading-3",
            ContextMenuAction::Heading4 => "editor-context-menu-heading-4",
            ContextMenuAction::Heading5 => "editor-context-menu-heading-5",
            ContextMenuAction::Heading6 => "editor-context-menu-heading-6",
            ContextMenuAction::BulletList => "editor-context-menu-bullet-list",
            ContextMenuAction::OrderedList => "editor-context-menu-ordered-list",
            ContextMenuAction::TaskList => "editor-context-menu-task-list",
            ContextMenuAction::Quote => "editor-context-menu-quote",
            ContextMenuAction::CodeBlock => "editor-context-menu-code-block",
            ContextMenuAction::MoveBlockUp => "editor-context-menu-move-block-up",
            ContextMenuAction::MoveBlockDown => "editor-context-menu-move-block-down",
            ContextMenuAction::DuplicateBlock => "editor-context-menu-duplicate-block",
            ContextMenuAction::DeleteBlock => "editor-context-menu-delete-block",
            ContextMenuAction::IndentBlock => "editor-context-menu-indent-block",
            ContextMenuAction::OutdentBlock => "editor-context-menu-outdent-block",
            ContextMenuAction::InsertTable => "editor-context-menu-insert-table",
            ContextMenuAction::ToggleViewMode => "editor-context-menu-toggle-view-mode",
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

    fn context_submenu_id(submenu: ContextSubmenu) -> &'static str {
        match submenu {
            ContextSubmenu::Format => "editor-context-menu-format",
            ContextSubmenu::BlockType => "editor-context-menu-block-type",
            ContextSubmenu::Block => "editor-context-menu-block",
            ContextSubmenu::Insert => "editor-context-menu-insert",
            ContextSubmenu::Table => "editor-context-menu-table",
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

    fn menu_entries_height(entries: &[ContextMenuEntry], theme: &Theme) -> f32 {
        let d = &theme.dimensions;
        let entries_height: f32 = entries
            .iter()
            .map(|entry| match entry {
                ContextMenuEntry::Action(_) | ContextMenuEntry::Submenu(_) => d.menu_item_height,
                ContextMenuEntry::Separator => {
                    d.menu_separator_height + d.menu_separator_margin_y * 2.0
                }
            })
            .sum();
        d.menu_panel_padding * 2.0
            + entries_height
            + d.menu_panel_gap * entries.len().saturating_sub(1) as f32
    }

    fn table_menu_height(theme: &Theme) -> f32 {
        let d = &theme.dimensions;
        let entries_height: f32 = table_menu_entries()
            .iter()
            .map(|entry| match entry {
                TableMenuEntry::Action(_) => d.menu_item_height,
                TableMenuEntry::Separator => {
                    d.menu_separator_height + d.menu_separator_margin_y * 2.0
                }
            })
            .sum();
        d.menu_panel_padding * 2.0
            + entries_height
            + d.menu_panel_gap * table_menu_entries().len().saturating_sub(1) as f32
    }

    fn context_submenu_offset(
        entries: &[ContextMenuEntry],
        submenu: ContextSubmenu,
        theme: &Theme,
    ) -> f32 {
        let d = &theme.dimensions;
        let mut offset = d.menu_panel_padding;
        for entry in entries {
            if matches!(entry, ContextMenuEntry::Submenu(candidate) if *candidate == submenu) {
                break;
            }
            offset += match entry {
                ContextMenuEntry::Action(_) | ContextMenuEntry::Submenu(_) => d.menu_item_height,
                ContextMenuEntry::Separator => {
                    d.menu_separator_height + d.menu_separator_margin_y * 2.0
                }
            };
            offset += d.menu_panel_gap;
        }
        offset
    }

    fn render_context_menu_action(
        theme: &Theme,
        action: ContextMenuAction,
        label: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        let danger = matches!(action, ContextMenuAction::DeleteBlock);
        div()
            .id(Self::context_menu_action_id(action))
            .h(px(d.menu_item_height))
            .px(px(d.menu_item_padding_x))
            .flex()
            .items_center()
            .rounded(px(d.menu_item_radius))
            .bg(c.dialog_surface)
            .text_size(px(d.menu_text_size))
            .font_weight(t.dialog_body_weight.to_font_weight())
            .text_color(if danger {
                c.dialog_danger_button_bg
            } else {
                c.dialog_secondary_button_text
            })
            .child(label)
            .hover(|this| {
                this.bg(c.dialog_primary_button_bg)
                    .text_color(c.dialog_primary_button_text)
            })
            .active(|this| this.opacity(0.92))
            .cursor_pointer()
            .on_click(cx.listener(move |editor, _event, window, cx| {
                editor.apply_context_menu_action(action, window, cx);
            }))
            .into_any_element()
    }

    fn render_context_submenu_trigger(
        theme: &Theme,
        submenu: ContextSubmenu,
        label: String,
        open: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let c = &theme.colors;
        let d = &theme.dimensions;
        let t = &theme.typography;
        div()
            .id(Self::context_submenu_id(submenu))
            .h(px(d.menu_item_height))
            .px(px(d.menu_item_padding_x))
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(d.menu_item_radius))
            .bg(if open {
                c.dialog_primary_button_bg
            } else {
                c.dialog_surface
            })
            .text_size(px(d.menu_text_size))
            .font_weight(t.dialog_body_weight.to_font_weight())
            .text_color(if open {
                c.dialog_primary_button_text
            } else {
                c.dialog_secondary_button_text
            })
            .child(label)
            .child("›")
            .hover(|this| {
                this.bg(c.dialog_primary_button_bg)
                    .text_color(c.dialog_primary_button_text)
            })
            .cursor_pointer()
            .on_hover(cx.listener(move |editor, hovered, _window, cx| {
                editor.set_context_menu_hover_state(*hovered, Some(submenu), cx);
            }))
            .into_any_element()
    }

    fn render_context_menu_entries(
        theme: &Theme,
        entries: &[ContextMenuEntry],
        open_submenu: Option<ContextSubmenu>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        entries
            .iter()
            .map(|entry| match entry {
                ContextMenuEntry::Action(action) => Self::render_context_menu_action(
                    theme,
                    *action,
                    Self::context_menu_action_label(*action),
                    cx,
                ),
                ContextMenuEntry::Submenu(submenu) => Self::render_context_submenu_trigger(
                    theme,
                    *submenu,
                    Self::context_submenu_label(*submenu),
                    open_submenu == Some(*submenu),
                    cx,
                ),
                ContextMenuEntry::Separator => Self::render_menu_separator(theme),
            })
            .collect()
    }

    pub(super) fn render_context_menu_overlay(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.context_menu.as_ref()?;
        let c = &theme.colors;
        let d = &theme.dimensions;
        let entries = self.context_menu_entries(menu);
        let panel_width = d.menu_panel_width.max(d.context_menu_axis_panel_width);
        let panel_height = Self::menu_entries_height(&entries, theme);
        let open_submenu = menu.open_submenu;
        let submenu_width = match open_submenu {
            Some(ContextSubmenu::Table) => d.menu_panel_width.max(d.context_menu_axis_panel_width),
            Some(_) => d.menu_panel_width.max(d.context_menu_submenu_width),
            None => d.context_menu_submenu_width,
        };
        let submenu_height = match open_submenu {
            Some(ContextSubmenu::Table) => Self::table_menu_height(theme),
            Some(submenu) => {
                Self::menu_entries_height(&Self::context_submenu_entries(submenu), theme)
            }
            None => 0.0,
        };
        let submenu_anchor_offset = open_submenu
            .map(|submenu| Self::context_submenu_offset(&entries, submenu, theme))
            .unwrap_or(0.0);
        let placement = place_context_menu(
            f32::from(menu.position.x),
            f32::from(menu.position.y),
            f32::from(self.root_bounds.size.width),
            f32::from(self.root_bounds.size.height),
            panel_width,
            panel_height,
            submenu_width,
            submenu_height,
            submenu_anchor_offset,
            CONTEXT_MENU_VIEWPORT_MARGIN,
            d.context_menu_submenu_gap,
        );
        let main_items = Self::render_context_menu_entries(theme, &entries, open_submenu, cx);
        let panel_id = if menu.table_target.is_some() {
            "editor-table-context-menu-panel"
        } else {
            "editor-context-menu-panel"
        };
        let overlay_id = if menu.table_target.is_some() {
            "editor-table-context-menu-overlay"
        } else {
            "editor-context-menu-overlay"
        };

        let panel = div()
            .id(panel_id)
            .debug_selector(move || panel_id.to_string())
            .absolute()
            .left(px(placement.panel_x))
            .top(px(placement.panel_y))
            .w(px(panel_width))
            .p(px(d.menu_panel_padding))
            .flex()
            .flex_col()
            .gap(px(d.menu_panel_gap))
            .bg(c.dialog_surface)
            .border(px(d.dialog_border_width))
            .border_color(c.dialog_border)
            .rounded(px(d.menu_panel_radius))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation()
            })
            .on_mouse_down(MouseButton::Right, |_event, _window, cx| {
                cx.stop_propagation()
            })
            .children(main_items);

        let overlay = div()
            .id(overlay_id)
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(Self::on_dismiss_context_menu_overlay),
            )
            .child(panel);

        let Some(open_submenu) = open_submenu else {
            return Some(overlay.into_any_element());
        };

        let submenu_items: Vec<AnyElement> = if open_submenu == ContextSubmenu::Table {
            table_menu_entries()
                .iter()
                .map(|entry| match entry {
                    TableMenuEntry::Action(action) => Self::render_table_menu_action(
                        theme,
                        *action,
                        Self::table_menu_label(*action),
                        cx,
                    ),
                    TableMenuEntry::Separator => Self::render_menu_separator(theme),
                })
                .collect()
        } else {
            Self::render_context_menu_entries(
                theme,
                &Self::context_submenu_entries(open_submenu),
                None,
                cx,
            )
        };
        let submenu_id = if open_submenu == ContextSubmenu::Table {
            "editor-table-context-menu-submenu"
        } else {
            "editor-context-menu-submenu"
        };
        let submenu = div()
            .id(submenu_id)
            .absolute()
            .left(px(placement.submenu_x))
            .top(px(placement.submenu_y))
            .w(px(submenu_width))
            .p(px(d.menu_panel_padding))
            .flex()
            .flex_col()
            .gap(px(d.menu_panel_gap))
            .occlude()
            .bg(c.dialog_surface)
            .border(px(d.dialog_border_width))
            .border_color(c.dialog_border)
            .rounded(px(d.menu_panel_radius))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                cx.stop_propagation()
            })
            .on_mouse_down(MouseButton::Right, |_event, _window, cx| {
                cx.stop_propagation()
            })
            .on_hover(cx.listener(Self::on_context_menu_submenu_hover))
            .children(submenu_items);

        Some(overlay.child(submenu).into_any_element())
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
    use super::{ContextSubmenu, Editor, TableInsertTarget, TableMenuTarget};
    use crate::components::BlockKind;
    use gpui::{AppContext, Point, TestAppContext, point, px};

    #[gpui::test]
    async fn context_menu_position_is_relative_to_embedded_editor_root(cx: &mut TestAppContext) {
        let editor = cx.new(|cx| Editor::from_markdown(cx, "alpha".to_string(), None));

        editor.update(cx, |editor, _cx| {
            editor.root_bounds.origin = point(px(120.0), px(80.0));
            assert_eq!(
                editor.context_menu_position_in_editor(point(px(156.0), px(104.0))),
                point(px(36.0), px(24.0)),
            );
        });
    }

    #[gpui::test]
    async fn context_submenu_stays_open_while_crossing_hover_gap(cx: &mut TestAppContext) {
        let editor = cx.new(|cx| Editor::from_markdown(cx, "alpha".to_string(), None));

        editor.update(cx, |editor, cx| {
            editor.open_context_menu(
                Point {
                    x: px(24.0),
                    y: px(24.0),
                },
                None,
                Some(TableInsertTarget::Append),
                None,
                cx,
            );

            editor.set_context_menu_hover_state(true, Some(ContextSubmenu::Insert), cx);
            let menu = editor
                .context_menu
                .as_ref()
                .expect("expected insert context menu");
            assert_eq!(menu.open_submenu, Some(ContextSubmenu::Insert));
            assert!(editor.context_menu_submenu_close_task.is_none());

            editor.set_context_menu_hover_state(false, Some(ContextSubmenu::Insert), cx);
            let menu = editor
                .context_menu
                .as_ref()
                .expect("expected insert context menu");
            assert_eq!(menu.open_submenu, Some(ContextSubmenu::Insert));
            assert!(editor.context_menu_submenu_close_task.is_some());

            editor.set_context_menu_hover_state(true, None, cx);
            let menu = editor
                .context_menu
                .as_ref()
                .expect("expected insert context menu");
            assert_eq!(menu.open_submenu, Some(ContextSubmenu::Insert));
            assert!(editor.context_menu_submenu_close_task.is_none());
        });
    }

    #[gpui::test]
    async fn table_submenu_stays_open_while_crossing_hover_gap(cx: &mut TestAppContext) {
        let editor = cx.new(|cx| {
            Editor::from_markdown(cx, "| A | B |\n| --- | --- |\n| 1 | 2 |".to_string(), None)
        });
        let table_block_id = editor.read_with(cx, |editor, cx| {
            editor
                .document
                .visible_blocks()
                .into_iter()
                .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
                .expect("the markdown table becomes a table block")
                .entity
                .entity_id()
        });

        editor.update(cx, |editor, cx| {
            editor.open_table_context_menu(
                point(px(24.0), px(24.0)),
                None,
                TableMenuTarget {
                    table_block_id,
                    row: 0,
                    column: 0,
                },
                cx,
            );

            editor.set_context_menu_hover_state(true, Some(ContextSubmenu::Table), cx);
            let menu = editor
                .context_menu
                .as_ref()
                .expect("expected table context menu");
            assert_eq!(menu.open_submenu, Some(ContextSubmenu::Table));
            assert!(editor.context_menu_submenu_close_task.is_none());

            editor.set_context_menu_hover_state(false, Some(ContextSubmenu::Table), cx);
            let menu = editor
                .context_menu
                .as_ref()
                .expect("expected table context menu");
            assert_eq!(menu.open_submenu, Some(ContextSubmenu::Table));
            assert!(editor.context_menu_submenu_close_task.is_some());

            editor.set_context_menu_hover_state(true, None, cx);
            let menu = editor
                .context_menu
                .as_ref()
                .expect("expected table context menu");
            assert_eq!(menu.open_submenu, Some(ContextSubmenu::Table));
            assert!(editor.context_menu_submenu_close_task.is_none());
        });
    }
}
