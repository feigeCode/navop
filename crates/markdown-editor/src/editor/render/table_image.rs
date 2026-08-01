use super::{
    MARKDOWN_BODY_FONT_SIZE, MARKDOWN_BODY_LINE_HEIGHT, MarkdownEditor,
    table::{table_cell_input_id, table_cell_input_selector},
};
use crate::{MarkdownProjection, editor::surface::MarkdownSurfaceKey};
use gpui::{
    AnyElement, Entity, ImageSource, InteractiveElement, IntoElement, ObjectFit, ParentElement,
    SharedString, Styled, StyledImage, TextAlign, img, prelude::FluentBuilder, relative, rems,
};
use gpui_component::{
    StyledExt,
    input::{Input, InputState},
};
use markdown_source::{
    SourceInlineKind, SourceMarkdownDocument, SourceNodeId, SourceTableCell, TableCellAddress,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
enum TableCellPreviewSegment {
    Text(String),
    Image {
        node_id: SourceNodeId,
        alt: String,
        destination: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MarkdownImageLocation {
    Uri(String),
    Path(PathBuf),
}

impl MarkdownEditor {
    pub(super) fn render_table_cell_surface_content(
        &self,
        address: TableCellAddress,
        input: Entity<InputState>,
        active: bool,
        alignment: TextAlign,
        header: bool,
    ) -> AnyElement {
        let image_preview = self.render_table_cell_image_preview(address, alignment, header);
        gpui::div()
            .grid()
            .grid_cols(1)
            .w_full()
            .min_w_0()
            .child(self.render_table_cell_input_layer(
                address,
                input,
                active,
                alignment,
                header,
                image_preview.is_some(),
            ))
            .children(image_preview.map(|preview| {
                gpui::div()
                    .col_start(1)
                    .row_start(1)
                    .w_full()
                    .min_w_0()
                    .when(active, |this| this.invisible())
                    .child(preview)
            }))
            .into_any_element()
    }

    fn render_table_cell_input_layer(
        &self,
        address: TableCellAddress,
        input: Entity<InputState>,
        active: bool,
        alignment: TextAlign,
        header: bool,
        has_image_preview: bool,
    ) -> AnyElement {
        gpui::div()
            .col_start(1)
            .row_start(1)
            .w_full()
            .min_w_0()
            .opacity(if has_image_preview && !active { 0. } else { 1. })
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
                    .child(self.render_table_cell_input(&input, alignment, header)),
            )
            .children(self.inline_math_overlays(MarkdownSurfaceKey::table_cell(address)))
            .into_any_element()
    }

    fn render_table_cell_input(
        &self,
        input: &Entity<InputState>,
        alignment: TextAlign,
        header: bool,
    ) -> Input {
        Input::new(input)
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
            .when(header, |input| input.font_semibold())
    }

    fn render_table_cell_image_preview(
        &self,
        address: TableCellAddress,
        alignment: TextAlign,
        header: bool,
    ) -> Option<AnyElement> {
        let document = self.history.document();
        let cell = document.table_cell(address).ok()?;
        let projection =
            MarkdownProjection::build_surface_range(document, None, cell.content_range.clone());
        let segments = table_cell_preview_segments(document, cell, &projection)?;
        let preview = gpui::div()
            .id(table_cell_preview_id(address))
            .debug_selector(move || table_cell_preview_selector(address))
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .w_full()
            .min_w_0()
            .min_h(rems(4.))
            .text_align(alignment)
            .when(alignment == TextAlign::Center, |this| this.justify_center())
            .when(alignment == TextAlign::Right, |this| this.justify_end())
            .when(header, |this| this.font_semibold())
            .children(segments.into_iter().map(|segment| match segment {
                TableCellPreviewSegment::Text(text) => gpui::div().child(text).into_any_element(),
                TableCellPreviewSegment::Image {
                    node_id,
                    alt,
                    destination,
                } => self.render_table_cell_image(address, node_id, alt, destination),
            }));
        Some(preview.into_any_element())
    }

    fn render_table_cell_image(
        &self,
        address: TableCellAddress,
        node_id: SourceNodeId,
        alt: String,
        destination: String,
    ) -> AnyElement {
        let fallback_alt = alt.clone();
        let muted = self.theme.muted_foreground;
        gpui::div()
            .id(table_cell_image_id(address, node_id))
            .debug_selector(move || table_cell_image_selector(address, node_id))
            .flex()
            .items_center()
            .justify_center()
            .h(rems(4.))
            .max_w(relative(1.))
            .child(
                img(markdown_image_source(
                    destination,
                    self.resource_base_path.as_deref(),
                ))
                .h_full()
                .max_w(relative(1.))
                .object_fit(ObjectFit::Contain)
                .with_loading(|| gpui::div().size_full().into_any_element())
                .with_fallback(move || {
                    gpui::div()
                        .flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .px_2()
                        .text_color(muted)
                        .child(fallback_alt.clone())
                        .into_any_element()
                }),
            )
            .into_any_element()
    }
}

fn table_cell_preview_segments(
    document: &SourceMarkdownDocument,
    cell: &SourceTableCell,
    projection: &MarkdownProjection,
) -> Option<Vec<TableCellPreviewSegment>> {
    let mut images = cell
        .inline_nodes
        .iter()
        .filter_map(|node| match &node.kind {
            SourceInlineKind::Image(image) => Some((node.id, image)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if images.is_empty() {
        return None;
    }
    images.sort_by_key(|(_, image)| image.alt_range.start);

    let mut segments = Vec::with_capacity(images.len() * 2 + 1);
    let mut display_cursor = 0;
    for (node_id, image) in images {
        let image_start = projection.source_to_display(image.alt_range.start);
        let image_end = projection.source_to_display(image.alt_range.end);
        push_preview_text(&mut segments, &projection.text, display_cursor..image_start);
        segments.push(TableCellPreviewSegment::Image {
            node_id,
            alt: document.source[image.alt_range.clone()].to_owned(),
            destination: document.source[image.destination_range.clone()].to_owned(),
        });
        display_cursor = image_end;
    }
    push_preview_text(
        &mut segments,
        &projection.text,
        display_cursor..projection.text.len(),
    );
    Some(segments)
}

fn push_preview_text(
    segments: &mut Vec<TableCellPreviewSegment>,
    projected_text: &str,
    range: std::ops::Range<usize>,
) {
    if range.start < range.end {
        segments.push(TableCellPreviewSegment::Text(
            projected_text[range].to_owned(),
        ));
    }
}

fn markdown_image_source(destination: String, resource_base_path: Option<&Path>) -> ImageSource {
    match resolve_markdown_image_location(destination, resource_base_path) {
        MarkdownImageLocation::Uri(uri) => gpui::SharedUri::from(uri).into(),
        MarkdownImageLocation::Path(path) => path.into(),
    }
}

fn resolve_markdown_image_location(
    destination: String,
    resource_base_path: Option<&Path>,
) -> MarkdownImageLocation {
    if destination.contains("://")
        || destination.starts_with("data:")
        || destination.starts_with("//")
    {
        MarkdownImageLocation::Uri(destination)
    } else {
        let path = PathBuf::from(destination);
        if path.is_absolute() {
            MarkdownImageLocation::Path(path)
        } else if let Some(resource_base_path) = resource_base_path {
            MarkdownImageLocation::Path(resource_base_path.join(path))
        } else {
            MarkdownImageLocation::Path(path)
        }
    }
}

fn table_cell_preview_id(address: TableCellAddress) -> SharedString {
    table_cell_preview_selector(address).into()
}

fn table_cell_preview_selector(address: TableCellAddress) -> String {
    format!(
        "markdown-table-cell-preview-{}-{}-{}",
        address.block_id.0, address.row, address.column
    )
}

fn table_cell_image_id(address: TableCellAddress, node_id: SourceNodeId) -> SharedString {
    table_cell_image_selector(address, node_id).into()
}

fn table_cell_image_selector(address: TableCellAddress, node_id: SourceNodeId) -> String {
    format!(
        "markdown-table-cell-image-{}-{}-{}-{}",
        address.block_id.0, address.row, address.column, node_id.0
    )
}

#[cfg(test)]
#[path = "table_image_tests.rs"]
mod tests;
