rust_i18n::i18n!("locales", fallback = "en");

pub mod content_state;
pub mod edit_table;
pub mod file_conflict_prompt;
mod geometry;
pub mod icon_button;
mod icon_size;
pub mod large_text_editor;
pub mod marquee_text;
pub mod panel_header;
pub mod resize_handle;
mod settings;
pub mod signature_help;
pub mod status_bar;
mod time;

pub use content_state::{ContentState, ContentStateKind};
pub use edit_table::{
    CellCoord, CellEditor, CellRange, Column, ColumnFixed, ColumnSort, EditTable,
    EditTableDelegate, EditTableEvent, EditTableState, FilterState, FilterValue, ScrollbarVisible,
    SelectNextColumn, SelectPrevColumn, TableKeybindings, TableOptions, TableSelection,
    TableVisibleRange, refresh_keybindings,
};
use gpui::App;
pub use icon_button::{IconButton, IconButtonRole};
pub use icon_size::IconSize;
pub use large_text_editor::{
    LargeTextEditor, LargeTextEditorEvent, LargeTextEditorTab,
    create_large_text_editor_with_content, large_text_values_equivalent,
};
pub use panel_header::{PanelHeader, PanelHeaderVariant};
pub use settings::{
    TableDisplaySettings, init_table_display_settings, set_table_row_height, table_row_height,
    table_row_height_or,
};
pub use signature_help::{ExtendedEditor, ExtendedEditorState, SignatureHelpProvider};
pub use status_bar::{StatusBar, StatusPresentation};
pub use time::datetime_picker::{DateTimePicker, DateTimePickerEvent, DateTimePickerState};
pub use time::time_picker::{TimePicker, TimePickerEvent, TimePickerState};

pub fn init(cx: &mut App, keybindings: TableKeybindings) {
    edit_table::init(cx, &keybindings);
    time::init(cx);
}
