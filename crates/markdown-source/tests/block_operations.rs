use markdown_source::{
    BlockMoveDirection, SourceHistory, SourceMarkdownDocument, SourceParseScope,
};

fn apply(
    document: &SourceMarkdownDocument,
    transaction: &markdown_source::SourceTransaction,
) -> markdown_source::SourceEditTransaction {
    document.apply_transaction(transaction).unwrap()
}

#[test]
fn deleting_a_middle_block_preserves_the_surrounding_source() {
    let document = SourceMarkdownDocument::parse("first\n\nsecond\n\nthird\n").unwrap();
    let transaction = document.delete_block(document.blocks[1].id).unwrap();
    let applied = apply(&document, &transaction);

    assert_eq!("first\n\nthird\n", applied.document.source);
    assert_eq!(SourceParseScope::FullDocument, applied.parse_scope);
}

#[test]
fn moving_a_block_preserves_block_spelling_and_the_original_gap() {
    let document = SourceMarkdownDocument::parse("_first_\n\n\n**second**\n").unwrap();
    let transaction = document
        .move_block(document.blocks[0].id, BlockMoveDirection::Down)
        .unwrap();
    let applied = apply(&document, &transaction);

    assert_eq!("**second**\n\n\n_first_\n", applied.document.source);
    assert_eq!(SourceParseScope::FullDocument, applied.parse_scope);
}

#[test]
fn splitting_list_items_reuses_the_marker_style() {
    let ordered = SourceMarkdownDocument::parse("2. one").unwrap();
    let ordered_transaction = ordered
        .split_block(ordered.blocks[0].id, ordered.source.len())
        .unwrap();
    assert_eq!(
        "2. one\n3. ",
        apply(&ordered, &ordered_transaction).document.source
    );

    let unordered = SourceMarkdownDocument::parse("- item").unwrap();
    let unordered_transaction = unordered
        .split_block(unordered.blocks[0].id, unordered.source.len())
        .unwrap();
    assert_eq!(
        "- item\n- ",
        apply(&unordered, &unordered_transaction).document.source
    );
}

#[test]
fn splitting_task_items_starts_the_next_item_unchecked() {
    for source in ["- [ ] Todo", "- [x] Done", "* [X] Done"] {
        let document = SourceMarkdownDocument::parse(source).unwrap();
        let transaction = document
            .split_block(document.blocks[0].id, document.source.len())
            .unwrap();
        let bullet = source.chars().next().unwrap();

        assert_eq!(
            format!("{source}\n{bullet} [ ] "),
            apply(&document, &transaction).document.source
        );
    }
}

#[test]
fn blockquote_and_code_fence_toggles_round_trip() {
    let paragraph = SourceMarkdownDocument::parse("one\ntwo").unwrap();
    let quote = apply(
        &paragraph,
        &paragraph.toggle_blockquote(paragraph.blocks[0].id).unwrap(),
    )
    .document;
    assert_eq!("> one\n> two", quote.source);
    let unquoted = apply(
        &quote,
        &quote.toggle_blockquote(quote.blocks[0].id).unwrap(),
    )
    .document;
    assert_eq!(paragraph.source, unquoted.source);

    let fenced = apply(
        &paragraph,
        &paragraph
            .toggle_code_fence(paragraph.blocks[0].id, Some("rust"))
            .unwrap(),
    )
    .document;
    assert_eq!("```rust\none\ntwo\n```", fenced.source);
    let unfenced = apply(
        &fenced,
        &fenced.toggle_code_fence(fenced.blocks[0].id, None).unwrap(),
    )
    .document;
    assert_eq!(paragraph.source, unfenced.source);
}

#[test]
fn block_operations_are_single_undoable_transactions() {
    let original = "first\n\nsecond\n";
    let document = SourceMarkdownDocument::parse(original).unwrap();
    let transaction = document
        .move_block(document.blocks[1].id, BlockMoveDirection::Up)
        .unwrap();
    let mut history = SourceHistory::new(document);

    history.apply(&transaction).unwrap();
    assert_eq!("second\n\nfirst\n", history.document().source);
    history.undo().unwrap().unwrap();
    assert_eq!(original, history.document().source);
}
