use super::MarkdownEditor;
use crate::MarkdownEditorError;
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled,
};
use gpui_component::{
    IconName, Selectable, Sizable,
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
        let alignment = self.active_column_alignment(address);
        let controls = h_flex()
            .gap_1()
            .child(self.render_table_grid_picker(address, cx))
            .children(row_buttons(&editor))
            .children(column_buttons(&editor))
            .children(alignment_buttons(&editor, alignment));
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

    fn active_column_alignment(&self, address: TableCellAddress) -> TableAlignment {
        let Some(block) = self.history.document().block_by_id(address.block_id) else {
            return TableAlignment::None;
        };
        let markdown_source::SourceBlockKind::Table(table) = &block.kind else {
            return TableAlignment::None;
        };
        let Some(delimiter) = table
            .rows
            .get(1)
            .and_then(|row| row.cells.get(address.column))
        else {
            return TableAlignment::None;
        };
        let value = delimiter.original_source.trim();
        match (value.starts_with(':'), value.ends_with(':')) {
            (true, true) => TableAlignment::Center,
            (false, true) => TableAlignment::Right,
            (true, false) => TableAlignment::Left,
            _ => TableAlignment::None,
        }
    }

    fn render_table_grid_picker(
        &self,
        address: TableCellAddress,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let editor = cx.entity();
        Popover::new(("markdown-table-grid", address.block_id.0))
            .trigger(
                Button::new(("markdown-table-grid-trigger", address.block_id.0))
                    .debug_selector(|| "markdown-table-grid-trigger".to_owned())
                    .icon(IconName::Table)
                    .ghost()
                    .xsmall()
                    .tooltip("调整表格大小"),
            )
            .on_open_change({
                let editor = editor.clone();
                move |open, _, cx| {
                    if !open {
                        editor.update(cx, |editor, cx| {
                            editor.table_grid_hover = None;
                            cx.notify();
                        });
                    }
                }
            })
            .content(move |_state, _window, cx| {
                let popover = cx.entity();
                let hovered = editor.read(cx).table_grid_hover;
                let label = hovered
                    .map(|(rows, columns)| format!("{columns} × {rows} 表格"))
                    .unwrap_or_else(|| "选择表格大小".to_owned());
                let label_selector = hovered
                    .map(|(rows, columns)| format!("markdown-table-grid-label-{columns}x{rows}"))
                    .unwrap_or_else(|| "markdown-table-grid-label-empty".to_owned());
                let grid_border = editor.read(cx).theme.border;
                v_flex()
                    .id(("markdown-table-grid-content", address.block_id.0))
                    .debug_selector(|| {
                        format!("markdown-table-grid-content-{}", address.block_id.0)
                    })
                    .gap_2()
                    .p_2()
                    .child(
                        gpui::div()
                            .debug_selector(move || label_selector.clone())
                            .text_sm()
                            .text_color(editor.read(cx).theme.muted_foreground)
                            .child(label),
                    )
                    .child(v_flex().gap_1().children((1..=6).map(|rows| {
                        h_flex().gap_1().children((1..=6).map(|columns| {
                            grid_cell(
                                editor.clone(),
                                popover.clone(),
                                rows,
                                columns,
                                grid_cell_highlighted(hovered, rows, columns),
                                grid_border,
                            )
                        }))
                    })))
            })
            .into_any_element()
    }
}

/// A hovered cell highlights the whole rectangle from the top-left corner of
/// the grid down to the hovered cell, matching Typora's table size picker.
fn grid_cell_highlighted(hovered: Option<(usize, usize)>, rows: usize, columns: usize) -> bool {
    hovered.is_some_and(|(hover_rows, hover_columns)| {
        rows <= hover_rows && columns <= hover_columns
    })
}

fn row_buttons(editor: &gpui::Entity<MarkdownEditor>) -> Vec<Button> {
    vec![
        icon_button("table-row-before", IconName::ArrowUp, "在上方插入行").on_click(
            table_listener(editor.clone(), move |editor, window, cx| {
                editor.insert_active_table_row(TableInsertPosition::Before, window, cx)
            }),
        ),
        icon_button("table-row-after", IconName::ArrowDown, "在下方插入行").on_click(
            table_listener(editor.clone(), move |editor, window, cx| {
                editor.insert_active_table_row(TableInsertPosition::After, window, cx)
            }),
        ),
        icon_button("table-delete-row", IconName::Minus, "删除当前行").on_click(table_listener(
            editor.clone(),
            MarkdownEditor::delete_active_table_row,
        )),
    ]
}

fn column_buttons(editor: &gpui::Entity<MarkdownEditor>) -> Vec<Button> {
    vec![
        icon_button("table-column-before", IconName::ArrowLeft, "在左侧插入列").on_click(
            table_listener(editor.clone(), move |editor, window, cx| {
                editor.insert_active_table_column(TableInsertPosition::Before, window, cx)
            }),
        ),
        icon_button("table-column-after", IconName::ArrowRight, "在右侧插入列").on_click(
            table_listener(editor.clone(), move |editor, window, cx| {
                editor.insert_active_table_column(TableInsertPosition::After, window, cx)
            }),
        ),
        icon_button("table-delete-column", IconName::Column, "删除当前列").on_click(
            table_listener(editor.clone(), MarkdownEditor::delete_active_table_column),
        ),
    ]
}

fn alignment_buttons(
    editor: &gpui::Entity<MarkdownEditor>,
    current: TableAlignment,
) -> Vec<Button> {
    [
        (
            "table-align-left",
            IconName::AlignLeft,
            "左对齐",
            TableAlignment::Left,
        ),
        (
            "table-align-center",
            IconName::AlignCenter,
            "居中对齐",
            TableAlignment::Center,
        ),
        (
            "table-align-right",
            IconName::AlignRight,
            "右对齐",
            TableAlignment::Right,
        ),
    ]
    .into_iter()
    .map(|(id, icon, tooltip, alignment)| {
        icon_button(id, icon, tooltip)
            .selected(current == alignment)
            .on_click(table_listener(editor.clone(), move |editor, window, cx| {
                editor.align_active_table_column(alignment, window, cx)
            }))
    })
    .collect()
}

fn delete_table_button(editor: gpui::Entity<MarkdownEditor>) -> Button {
    icon_button("table-delete", IconName::Delete, "删除表格").on_click(move |_, window, cx| {
        editor.update(cx, |editor, cx| {
            let _ = editor.delete_active_block(window, cx);
        });
    })
}

fn grid_cell(
    editor: gpui::Entity<MarkdownEditor>,
    popover: gpui::Entity<gpui_component::popover::PopoverState>,
    rows: usize,
    columns: usize,
    highlighted: bool,
    border: gpui::Hsla,
) -> gpui::AnyElement {
    gpui::div()
        .id(SharedString::from(format!(
            "markdown-table-grid-hover-{rows}-{columns}"
        )))
        .debug_selector(move || format!("markdown-table-size-{rows}-{columns}"))
        .on_hover({
            let editor = editor.clone();
            move |hovered, _, cx| {
                if *hovered {
                    editor.update(cx, |editor, cx| {
                        editor.table_grid_hover = Some((rows, columns));
                        cx.notify();
                    });
                }
            }
        })
        .child(
            Button::new(SharedString::from(format!(
                "markdown-table-size-{rows}-{columns}"
            )))
            .label("")
            .xsmall()
            .ghost()
            .selected(highlighted)
            .w(gpui::px(24.))
            .h(gpui::px(24.))
            .border_1()
            // Ghost buttons intentionally have a transparent border. The
            // picker needs an explicit theme border so unselected cells do
            // not disappear against the popover background.
            .border_color(border)
            .tooltip(format!("{columns} × {rows}"))
            .on_click(move |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.table_grid_hover = None;
                    let _ = editor.resize_active_table(rows, columns, window, cx);
                });
                popover.update(cx, |popover, cx| popover.dismiss(window, cx));
            }),
        )
        .into_any_element()
}

fn icon_button(id: &'static str, icon: IconName, tooltip: &'static str) -> Button {
    Button::new(id)
        .debug_selector(move || id.to_owned())
        .icon(icon)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_hover_highlights_the_top_left_rectangle() {
        let hovered = Some((4, 5));
        assert!(grid_cell_highlighted(hovered, 1, 1));
        assert!(grid_cell_highlighted(hovered, 4, 5));
        assert!(grid_cell_highlighted(hovered, 2, 3));
        assert!(!grid_cell_highlighted(hovered, 5, 1));
        assert!(!grid_cell_highlighted(hovered, 1, 6));
        assert!(!grid_cell_highlighted(hovered, 6, 6));
        assert!(!grid_cell_highlighted(None, 1, 1));
    }
}
