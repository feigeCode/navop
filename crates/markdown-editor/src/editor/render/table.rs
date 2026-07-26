use super::{MARKDOWN_BODY_FONT_SIZE, MARKDOWN_BODY_LINE_HEIGHT, MarkdownEditor};
use crate::editor::surface::MarkdownSurfaceKey;
use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString, Styled,
    TextAlign, prelude::FluentBuilder, rems,
};
use gpui_component::{ElementExt, StyledExt, h_flex, input::Input, v_flex};
use markdown_source::{SourceBlock, SourceTableMap, TableCellAddress};

impl MarkdownEditor {
    pub(super) fn render_table(
        &self,
        block: &SourceBlock,
        table: &SourceTableMap,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let editor = cx.entity();
        let block_id = block.id;
        let alignments = table_alignments(table);
        let rows = table
            .rows
            .iter()
            .enumerate()
            .filter(|(row, _)| *row != 1)
            .map(|(row, source_row)| {
                h_flex().w_full().min_w_0().items_stretch().children(
                    source_row.cells.iter().enumerate().map(|(column, _)| {
                        self.render_table_cell(
                            TableCellAddress {
                                block_id: block.id,
                                row,
                                column,
                            },
                            row == 0,
                            alignments.get(column).copied().unwrap_or(TextAlign::Left),
                            cx,
                        )
                    }),
                )
            });
        let table_body = v_flex()
            .w_full()
            .min_w_0()
            .rounded_md()
            .border_1()
            .border_color(self.theme.border)
            .overflow_hidden()
            .children(rows);
        v_flex()
            .id(("markdown-table", block.id.0))
            .debug_selector(|| format!("markdown-table-{}", block.id.0))
            .w_full()
            .min_w_0()
            .my_2()
            .pt(gpui::px(34.))
            .relative()
            .on_prepaint(move |bounds, _, cx| {
                editor.update(cx, |editor, cx| {
                    editor.record_measured_block_height(
                        block_id,
                        bounds.size.height + gpui::px(16.),
                        cx,
                    );
                });
            })
            .child(table_body)
            .when_some(
                self.active_table_cell
                    .filter(|cell| cell.block_id == block.id),
                |table, address| table.child(self.render_table_toolbar(address, cx)),
            )
            .into_any_element()
    }

    fn render_table_cell(
        &self,
        address: TableCellAddress,
        header: bool,
        alignment: TextAlign,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = MarkdownSurfaceKey::table_cell(address);
        let surface = self
            .surface(key)
            .expect("every rendered table cell must own an edit surface");
        let input = surface.input.clone();
        let click_input = input.clone();
        let active = self.active_table_cell == Some(address) && self.active_surface_key() == key;
        let editor = cx.entity();
        gpui::div()
            .id(table_cell_id(address))
            .debug_selector(|| table_cell_selector(address))
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h(rems(2.5))
            .px_3()
            .py_2()
            .border_r_1()
            .border_b_1()
            .border_color(self.theme.border)
            .text_align(alignment)
            .when(header, |cell| {
                cell.bg(self.theme.border.opacity(0.2)).font_semibold()
            })
            .cursor_text()
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                editor.update(cx, |editor, cx| {
                    let input_contains_click = click_input
                        .read(cx)
                        .laid_out_input_bounds()
                        .contains(&event.position);
                    if editor.active_table_cell == Some(address)
                        && editor.active_surface_key() == key
                        && input_contains_click
                    {
                        return;
                    }
                    if event.click_count == 1 && !event.modifiers.shift {
                        editor.activate_table_cell_at(address, event.position, window, cx);
                    } else {
                        editor.focus_surface(key, window, cx);
                    }
                });
            })
            .child(
                gpui::div()
                    .id(table_cell_surface_id(address))
                    .debug_selector(|| table_cell_surface_selector(address))
                    .flex()
                    .flex_col()
                    .w_full()
                    .min_w_0()
                    .relative()
                    .child(
                        gpui::div()
                            .id(table_cell_input_id(address))
                            .debug_selector(move || {
                                if active {
                                    "markdown-active-table-input-slot".to_owned()
                                } else {
                                    table_cell_input_selector(address)
                                }
                            })
                            .flex()
                            .flex_col()
                            .w_full()
                            .min_w_0()
                            .child(
                                Input::new(&input)
                                    .w_full()
                                    .h_auto()
                                    .bare()
                                    .bordered(false)
                                    .focus_bordered(false)
                                    .local_style(self.input_style())
                                    .highlight_theme(self.theme.highlight_theme.clone())
                                    .editor_scrollbar(false)
                                    .text_layout_margin(false)
                                    .text_size(gpui::px(MARKDOWN_BODY_FONT_SIZE))
                                    .line_height(gpui::px(MARKDOWN_BODY_LINE_HEIGHT))
                                    .text_align(alignment)
                                    .caret_color(self.theme.primary)
                                    .when(header, |input| input.font_semibold()),
                            ),
                    )
                    .children(self.inline_math_overlays(key)),
            )
            .into_any_element()
    }
}

fn table_alignments(table: &SourceTableMap) -> Vec<TextAlign> {
    table
        .rows
        .get(1)
        .map(|row| {
            row.cells
                .iter()
                .map(|cell| delimiter_alignment(cell.original_source.trim()))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn delimiter_alignment(delimiter: &str) -> TextAlign {
    match (delimiter.starts_with(':'), delimiter.ends_with(':')) {
        (true, true) => TextAlign::Center,
        (false, true) => TextAlign::Right,
        _ => TextAlign::Left,
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

fn table_cell_surface_id(address: TableCellAddress) -> SharedString {
    table_cell_surface_selector(address).into()
}

fn table_cell_surface_selector(address: TableCellAddress) -> String {
    format!(
        "markdown-table-cell-edit-surface-{}-{}-{}",
        address.block_id.0, address.row, address.column
    )
}

fn table_cell_input_id(address: TableCellAddress) -> SharedString {
    table_cell_input_selector(address).into()
}

fn table_cell_input_selector(address: TableCellAddress) -> String {
    format!(
        "markdown-table-cell-input-slot-{}-{}-{}",
        address.block_id.0, address.row, address.column
    )
}
