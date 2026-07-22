use super::MarkdownEditor;
use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString, Styled,
    prelude::FluentBuilder, rems,
};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    text::TextView,
    v_flex,
};
use markdown_source::{SourceBlock, SourceTableCell, SourceTableMap, TableCellAddress};

impl MarkdownEditor {
    pub(super) fn render_table(
        &self,
        block: &SourceBlock,
        table: &SourceTableMap,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let rows = table
            .rows
            .iter()
            .enumerate()
            .filter(|(row, _)| *row != 1)
            .map(|(row, source_row)| {
                h_flex()
                    .w_full()
                    .min_w_0()
                    .children(source_row.cells.iter().enumerate().map(|(column, cell)| {
                        self.render_table_cell(
                            TableCellAddress {
                                block_id: block.id,
                                row,
                                column,
                            },
                            cell,
                            row == 0,
                            cx,
                        )
                    }))
            });
        v_flex()
            .id(("markdown-table", block.id.0))
            .debug_selector(|| format!("markdown-table-{}", block.id.0))
            .w_full()
            .min_w_0()
            .my_2()
            .rounded_md()
            .border_1()
            .border_color(self.theme.border)
            .overflow_hidden()
            .children(rows)
            .into_any_element()
    }

    fn render_table_cell(
        &self,
        address: TableCellAddress,
        cell: &SourceTableCell,
        header: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let active = self.active_table_cell == Some(address);
        let editor = cx.entity();
        gpui::div()
            .id(table_cell_id(address))
            .debug_selector(|| table_cell_selector(address))
            .flex_1()
            .min_w_0()
            .min_h(rems(2.5))
            .px_3()
            .py_2()
            .border_r_1()
            .border_b_1()
            .border_color(self.theme.border)
            .when(header, |cell| cell.bg(self.theme.border.opacity(0.2)))
            .when(!active, |cell| {
                cell.cursor_text()
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        editor.update(cx, |editor, cx| {
                            editor.activate_table_cell(address, window, cx);
                        });
                    })
            })
            .child(if active {
                self.render_active_table_cell(address, cx)
            } else {
                TextView::markdown(
                    table_cell_id(address),
                    cell.original_source.trim().to_owned(),
                )
                .style(self.text_view_style())
                .into_any_element()
            })
            .into_any_element()
    }

    fn render_active_table_cell(
        &self,
        address: TableCellAddress,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let clear_editor = cx.entity();
        v_flex()
            .w_full()
            .min_w_0()
            .child(
                Input::new(&self.input)
                    .w_full()
                    .h(rems(2.25))
                    .bare()
                    .bordered(false)
                    .focus_bordered(false)
                    .local_style(self.input_style())
                    .highlight_theme(self.theme.highlight_theme.clone())
                    .caret_color(self.theme.primary),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new(SharedString::from(format!(
                        "markdown-table-clear-{}-{}-{}",
                        address.block_id.0, address.row, address.column
                    )))
                    .label("Clear")
                    .xsmall()
                    .ghost()
                    .on_click(move |_, window, cx| {
                        clear_editor.update(cx, |editor, cx| {
                            let _ = editor.clear_active_table_cell(window, cx);
                        });
                    }),
                ),
            )
            .into_any_element()
    }
}

fn table_cell_id(address: TableCellAddress) -> SharedString {
    table_cell_selector(address).into()
}

fn table_cell_selector(address: TableCellAddress) -> String {
    format!(
        "markdown-table-cell-{}-{}-{}",
        address.block_id.0, address.row, address.column
    )
}
