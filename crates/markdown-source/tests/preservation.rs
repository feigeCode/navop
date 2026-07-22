use markdown_source::{SourceBlockKind, SourceMarkdownDocument};

#[test]
fn opening_markdown_keeps_source_bytes_and_valid_ranges() {
    let cases = [
        "",
        "\u{feff}# 标题\r\n\r\nUse snake_case(value)\r\n",
        "2. second\n",
        "_italic_ and __bold__\n\n",
        "line with hard break  \nnext",
    ];
    for source in cases {
        let document = SourceMarkdownDocument::parse(source).unwrap();
        assert_eq!(source, document.source);
        for block in &document.blocks {
            assert!(source.is_char_boundary(block.source_range.start));
            assert!(source.is_char_boundary(block.source_range.end));
            assert_eq!(&source[block.source_range.clone()], block.original_source);
        }
    }
}

#[test]
fn parser_records_heading_and_paragraph_byte_ranges() {
    let source = "# 标题\n\nUse snake_case(value)\n";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    assert_eq!(2, document.blocks.len());
    assert!(matches!(
        document.blocks[0].kind,
        SourceBlockKind::Heading { level: 1, .. }
    ));
    assert_eq!("# 标题", document.blocks[0].original_source);
    assert!(matches!(
        document.blocks[1].kind,
        SourceBlockKind::Paragraph
    ));
    assert_eq!("Use snake_case(value)", document.blocks[1].original_source);
    assert_eq!(
        Some(document.blocks[0].id),
        document
            .block_at(source.find("标题").unwrap())
            .map(|block| block.id)
    );
    assert_eq!(
        Some(document.blocks[1].source_range.clone()),
        document
            .block_by_id(document.blocks[1].id)
            .map(|block| block.source_range.clone())
    );
}

#[test]
fn unknown_directive_is_preserved_as_raw_markdown() {
    let source = "# Safe\n\n::: custom-extension\nUnknown\n:::\n\nAfter";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let raw = document
        .blocks
        .iter()
        .find(|block| matches!(block.kind, SourceBlockKind::RawMarkdown))
        .expect("unknown directive must be represented explicitly");
    assert_eq!("::: custom-extension\nUnknown\n:::", raw.original_source);
    assert!(document.compatibility.partially_editable);
    assert!(!document.compatibility.fully_editable);
}
