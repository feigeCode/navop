use super::MarkdownEditor;
use crate::MarkdownEditorError;
use gpui::{Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    popover::Popover,
    v_flex,
};
use markdown_source::{TableAlignment, TableCellAddress, TableInsertPosition};

impl MarkdownEditor {
    pub(super) fn render_table_toolbar(
        &self,
        address: TableCellAddress,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let editor = cx.entity();
        let controls = h_flex()
            .gap_1()
            .child(self.render_table_grid_picker(address, cx))
            .children(row_buttons(&editor))
            .children(column_buttons(&editor))
            .children(alignment_buttons(&editor));
        let toolbar = h_flex()
            .id(("markdown-table-toolbar", address.block_id.0))
            .debug_selector(|| format!("markdown-table-toolbar-{}", address.block_id.0))
            .w_full()
            .justify_between()
            .p_1()
            .border_b_1()
            .border_color(self.theme.border)
            .bg(self.theme.background)
            .child(controls)
            .child(delete_table_button(editor));
        gpui::div()
            .absolute()
            .left_0()
            .right_0()
            .top_0()
            .child(toolbar)
            .into_any_element()
    }

    fn render_table_grid_picker(
        &self,
        address: TableCellAddress,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let editor = cx.entity();
        let border = self.theme.border;
        Popover::new(("markdown-table-grid", address.block_id.0))
            .trigger(
                Button::new(("markdown-table-grid-trigger", address.block_id.0))
                    .label("▦")
                    .ghost()
                    .xsmall()
                    .tooltip("Resize table"),
            )
            .content(move |_state, _window, cx| {
                let popover = cx.entity();
                v_flex().gap_1().children((1..=6).map(|rows| {
                    h_flex().gap_1().children((1..=6).map(|columns| {
                        grid_cell(editor.clone(), popover.clone(), border, rows, columns)
                    }))
                }))
            })
            .into_any_element()
    }
}

fn row_buttons(editor: &gpui::Entity<MarkdownEditor>) -> Vec<Button> {
    vec![
        action_button("table-row-before", "↑R", "Insert row above").on_click(table_listener(
            editor.clone(),
            move |editor, window, cx| {
                editor.insert_active_table_row(TableInsertPosition::Before, window, cx)
            },
        )),
        action_button("table-row-after", "↓R", "Insert row below").on_click(table_listener(
            editor.clone(),
            move |editor, window, cx| {
                editor.insert_active_table_row(TableInsertPosition::After, window, cx)
            },
        )),
        action_button("table-delete-row", "−R", "Delete row").on_click(table_listener(
            editor.clone(),
            MarkdownEditor::delete_active_table_row,
        )),
    ]
}

fn column_buttons(editor: &gpui::Entity<MarkdownEditor>) -> Vec<Button> {
    vec![
        action_button("table-column-before", "←C", "Insert column left").on_click(table_listener(
            editor.clone(),
            move |editor, window, cx| {
                editor.insert_active_table_column(TableInsertPosition::Before, window, cx)
            },
        )),
        action_button("table-column-after", "→C", "Insert column right").on_click(table_listener(
            editor.clone(),
            move |editor, window, cx| {
                editor.insert_active_table_column(TableInsertPosition::After, window, cx)
            },
        )),
        action_button("table-delete-column", "−C", "Delete column").on_click(table_listener(
            editor.clone(),
            MarkdownEditor::delete_active_table_column,
        )),
    ]
}

fn alignment_buttons(editor: &gpui::Entity<MarkdownEditor>) -> Vec<Button> {
    [
        ("table-align-left", "左", TableAlignment::Left),
        ("table-align-center", "中", TableAlignment::Center),
        ("table-align-right", "右", TableAlignment::Right),
    ]
    .into_iter()
    .map(|(id, label, alignment)| {
        action_button(id, label, "Align active column")
            .on_click(table_listener(editor.clone(), move |editor, window, cx| {
                editor.align_active_table_column(alignment, window, cx)
            }))
    })
    .collect()
}

fn delete_table_button(editor: gpui::Entity<MarkdownEditor>) -> Button {
    action_button("table-delete", "⌫", "Delete table").on_click(move |_, window, cx| {
        editor.update(cx, |editor, cx| {
            let _ = editor.delete_active_block(window, cx);
        });
    })
}

fn grid_cell(
    editor: gpui::Entity<MarkdownEditor>,
    popover: gpui::Entity<gpui_component::popover::PopoverState>,
    border: gpui::Hsla,
    rows: usize,
    columns: usize,
) -> Button {
    Button::new(SharedString::from(format!(
        "markdown-table-size-{rows}-{columns}"
    )))
    .label("")
    .xsmall()
    .ghost()
    .w(gpui::px(24.))
    .h(gpui::px(24.))
    .border_1()
    .border_color(border)
    .tooltip(format!("{columns} × {rows}"))
    .on_click(move |_, window, cx| {
        editor.update(cx, |editor, cx| {
            let _ = editor.resize_active_table(rows, columns, window, cx);
        });
        popover.update(cx, |popover, cx| popover.dismiss(window, cx));
    })
}

fn action_button(id: &'static str, label: &'static str, tooltip: &'static str) -> Button {
    Button::new(id)
        .label(label)
        .ghost()
        .xsmall()
        .tooltip(tooltip)
}

fn table_listener(
    editor: gpui::Entity<MarkdownEditor>,
    operation: impl Fn(
        &mut MarkdownEditor,
        &mut gpui::Window,
        &mut gpui::Context<MarkdownEditor>,
    ) -> Result<bool, MarkdownEditorError>
    + 'static,
) -> impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static {
    move |_, window, cx| {
        editor.update(cx, |editor, cx| {
            let _ = operation(editor, window, cx);
        });
    }
}
