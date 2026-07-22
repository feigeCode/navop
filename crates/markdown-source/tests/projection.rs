use markdown_source::{SourceMarkdownDocument, reconcile_projection};

#[test]
fn normalized_projection_does_not_rewrite_unchanged_source() {
    let original = concat!(
        "_italic_ and __bold__\n\n",
        "2. second\n\n",
        "[README](README_CN.md)\n\n",
        "|A|B|\n|-|-|\n|1|2|\n"
    );
    let candidate = concat!(
        "*italic* and **bold**\n\n",
        "1. second\n\n",
        "[README](<README_CN.md>)\n\n",
        "| A | B |\n| :--- | :--- |\n| 1 | 2 |"
    );
    let document = SourceMarkdownDocument::parse(original).unwrap();
    let result = reconcile_projection(&document, candidate).unwrap();
    assert_eq!(original, result.document.source);
    assert!(result.transaction.is_none());
}

#[test]
fn projection_edit_preserves_unrelated_markdown_bytes() {
    let original = "# Title\n\n_italic_\n\n2. second\n";
    let candidate = "# New Title\n\n*italic*\n\n1. second";
    let document = SourceMarkdownDocument::parse(original).unwrap();
    let result = reconcile_projection(&document, candidate).unwrap();
    assert_eq!(
        "# New Title\n\n_italic_\n\n2. second\n",
        result.document.source
    );
    let transaction = result
        .transaction
        .expect("title edit should create a patch");
    assert_eq!(1, transaction.edits.len());
}

#[test]
fn inline_edit_preserves_unedited_markers_in_the_same_paragraph() {
    let original = "Use _snake_case(value)_ and old text.\n";
    let candidate = "Use *snake_case(value)* and new text.";
    let document = SourceMarkdownDocument::parse(original).unwrap();
    let result = reconcile_projection(&document, candidate).unwrap();
    assert_eq!(
        "Use _snake_case(value)_ and new text.\n",
        result.document.source
    );
}

#[test]
fn list_item_edit_preserves_original_ordered_marker() {
    let original = "2. old text\n";
    let candidate = "1. new text";
    let document = SourceMarkdownDocument::parse(original).unwrap();
    let result = reconcile_projection(&document, candidate).unwrap();
    assert_eq!("2. new text\n", result.document.source);
}

#[test]
fn table_cell_edit_preserves_layout_and_linked_image_wrapper() {
    let original = concat!(
        "| Name | Preview |\n",
        "| :--- | ---: |\n",
        "| DB | [![Database](database.png)](database.png) |\n"
    );
    let candidate = concat!(
        "| Name | Preview |\n",
        "| :--- | ---: |\n",
        "| Database | ![Database](database.png) |"
    );
    let document = SourceMarkdownDocument::parse(original).unwrap();
    let result = reconcile_projection(&document, candidate).unwrap();
    assert_eq!(
        concat!(
            "| Name | Preview |\n",
            "| :--- | ---: |\n",
            "| Database | [![Database](database.png)](database.png) |\n"
        ),
        result.document.source
    );
}

#[test]
fn rich_projection_cannot_overwrite_preserved_raw_nodes() {
    let original = concat!(
        "# Title\n\n",
        "::: custom-extension\nKeep _exactly_ this\n:::\n\n",
        "After\n"
    );
    let candidate = concat!(
        "# New Title\n\n",
        "::: custom-extension\nKeep *exactly* this\n:::\n\n",
        "After"
    );
    let document = SourceMarkdownDocument::parse(original).unwrap();
    let result = reconcile_projection(&document, candidate).unwrap();
    assert_eq!(
        concat!(
            "# New Title\n\n",
            "::: custom-extension\nKeep _exactly_ this\n:::\n\n",
            "After\n"
        ),
        result.document.source
    );
}
