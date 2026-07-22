use markdown_source::{
    SourceBlockKind, SourceInlineKind, SourceMarkdownDocument, TableCellAddress,
};

#[test]
fn table_cells_keep_exact_ranges_and_linked_image_mapping() {
    let source = concat!(
        "| Name | Preview |\n",
        "| :--- | ---: |\n",
        "| DB | [![Database](database.png)](database.png) |\n"
    );
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let SourceBlockKind::Table(table) = &document.blocks[0].kind else {
        panic!("expected table block");
    };
    let cell = &table.rows[2].cells[1];
    assert_eq!(
        "[![Database](database.png)](database.png)",
        &source[cell.content_range.clone()]
    );
    let image = cell
        .inline_nodes
        .iter()
        .find_map(|node| match &node.kind {
            SourceInlineKind::Image(image) => Some(image),
            _ => None,
        })
        .expect("linked image must be mapped as an image");
    assert_eq!("Database", &source[image.alt_range.clone()]);
    assert_eq!("database.png", &source[image.destination_range.clone()]);
    let outer = image
        .outer_link
        .as_ref()
        .expect("outer link must be mapped");
    assert_eq!("database.png", &source[outer.destination_range.clone()]);
    assert_eq!(
        "[![Database](database.png)](database.png)",
        &source[image.full_range.clone()]
    );
}

#[test]
fn caret_inside_inline_syntax_reveals_only_that_original_source() {
    let source = "Before _italic_ and [link](path_(item)) after";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let emphasis = document
        .active_inline_source(source.find("italic").unwrap())
        .expect("emphasis should expose its source while active");
    assert_eq!("_italic_", emphasis.source);
    let link = document
        .active_inline_source(source.find("link").unwrap())
        .expect("link should expose its source while active");
    assert_eq!("[link](path_(item))", link.source);
}

#[test]
fn table_and_image_operations_patch_only_mapped_ranges() {
    let source = concat!(
        "| Name | Preview |\n",
        "| :--- | ---: |\n",
        "| DB | [![Database](database.png)](database.png) |\n"
    );
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let SourceBlockKind::Table(table) = &document.blocks[0].kind else {
        panic!("expected table block");
    };
    let address = TableCellAddress {
        block_id: document.blocks[0].id,
        row: 2,
        column: 0,
    };
    let edited = document
        .apply_transaction(&document.edit_table_cell(address, "Database").unwrap())
        .unwrap()
        .document;
    assert!(edited.source.contains("| Database | [![Database]"));
    assert!(edited.source.contains("| :--- | ---: |"));

    let image_id = table.rows[2].cells[1]
        .inline_nodes
        .iter()
        .find(|node| matches!(node.kind, SourceInlineKind::Image(_)))
        .unwrap()
        .id;
    let deleted = document
        .apply_transaction(&document.delete_image(image_id).unwrap())
        .unwrap()
        .document;
    assert_eq!(
        concat!("| Name | Preview |\n", "| :--- | ---: |\n", "| DB |  |\n"),
        deleted.source
    );
}

#[test]
fn editing_image_properties_is_one_source_transaction() {
    let source = "Before [![Old](old.png)](old.png) after";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let image_id = document.blocks[0]
        .inline_nodes
        .iter()
        .find(|node| matches!(node.kind, SourceInlineKind::Image(_)))
        .unwrap()
        .id;
    let transaction = document.edit_image(image_id, "New", "new.png").unwrap();
    assert_eq!(2, transaction.edits.len());
    let edited = document.apply_transaction(&transaction).unwrap().document;
    assert_eq!("Before [![New](new.png)](old.png) after", edited.source);
}
