pub mod edit_table;
pub mod large_text_editor;
pub mod resize_handle;
mod settings;
mod time;

pub use edit_table::{
    CellCoord, CellEditor, CellRange, Column, ColumnFixed, ColumnSort, EditTable,
    EditTableDelegate, EditTableEvent, EditTableState, FilterState, FilterValue, ScrollbarVisible,
    SelectNextColumn, SelectPrevColumn, TableOptions, TableSelection, TableVisibleRange,
    refresh_keybindings,
};
use gpui::App;
pub use large_text_editor::{
    LargeTextEditor, LargeTextEditorEvent, LargeTextEditorTab,
    create_large_text_editor_with_content, large_text_values_equivalent,
};
pub use settings::{
    TableDisplaySettings, init_table_display_settings, set_table_row_height, table_row_height,
    table_row_height_or,
};

pub fn init(cx: &mut App) {
    init_table_display_settings(cx, TableDisplaySettings::default());
    edit_table::init(cx);
}
