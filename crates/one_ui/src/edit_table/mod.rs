mod column;
mod delegate;
pub mod filter_panel;
mod filter_state;
pub(crate) mod loading;
pub mod selection;
mod state;

use std::collections::HashSet;

use gpui::{Action, App, KeyBinding, Keystroke, NoAction};
use gpui_component::Size;

pub(crate) use column::{ColGroup, DragColumn, DragSelectCell, ResizeColumn};
pub use column::{Column, ColumnFixed, ColumnSort};
pub use delegate::{CellEditor, EditTableDelegate};
pub use filter_panel::{FilterValue, FilterValueKey};
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
pub fn init(cx: &mut App, keybindings: &TableKeybindings) {
    cx.bind_keys(init_keybindings(keybindings));
}

pub fn refresh_keybindings(cx: &mut App, keybindings: TableKeybindings) {
    cx.bind_keys(refreshable_keybindings(cx, &keybindings));
}

#[derive(Clone, Debug)]
pub struct TableKeybindings {
    cancel: Vec<String>,
    copy: Vec<String>,
    paste: Vec<String>,
    select_all: Vec<String>,
}

impl TableKeybindings {
    pub fn new(
        cancel: Vec<String>,
        copy: Vec<String>,
        paste: Vec<String>,
        select_all: Vec<String>,
    ) -> Self {
        Self {
            cancel,
            copy,
            paste,
            select_all,
        }
    }
}

impl Default for TableKeybindings {
    fn default() -> Self {
        Self::new(
            vec!["escape".to_string()],
            vec![table_platform_shortcut("cmd-c", "ctrl-c").to_string()],
            vec![table_platform_shortcut("cmd-v", "ctrl-v").to_string()],
            vec![table_platform_shortcut("cmd-a", "ctrl-a").to_string()],
        )
    }
}

fn init_keybindings(bindings: &TableKeybindings) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    keybindings.extend(
        bindings
            .cancel
            .iter()
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
        bindings
            .copy
            .iter()
            .map(|key| KeyBinding::new(&key, Copy, Some(CONTEXT))),
    );
    keybindings.extend(
        bindings
            .paste
            .iter()
            .map(|key| KeyBinding::new(&key, Paste, Some(CONTEXT))),
    );
    keybindings.extend(
        bindings
            .select_all
            .iter()
            .map(|key| KeyBinding::new(&key, SelectAll, Some(CONTEXT))),
    );
    keybindings.extend([
        KeyBinding::new("tab", SelectNextColumn, Some(CONTEXT)),
        KeyBinding::new("shift-tab", SelectPrevColumn, Some(CONTEXT)),
    ]);
    keybindings
}

fn refreshable_keybindings(cx: &App, bindings: &TableKeybindings) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    keybindings.extend(rebind_keybindings(
        cx,
        &["escape"],
        &bindings.cancel,
        Some(CONTEXT),
        Cancel,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        &[table_platform_shortcut("cmd-c", "ctrl-c")],
        &bindings.copy,
        Some(CONTEXT),
        Copy,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        &[table_platform_shortcut("cmd-v", "ctrl-v")],
        &bindings.paste,
        Some(CONTEXT),
        Paste,
    ));
    keybindings.extend(rebind_keybindings(
        cx,
        &[table_platform_shortcut("cmd-a", "ctrl-a")],
        &bindings.select_all,
        Some(CONTEXT),
        SelectAll,
    ));
    keybindings
}

fn rebind_keybindings<A>(
    cx: &App,
    defaults: &[&str],
    current: &[String],
    context: Option<&str>,
    action: A,
) -> Vec<KeyBinding>
where
    A: Action + Clone,
{
    let active = cx
        .key_bindings()
        .borrow()
        .bindings_for_action(&action)
        .map(binding_shortcut)
        .collect::<Vec<_>>();
    let mut keybindings = shadow_shortcuts(defaults, current, active)
        .into_iter()
        .map(|key| KeyBinding::new(&key, NoAction, context))
        .collect::<Vec<_>>();
    keybindings.extend(
        current
            .iter()
            .map(|key| KeyBinding::new(key, action.clone(), context)),
    );
    keybindings
}

fn shadow_shortcuts(
    defaults: &[&str],
    current: &[String],
    active: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    defaults
        .iter()
        .map(|key| key.to_string())
        .chain(active)
        .chain(current.iter().cloned())
        .filter(|key| Keystroke::parse(key).is_ok() && seen.insert(key.clone()))
        .collect()
}

fn binding_shortcut(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
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
