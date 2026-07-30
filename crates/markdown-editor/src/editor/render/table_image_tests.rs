use super::*;
use markdown_source::SourceBlockKind;
use std::path::{Path, PathBuf};

#[test]
fn preview_segments_replace_images_and_keep_surrounding_projected_text() {
    let source = "| A |\n| --- |\n| Before ![one](one.png) middle ![two](two.png) after |\n";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let cell = table_cell(&document);
    let projection =
        MarkdownProjection::build_surface_range(&document, None, cell.content_range.clone());

    let segments = table_cell_preview_segments(&document, cell, &projection).unwrap();

    assert!(matches!(
        segments.as_slice(),
        [
            TableCellPreviewSegment::Text(before),
            TableCellPreviewSegment::Image {
                alt: one_alt,
                destination: one_destination,
                ..
            },
            TableCellPreviewSegment::Text(middle),
            TableCellPreviewSegment::Image {
                alt: two_alt,
                destination: two_destination,
                ..
            },
            TableCellPreviewSegment::Text(after),
        ] if before == "Before "
            && one_alt == "one"
            && one_destination == "one.png"
            && middle == " middle "
            && two_alt == "two"
            && two_destination == "two.png"
            && after == " after"
    ));
}

#[test]
fn linked_image_preview_does_not_leave_outer_link_markers_in_text() {
    let source = "| A |\n| --- |\n| Before [![logo](logo.png)](https://example.com) after |\n";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let cell = table_cell(&document);
    let projection =
        MarkdownProjection::build_surface_range(&document, None, cell.content_range.clone());

    let segments = table_cell_preview_segments(&document, cell, &projection).unwrap();

    assert!(matches!(
        segments.as_slice(),
        [
            TableCellPreviewSegment::Text(before),
            TableCellPreviewSegment::Image {
                alt,
                destination,
                ..
            },
            TableCellPreviewSegment::Text(after),
        ] if before == "Before "
            && alt == "logo"
            && destination == "logo.png"
            && after == " after"
    ));
}

#[test]
fn preview_segments_keep_unicode_boundaries() {
    let source = "| A |\n| --- |\n| 前缀 ![图标](logo.png) 后缀 |\n";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let cell = table_cell(&document);
    let projection =
        MarkdownProjection::build_surface_range(&document, None, cell.content_range.clone());

    let segments = table_cell_preview_segments(&document, cell, &projection).unwrap();

    assert!(matches!(
        segments.as_slice(),
        [
            TableCellPreviewSegment::Text(before),
            TableCellPreviewSegment::Image {
                alt,
                destination,
                ..
            },
            TableCellPreviewSegment::Text(after),
        ] if before == "前缀 "
            && alt == "图标"
            && destination == "logo.png"
            && after == " 后缀"
    ));
}

#[test]
fn relative_image_paths_resolve_from_the_markdown_document_directory() {
    let source = resolve_markdown_image_location(
        "assets/logo.png".to_owned(),
        Some(Path::new("/tmp/notes")),
    );

    assert_eq!(
        source,
        MarkdownImageLocation::Path(PathBuf::from("/tmp/notes/assets/logo.png"))
    );
}

#[test]
fn absolute_image_paths_are_not_rebased() {
    let source = resolve_markdown_image_location(
        "/tmp/shared/logo.png".to_owned(),
        Some(Path::new("/tmp/notes")),
    );

    assert_eq!(
        source,
        MarkdownImageLocation::Path(PathBuf::from("/tmp/shared/logo.png"))
    );
}

#[test]
fn relative_image_paths_without_a_document_directory_keep_their_original_path() {
    let source = resolve_markdown_image_location("assets/logo.png".to_owned(), None);

    assert_eq!(
        source,
        MarkdownImageLocation::Path(PathBuf::from("assets/logo.png"))
    );
}

#[test]
fn remote_and_data_image_sources_are_not_rebased() {
    for destination in [
        "https://example.com/logo.png",
        "//cdn.example.com/logo.png",
        "data:image/png;base64,AA==",
    ] {
        assert_eq!(
            resolve_markdown_image_location(destination.to_owned(), Some(Path::new("/tmp/notes"))),
            MarkdownImageLocation::Uri(destination.to_owned())
        );
    }
}

fn table_cell(document: &SourceMarkdownDocument) -> &SourceTableCell {
    let SourceBlockKind::Table(table) = &document.blocks[0].kind else {
        panic!("fixture must parse as a table");
    };
    &table.rows[2].cells[0]
}
