use markdown_source::{SourceInlineKind, SourceMarkdownDocument};

#[test]
fn delimited_inline_nodes_expose_content_ranges_without_markers() {
    let source = "Run `cargo test` and solve $x + y$.";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let nodes = &document.blocks[0].inline_nodes;
    let code = nodes
        .iter()
        .find(|node| matches!(node.kind, SourceInlineKind::InlineCode { .. }))
        .unwrap();
    let math = nodes
        .iter()
        .find(|node| matches!(node.kind, SourceInlineKind::InlineMath { .. }))
        .unwrap();
    assert_eq!("cargo test", &source[code.content_range.clone().unwrap()]);
    assert_eq!("x + y", &source[math.content_range.clone().unwrap()]);
}
