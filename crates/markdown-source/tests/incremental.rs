use markdown_source::{
    SourceEdit, SourceEditOrigin, SourceMarkdownDocument, SourceParseScope, SourceSelection,
    SourceTransaction,
};

#[test]
fn edit_inside_one_block_uses_incremental_parse_and_keeps_trailing_ids() {
    let source = "# Title\n\nBefore _old_ after\n\nTrailing [link](target)";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let trailing_id = document.blocks[2].id;
    let trailing_range = document.blocks[2].source_range.clone();
    let start = source.find("old").unwrap();
    let transaction = SourceTransaction {
        edits: vec![SourceEdit::new(start..start + 3, "longer", 0)],
        origin: SourceEditOrigin::RichTextTyping,
        allowed_ranges: vec![start..start + 3],
        selection_before: SourceSelection::default(),
        selection_after: SourceSelection::default(),
    };
    let applied = document.apply_transaction(&transaction).unwrap();
    assert_eq!(SourceParseScope::SingleBlock, applied.parse_scope);
    assert_eq!(trailing_id, applied.document.blocks[2].id);
    assert_eq!(
        trailing_range.start + 3..trailing_range.end + 3,
        applied.document.blocks[2].source_range
    );
    assert_eq!(
        "Before _longer_ after",
        applied.document.blocks[1].original_source
    );
}

#[test]
fn structural_edit_falls_back_to_full_document_parse() {
    let source = "one\n\ntwo";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let transaction = SourceTransaction {
        edits: vec![SourceEdit::new(3..5, "\n\n# inserted\n\n", 0)],
        origin: SourceEditOrigin::InsertBlock,
        allowed_ranges: vec![3..5],
        selection_before: SourceSelection::default(),
        selection_after: SourceSelection::default(),
    };
    let applied = document.apply_transaction(&transaction).unwrap();
    assert_eq!(SourceParseScope::FullDocument, applied.parse_scope);
    assert!(applied.document.blocks.len() >= 3);
}
