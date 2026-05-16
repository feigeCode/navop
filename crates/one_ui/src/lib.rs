pub mod edit_table;
pub mod large_text_editor;
pub mod resize_handle;
mod time;

pub use edit_table::{
    CellCoord, CellEditor, CellRange, Column, ColumnFixed, ColumnSort, EditTable,
    EditTableDelegate, EditTableEvent, EditTableState, FilterState, FilterValue, ScrollbarVisible,
    SelectNextColumn, SelectPrevColumn, TableOptions, TableSelection, TableVisibleRange,
};
use gpui::App;
pub use large_text_editor::{
    LargeTextEditor, LargeTextEditorEvent, LargeTextEditorTab,
    create_large_text_editor_with_content, large_text_values_equivalent,
};

pub fn init(cx: &mut App) {
    edit_table::init(cx);
}
