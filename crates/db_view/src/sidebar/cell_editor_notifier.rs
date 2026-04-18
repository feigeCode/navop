use crate::table_data::data_grid::DataGrid;
use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, WeakEntity};

#[derive(Clone, Debug)]
pub enum CellEditorSidebarEvent {
    Toggle(WeakEntity<DataGrid>),
}

pub struct CellEditorSidebarNotifier;

impl EventEmitter<CellEditorSidebarEvent> for CellEditorSidebarNotifier {}

#[derive(Clone)]
pub struct GlobalCellEditorSidebarNotifier(pub Entity<CellEditorSidebarNotifier>);

impl Global for GlobalCellEditorSidebarNotifier {}

pub fn init_cell_editor_sidebar_notifier(cx: &mut App) {
    let notifier = cx.new(|_| CellEditorSidebarNotifier);
    cx.set_global(GlobalCellEditorSidebarNotifier(notifier));
}

pub fn get_cell_editor_sidebar_notifier(cx: &App) -> Option<Entity<CellEditorSidebarNotifier>> {
    cx.try_global::<GlobalCellEditorSidebarNotifier>()
        .map(|global| global.0.clone())
}

pub fn emit_toggle_cell_editor_sidebar_event<T>(
    data_grid: WeakEntity<DataGrid>,
    cx: &mut Context<T>,
) -> bool {
    let Some(notifier) = cx.try_global::<GlobalCellEditorSidebarNotifier>().cloned() else {
        return false;
    };

    notifier.0.update(cx, |_, cx| {
        cx.emit(CellEditorSidebarEvent::Toggle(data_grid));
    });
    true
}
