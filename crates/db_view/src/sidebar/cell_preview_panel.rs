use crate::table_data::data_grid::{DataGrid, DataGridEvent, LargeTextCellTarget};
use crate::table_data::multi_text_editor::{
    MultiTextEditor, MultiTextEditorEvent, create_multi_text_editor_with_content,
};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled,
    Subscription, WeakEntity, Window, div,
};
use gpui_component::{ActiveTheme, StyledExt, WindowExt, h_flex, v_flex};
use rust_i18n::t;

pub struct CellPreviewPanel {
    editor: Entity<MultiTextEditor>,
    data_grid: Option<WeakEntity<DataGrid>>,
    current_target: Option<LargeTextCellTarget>,
    _grid_sub: Option<Subscription>,
    _editor_sub: Subscription,
    focus_handle: FocusHandle,
}

impl CellPreviewPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = create_multi_text_editor_with_content(None, window, cx);
        let editor_sub = cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &MultiTextEditorEvent, _window, cx| match event {
                MultiTextEditorEvent::ActiveEditorBlurred(value) => {
                    this.write_back_value(value.clone(), cx);
                }
            },
        );

        Self {
            editor,
            data_grid: None,
            current_target: None,
            _grid_sub: None,
            _editor_sub: editor_sub,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn bind_data_grid(
        &mut self,
        data_grid: WeakEntity<DataGrid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_pending(cx);

        let Some(grid) = data_grid.upgrade() else {
            self.clear_binding(window, cx);
            return;
        };

        self.data_grid = Some(data_grid);
        self._grid_sub = Some(cx.subscribe_in(
            &grid,
            window,
            |this, _, event: &DataGridEvent, window, cx| match event {
                DataGridEvent::LargeTextSelectionChanged => {
                    this.handle_selection_changed(window, cx);
                }
            },
        ));
        self.load_selected_cell(window, cx);
    }

    pub fn unbind(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_binding(window, cx);
        cx.notify();
    }

    pub fn flush_pending(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(target) = self.current_target.clone() else {
            return true;
        };
        if !target.editable {
            return true;
        }
        if !self.editor.read(cx).has_pending_writeback() {
            return true;
        }

        let Some(grid) = self.data_grid.as_ref().and_then(|grid| grid.upgrade()) else {
            return true;
        };

        let value = match self.editor.read(cx).get_writeback_text(cx) {
            Ok(value) => value,
            Err(err) => {
                self.show_error(
                    t!("TableDataGrid.error_message", error = err.to_string()).to_string(),
                    cx,
                );
                return false;
            }
        };

        if value == target.value {
            self.editor.update(cx, |editor, _| {
                editor.mark_writeback_clean();
            });
            return true;
        }

        self.apply_value_to_target(grid, target, value, cx)
    }

    fn handle_selection_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flush_pending(cx) {
            return;
        }

        self.load_selected_cell(window, cx);
    }

    fn load_selected_cell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(grid) = self.data_grid.as_ref().and_then(|grid| grid.upgrade()) else {
            self.clear_binding(window, cx);
            return;
        };

        let Some(target) = grid.read(cx).selected_large_text_target(cx) else {
            self.current_target = None;
            self.set_editor_content(String::new(), window, cx);
            cx.notify();
            return;
        };

        self.current_target = Some(target.clone());
        self.set_editor_content(target.value, window, cx);
        cx.notify();
    }

    fn write_back_value(&mut self, value: String, cx: &mut Context<Self>) -> bool {
        let Some(target) = self.current_target.clone() else {
            return true;
        };
        if !target.editable || !self.editor.read(cx).has_pending_writeback() {
            return true;
        }

        let Some(grid) = self.data_grid.as_ref().and_then(|grid| grid.upgrade()) else {
            return true;
        };

        if value == target.value {
            self.editor.update(cx, |editor, _| {
                editor.mark_writeback_clean();
            });
            return true;
        }

        self.apply_value_to_target(grid, target, value, cx)
    }

    fn apply_value_to_target(
        &mut self,
        grid: Entity<DataGrid>,
        target: LargeTextCellTarget,
        value: String,
        cx: &mut Context<Self>,
    ) -> bool {
        if grid.update(cx, |grid, cx| {
            grid.apply_large_text_target_value(&target, value.clone(), cx)
        }) {
            if let Some(current_target) = self.current_target.as_mut() {
                current_target.value = value;
            }
            self.editor.update(cx, |editor, _| {
                editor.mark_writeback_clean();
            });
            true
        } else {
            false
        }
    }

    fn clear_binding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.data_grid = None;
        self.current_target = None;
        self._grid_sub = None;
        self.set_editor_content(String::new(), window, cx);
    }

    fn set_editor_content(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            editor.load_external_text(value, window, cx);
        });
    }

    fn show_error(&self, message: String, cx: &mut Context<Self>) {
        let Some(window) = cx.active_window() else {
            return;
        };
        let _ = window.update(cx, |_, window, cx| {
            window.push_notification(message, cx);
        });
    }
}

impl Focusable for CellPreviewPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CellPreviewPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let column_label = self
            .current_target
            .as_ref()
            .map(|target| target.column_name.clone())
            .unwrap_or_else(|| t!("TableData.select_cell").to_string());
        let row_label = self
            .current_target
            .as_ref()
            .map(|target| format!("#{}", target.display_row_ix + 1))
            .unwrap_or_else(|| "--".to_string());
        let editable = self
            .current_target
            .as_ref()
            .map(|target| target.editable)
            .unwrap_or(false);

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_shrink_0()
                    .justify_between()
                    .items_center()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().font_semibold().child(column_label))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(row_label),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if editable {
                                t!("Common.edit").to_string()
                            } else {
                                t!("Common.preview").to_string()
                            }),
                    ),
            )
            .child(div().flex_1().min_h_0().child(self.editor.clone()))
    }
}
