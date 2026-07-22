use markdown_source::{InlineFormat, ListFormat, SourceMarkdownDocument};

#[test]
fn inline_formats_wrap_and_unwrap_exact_source_ranges() {
    for (format, expected) in [
        (InlineFormat::Bold, "before **word** after"),
        (InlineFormat::Italic, "before _word_ after"),
        (InlineFormat::Underline, "before <u>word</u> after"),
        (InlineFormat::Strike, "before ~~word~~ after"),
        (InlineFormat::Code, "before `word` after"),
    ] {
        let document = SourceMarkdownDocument::parse("before word after").unwrap();
        let transaction = document.toggle_inline_format(7..11, format).unwrap();
        let formatted = document.apply_transaction(&transaction).unwrap().document;
        assert_eq!(expected, formatted.source);
        let range = 7..expected.len() - 6;
        let transaction = formatted.toggle_inline_format(range, format).unwrap();
        assert_eq!(
            "before word after",
            formatted
                .apply_transaction(&transaction)
                .unwrap()
                .document
                .source
        );
    }
}

#[test]
fn heading_and_list_commands_replace_only_the_active_block() {
    let document = SourceMarkdownDocument::parse("keep\n\ntarget").unwrap();
    let target = document.blocks[1].id;
    let heading = document
        .apply_transaction(&document.set_block_heading(target, Some(2)).unwrap())
        .unwrap()
        .document;
    assert_eq!("keep\n\n## target", heading.source);

    let list = document
        .apply_transaction(
            &document
                .toggle_list_format(target, ListFormat::Task)
                .unwrap(),
        )
        .unwrap()
        .document;
    assert_eq!("keep\n\n- [ ] target", list.source);
}

#[test]
fn duplicate_block_preserves_original_spelling() {
    let document = SourceMarkdownDocument::parse("_value_").unwrap();
    let transaction = document.duplicate_block(document.blocks[0].id).unwrap();
    let duplicated = document.apply_transaction(&transaction).unwrap().document;
    assert_eq!("_value_\n\n_value_", duplicated.source);
}
