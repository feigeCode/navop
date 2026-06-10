mod column;
mod delegate;
pub mod filter_panel;
mod filter_state;
pub(crate) mod loading;
pub mod selection;
mod state;

use gpui::{App, KeyBinding};
use gpui_component::Size;
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};

pub(crate) use column::{ColGroup, DragColumn, DragSelectCell, ResizeColumn};
pub use column::{Column, ColumnFixed, ColumnSort};
pub use delegate::{CellEditor, EditTableDelegate};
pub use filter_panel::FilterValue;
pub use filter_state::FilterState;
pub use selection::{CellCoord, CellRange, TableSelection};
use state::{
    Cancel, Copy, Paste, SelectAll, SelectDown, SelectFirst, SelectLast, SelectPageDown,
    SelectPageUp, SelectUp,
};
pub use state::{EditTableEvent, EditTableState, TableVisibleRange};

const CONTEXT: &str = "EditTable";

gpui::actions!(edit_table, [SelectPrevColumn, SelectNextColumn]);

/// 初始化 EditTable 的键盘绑定
pub fn init(cx: &mut App) {
    cx.bind_keys(init_keybindings(cx));
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
}

fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    keybindings.extend(
        shortcuts_for(cx, action_id::TABLE_CANCEL, &["escape"])
            .into_iter()
            .map(|key| KeyBinding::new(&key, Cancel, Some(CONTEXT))),
    );
    keybindings.extend([
        KeyBinding::new("up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("left", SelectPrevColumn, Some(CONTEXT)),
        KeyBinding::new("right", SelectNextColumn, Some(CONTEXT)),
        KeyBinding::new("home", SelectFirst, Some(CONTEXT)),
        KeyBinding::new("end", SelectLast, Some(CONTEXT)),
        KeyBinding::new("pageup", SelectPageUp, Some(CONTEXT)),
        KeyBinding::new("pagedown", SelectPageDown, Some(CONTEXT)),
    ]);
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TABLE_COPY,
            &[table_platform_shortcut("cmd-c", "ctrl-c")],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, Copy, Some(CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TABLE_PASTE,
            &[table_platform_shortcut("cmd-v", "ctrl-v")],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, Paste, Some(CONTEXT))),
    );
    keybindings.extend(
        shortcuts_for(
            cx,
            action_id::TABLE_SELECT_ALL,
            &[table_platform_shortcut("cmd-a", "ctrl-a")],
        )
        .into_iter()
        .map(|key| KeyBinding::new(&key, SelectAll, Some(CONTEXT))),
    );
    keybindings.extend([
        KeyBinding::new("tab", SelectNextColumn, Some(CONTEXT)),
        KeyBinding::new("shift-tab", SelectPrevColumn, Some(CONTEXT)),
    ]);
    keybindings
}

fn refreshable_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TABLE_CANCEL,
        &["escape"],
        Some(CONTEXT),
        Cancel,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TABLE_COPY,
        &[table_platform_shortcut("cmd-c", "ctrl-c")],
        Some(CONTEXT),
        Copy,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TABLE_PASTE,
        &[table_platform_shortcut("cmd-v", "ctrl-v")],
        Some(CONTEXT),
        Paste,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        action_id::TABLE_SELECT_ALL,
        &[table_platform_shortcut("cmd-a", "ctrl-a")],
        Some(CONTEXT),
        SelectAll,
    ));
    keybindings
}

fn table_platform_shortcut(macos: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        macos
    } else {
        other
    }
}

#[derive(Clone, Copy, Default)]
pub struct ScrollbarVisible {
    pub right: bool,
    pub bottom: bool,
}

impl ScrollbarVisible {
    pub fn all() -> Self {
        Self {
            right: true,
            bottom: true,
        }
    }

    pub fn none() -> Self {
        Self {
            right: false,
            bottom: false,
        }
    }
}

#[derive(Clone, Copy)]
pub struct TableOptions {
    pub size: Size,
    pub stripe: bool,
    pub scrollbar_visible: ScrollbarVisible,
}

impl Default for TableOptions {
    fn default() -> Self {
        Self {
            size: Size::Medium,
            stripe: true,
            scrollbar_visible: ScrollbarVisible::all(),
        }
    }
}

pub struct EditTable;

impl EditTable {
    pub fn new<D: EditTableDelegate>(
        state: &gpui::Entity<EditTableState<D>>,
    ) -> gpui::Entity<EditTableState<D>> {
        state.clone()
    }
}
