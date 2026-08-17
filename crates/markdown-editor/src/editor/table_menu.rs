//! Pure contracts for the table submenu rendered by gpui-component.

use gpui::EntityId;

/// Cell-relative target used by all rendered table menu actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TableMenuTarget {
    pub table_block_id: EntityId,
    pub row: usize,
    pub column: usize,
}

/// Actions exposed by the rendered table submenu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TableMenuAction {
    InsertRowAbove,
    InsertRowBelow,
    InsertColumnLeft,
    InsertColumnRight,
    AlignColumnLeft,
    AlignColumnCenter,
    AlignColumnRight,
    MoveTableRowUp,
    MoveTableRowDown,
    MoveTableColumnLeft,
    MoveTableColumnRight,
    DeleteRow,
    DeleteColumn,
    CopyTable,
    FormatTableSource,
    DeleteTable,
}

/// Ordered entries in the rendered table submenu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TableMenuEntry {
    Action(TableMenuAction),
    Separator,
}

const TABLE_MENU_ENTRIES: [TableMenuEntry; 21] = [
    TableMenuEntry::Action(TableMenuAction::InsertRowAbove),
    TableMenuEntry::Action(TableMenuAction::InsertRowBelow),
    TableMenuEntry::Action(TableMenuAction::MoveTableRowUp),
    TableMenuEntry::Action(TableMenuAction::MoveTableRowDown),
    TableMenuEntry::Separator,
    TableMenuEntry::Action(TableMenuAction::InsertColumnLeft),
    TableMenuEntry::Action(TableMenuAction::InsertColumnRight),
    TableMenuEntry::Action(TableMenuAction::MoveTableColumnLeft),
    TableMenuEntry::Action(TableMenuAction::MoveTableColumnRight),
    TableMenuEntry::Separator,
    TableMenuEntry::Action(TableMenuAction::AlignColumnLeft),
    TableMenuEntry::Action(TableMenuAction::AlignColumnCenter),
    TableMenuEntry::Action(TableMenuAction::AlignColumnRight),
    TableMenuEntry::Separator,
    TableMenuEntry::Action(TableMenuAction::DeleteRow),
    TableMenuEntry::Action(TableMenuAction::DeleteColumn),
    TableMenuEntry::Separator,
    TableMenuEntry::Action(TableMenuAction::CopyTable),
    TableMenuEntry::Action(TableMenuAction::FormatTableSource),
    TableMenuEntry::Separator,
    TableMenuEntry::Action(TableMenuAction::DeleteTable),
];

pub(super) fn table_menu_entries() -> &'static [TableMenuEntry] {
    &TABLE_MENU_ENTRIES
}

pub(super) fn table_menu_move_delta(action: TableMenuAction) -> Option<i32> {
    match action {
        TableMenuAction::MoveTableRowUp | TableMenuAction::MoveTableColumnLeft => Some(-1),
        TableMenuAction::MoveTableRowDown | TableMenuAction::MoveTableColumnRight => Some(1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_menu_items_follow_typora_order() {
        use TableMenuAction::*;
        use TableMenuEntry::{Action, Separator};

        assert_eq!(
            table_menu_entries(),
            &[
                Action(InsertRowAbove),
                Action(InsertRowBelow),
                Action(MoveTableRowUp),
                Action(MoveTableRowDown),
                Separator,
                Action(InsertColumnLeft),
                Action(InsertColumnRight),
                Action(MoveTableColumnLeft),
                Action(MoveTableColumnRight),
                Separator,
                Action(AlignColumnLeft),
                Action(AlignColumnCenter),
                Action(AlignColumnRight),
                Separator,
                Action(DeleteRow),
                Action(DeleteColumn),
                Separator,
                Action(CopyTable),
                Action(FormatTableSource),
                Separator,
                Action(DeleteTable),
            ]
        );
    }

    #[test]
    fn table_move_actions_use_visual_index_direction() {
        assert_eq!(
            table_menu_move_delta(TableMenuAction::MoveTableRowUp),
            Some(-1)
        );
        assert_eq!(
            table_menu_move_delta(TableMenuAction::MoveTableRowDown),
            Some(1)
        );
        assert_eq!(
            table_menu_move_delta(TableMenuAction::MoveTableColumnLeft),
            Some(-1)
        );
        assert_eq!(
            table_menu_move_delta(TableMenuAction::MoveTableColumnRight),
            Some(1)
        );
        assert_eq!(table_menu_move_delta(TableMenuAction::DeleteRow), None);
    }
}
