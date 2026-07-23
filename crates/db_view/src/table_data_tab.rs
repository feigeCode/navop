use crate::table_data::cell_preview_host::CellPreviewHost;
use crate::table_data::data_grid::{DataGrid, DataGridConfig};
use futures::channel::oneshot;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Task, Window,
};
use gpui_component::button::Button;
use gpui_component::{Icon, IconName, WindowExt, button::ButtonVariants, v_flex};
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub enum TableDataTabEvent {
    OpenTableDesignerRequested,
}

pub struct TableDataTabContent {
    pub data_grid: Entity<DataGrid>,
    content: Entity<CellPreviewHost>,
    database_name: String,
    table_name: String,
    focus_handle: FocusHandle,
    _data_grid_sub: Option<Subscription>,
}

pub struct TableDataTabParams {
    pub database_name: String,
    pub schema_name: Option<String>,
    pub table_name: String,
    pub connection_id: String,
    pub database_type: one_core::storage::DatabaseType,
    pub editable: bool,
}

impl TableDataTabContent {
    pub fn new(params: TableDataTabParams, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut config = DataGridConfig::new(
            params.database_name.clone(),
            params.table_name.clone(),
            params.connection_id,
            params.database_type,
        )
        .editable(params.editable)
        .show_toolbar(true);

        if let Some(schema) = params.schema_name {
            config = config.with_schema(schema);
        }

        let data_grid = cx.new(|cx| DataGrid::new(config, window, cx));
        let content = cx.new(|cx| CellPreviewHost::new(data_grid.clone(), window, cx));
        let focus_handle = cx.focus_handle();
        let data_grid_sub = cx.subscribe_in(
            &data_grid,
            window,
            |_this, _, event: &crate::table_data::data_grid::DataGridEvent, _, cx| match event {
                crate::table_data::data_grid::DataGridEvent::OpenTableDesignerRequested => {
                    cx.emit(TableDataTabEvent::OpenTableDesignerRequested);
                }
                _ => {}
            },
        );

        Self {
            data_grid,
            content,
            database_name: params.database_name,
            table_name: params.table_name,
            focus_handle,
            _data_grid_sub: Some(data_grid_sub),
        }
    }
}

impl Render for TableDataTabContent {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(self.content.clone())
    }
}

impl Focusable for TableDataTabContent {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for TableDataTabContent {}

impl EventEmitter<TableDataTabEvent> for TableDataTabContent {}

fn table_data_tab_title(database_name: &str, table_name: &str) -> String {
    format!("{table_name} - Data ({database_name})")
}

impl TabContent for TableDataTabContent {
    fn content_key(&self) -> &'static str {
        "TableData"
    }

    fn title(&self, _cx: &App) -> SharedString {
        table_data_tab_title(&self.database_name, &self.table_name).into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::TableData.color())
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let _ = self
            .content
            .update(cx, |content, cx| content.flush_pending(cx));
        let has_changes = self.data_grid.read(cx).has_unsaved_changes(cx);
        if !has_changes {
            return Task::ready(true);
        }

        let table_name = format!("{}.{}", self.database_name, self.table_name);
        let data_grid = self.data_grid.clone();

        let (tx, rx) = oneshot::channel::<bool>();
        let tx = Arc::new(Mutex::new(Some(tx)));

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let tx_save = tx.clone();
            let tx_discard = tx.clone();
            let tx_cancel = tx.clone();
            let data_grid = data_grid.clone();

            dialog
                .title(format!("{} {}", t!("Common.close"), table_name))
                .overlay_closable(false)
                .close_button(false)
                .footer(move |_ok, _cancel, _window, _cx| {
                    let data_grid = data_grid.clone();
                    let tx_save = tx_save.clone();
                    let tx_discard = tx_discard.clone();
                    let tx_cancel = tx_cancel.clone();

                    vec![
                        Button::new("cancel")
                            .label(t!("Common.cancel"))
                            .on_click(move |_, window: &mut Window, cx| {
                                window.close_dialog(cx);
                                if let Some(tx) = tx_cancel.lock().ok().and_then(|mut g| g.take()) {
                                    let _ = tx.send(false);
                                }
                            })
                            .into_any_element(),
                        Button::new("discard")
                            .label(t!("Common.discard"))
                            .on_click(move |_, window: &mut Window, cx| {
                                window.close_dialog(cx);
                                if let Some(tx) = tx_discard.lock().ok().and_then(|mut g| g.take())
                                {
                                    let _ = tx.send(true);
                                }
                            })
                            .into_any_element(),
                        Button::new("save")
                            .label(t!("Common.save"))
                            .primary()
                            .on_click(move |_, window: &mut Window, cx| {
                                window.close_dialog(cx);
                                data_grid.update(cx, |grid, cx| {
                                    grid.save_changes(window, cx);
                                });
                                if let Some(tx) = tx_save.lock().ok().and_then(|mut g| g.take()) {
                                    let _ = tx.send(true);
                                }
                            })
                            .into_any_element(),
                    ]
                })
                .child(t!("Table.unsaved_changes_prompt").to_string())
        });

        cx.spawn(async move |_handle, _cx| rx.await.unwrap_or(false))
    }
}

impl Clone for TableDataTabContent {
    fn clone(&self) -> Self {
        Self {
            data_grid: self.data_grid.clone(),
            content: self.content.clone(),
            database_name: self.database_name.clone(),
            table_name: self.table_name.clone(),
            focus_handle: self.focus_handle.clone(),
            _data_grid_sub: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_data_tab_title_keeps_table_name_at_front() {
        assert_eq!(
            "orders - Data (analytics)",
            table_data_tab_title("analytics", "orders")
        );
    }
}
