//! Pure contracts and placement helpers for the Typora-style table menu.

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

/// Horizontal side selected for the submenu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HorizontalSide {
    Left,
    Right,
}

/// Clamped positions for a root context-menu panel and its submenu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MenuPlacement {
    pub panel_x: f32,
    pub panel_y: f32,
    pub submenu_x: f32,
    pub submenu_y: f32,
    pub submenu_side: HorizontalSide,
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

#[allow(clippy::too_many_arguments)]
pub(super) fn place_context_menu(
    requested_x: f32,
    requested_y: f32,
    editor_width: f32,
    editor_height: f32,
    panel_width: f32,
    panel_height: f32,
    submenu_width: f32,
    submenu_height: f32,
    submenu_anchor_offset: f32,
    margin: f32,
    gap: f32,
) -> MenuPlacement {
    let max_panel_x = (editor_width - margin - panel_width).max(margin);
    let max_panel_y = (editor_height - margin - panel_height).max(margin);
    let panel_x = requested_x.clamp(margin, max_panel_x);
    let panel_y = requested_y.clamp(margin, max_panel_y);

    let right_x = panel_x + panel_width + gap;
    let right_fits = right_x + submenu_width <= editor_width - margin;
    let left_x = panel_x - gap - submenu_width;
    let left_fits = left_x >= margin;
    let (submenu_x, submenu_side) = if right_fits || !left_fits {
        let max_x = (editor_width - margin - submenu_width).max(margin);
        (right_x.clamp(margin, max_x), HorizontalSide::Right)
    } else {
        (left_x, HorizontalSide::Left)
    };
    let max_submenu_y = (editor_height - margin - submenu_height).max(margin);

    MenuPlacement {
        panel_x,
        panel_y,
        submenu_x,
        submenu_y: (panel_y + submenu_anchor_offset).clamp(margin, max_submenu_y),
        submenu_side,
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

    #[test]
    fn context_panel_is_clamped_inside_editor() {
        let placement = place_context_menu(
            490.0, 390.0, 500.0, 400.0, 132.0, 36.0, 164.0, 340.0, 0.0, 8.0, 2.0,
        );

        assert_eq!(placement.panel_x, 360.0);
        assert_eq!(placement.panel_y, 356.0);
    }

    #[test]
    fn table_submenu_flips_left_near_right_edge() {
        let placement = place_context_menu(
            490.0, 40.0, 500.0, 400.0, 132.0, 36.0, 164.0, 340.0, 0.0, 8.0, 2.0,
        );

        assert_eq!(placement.submenu_side, HorizontalSide::Left);
        assert_eq!(placement.submenu_x, 194.0);
    }

    #[test]
    fn table_submenu_y_is_clamped() {
        let placement = place_context_menu(
            40.0, 390.0, 500.0, 400.0, 132.0, 36.0, 164.0, 340.0, 0.0, 8.0, 2.0,
        );

        assert_eq!(placement.submenu_y, 52.0);
    }

    #[test]
    fn table_submenu_y_tracks_the_trigger_offset_before_clamping() {
        let placement = place_context_menu(
            40.0, 40.0, 500.0, 400.0, 132.0, 36.0, 164.0, 100.0, 72.0, 8.0, 2.0,
        );

        assert_eq!(placement.submenu_y, 112.0);
    }
}
