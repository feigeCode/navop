use markdown_editor::{MarkdownProjection, ProjectionStyle};
use markdown_source::SourceMarkdownDocument;

#[test]
fn inactive_inline_nodes_hide_markers_but_keep_utf8_mapping() {
    let source = "中文 _italic_ 🎉 and [link](path_(item))";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let projection = MarkdownProjection::build(&document, None);
    assert_eq!("中文 italic 🎉 and link", projection.text);
    let display = projection.text.find("italic").unwrap();
    let source_offset = source.find("italic").unwrap();
    assert_eq!(source_offset, projection.display_to_source(display));
    assert_eq!(display, projection.source_to_display(source_offset));
}

#[test]
fn active_inline_node_reveals_only_its_original_source() {
    let source = "Before _italic_ and **bold** after";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let inactive = MarkdownProjection::build(&document, None);
    let source_offset = source.find("italic").unwrap();
    let node = document.inline_node_at(source_offset).unwrap();
    let active = MarkdownProjection::build(&document, Some(node.id));
    assert_eq!("Before _italic_ and bold after", active.text);
    assert_eq!("Before italic and bold after", inactive.text);
}

#[test]
fn nested_inline_source_reveals_only_the_exact_active_node() {
    let source = "Before **bold _nested_ text** after";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let nested = document
        .inline_node_at(source.find("nested").unwrap())
        .filter(|node| {
            matches!(
                node.kind,
                markdown_source::SourceInlineKind::Emphasis { .. }
            )
        })
        .unwrap();
    let projection = MarkdownProjection::build(&document, Some(nested.id));

    assert_eq!("Before bold _nested_ text after", projection.text);
}

#[test]
fn block_markers_stay_hidden_while_inline_markers_follow_the_cursor() {
    let source = "## Before _italic_ after";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let block = &document.blocks[0];
    let inactive = MarkdownProjection::build_range(&document, None, block.source_range.clone());
    let inline = document
        .inline_node_at(source.find("italic").unwrap())
        .unwrap();
    let active =
        MarkdownProjection::build_range(&document, Some(inline.id), block.source_range.clone());

    assert_eq!("Before italic after", inactive.text);
    assert_eq!("Before _italic_ after", active.text);
}

#[test]
fn structural_block_syntax_is_projected_as_content() {
    let source = concat!(
        "> quoted **text**\n\n",
        "- first\n- second\n\n",
        "```rust\nlet value = 1;\n```",
    );
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let projections = document
        .blocks
        .iter()
        .map(|block| {
            MarkdownProjection::build_range(&document, None, block.source_range.clone()).text
        })
        .collect::<Vec<_>>();

    assert_eq!("quoted text", projections[0]);
    assert_eq!("first\nsecond\n", projections[1]);
    assert_eq!("let value = 1;", projections[2]);
}

#[test]
fn math_syntax_is_projected_like_typora_source_blocks() {
    let slash = char::from(92);
    let source = format!("Euler: $e^{{i{slash}pi}} + 1 = 0$\n\n$$\n{slash}frac{{a}}{{b}}\n$$");
    let document = SourceMarkdownDocument::parse(&source).unwrap();
    assert!(matches!(
        document.blocks[1].kind,
        markdown_source::SourceBlockKind::MathBlock { .. }
    ));
    let paragraph =
        MarkdownProjection::build_range(&document, None, document.blocks[0].source_range.clone());
    let math =
        MarkdownProjection::build_range(&document, None, document.blocks[1].source_range.clone());
    assert_eq!(format!("Euler: e^{{i{slash}pi}} + 1 = 0"), paragraph.text);
    assert_eq!(format!("{slash}frac{{a}}{{b}}"), math.text);
}

#[test]
fn block_projection_keeps_global_source_offsets() {
    let source = "# Title\n\nUse _old_ here";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let paragraph = document.blocks[1].source_range.clone();
    let projection = MarkdownProjection::build_range(&document, None, paragraph.clone());
    assert_eq!("Use old here", projection.text);
    assert_eq!(paragraph.start, projection.display_to_source(0));
    assert_eq!(
        source.find("old").unwrap(),
        projection.display_to_source(projection.text.find("old").unwrap())
    );
}

#[test]
fn projected_text_edit_maps_back_to_exact_source_range() {
    let source = "Use _old_ and [link](target).";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let projection = MarkdownProjection::build(&document, None);
    let edit = projection
        .edit_for_value("Use new and link.")
        .expect("text change should map to source");
    assert_eq!(
        source.find("old").unwrap()..source.find("old").unwrap() + 3,
        edit.source_range
    );
    assert_eq!("new", edit.replacement);
}

#[test]
fn hidden_marker_offsets_collapse_to_the_correct_display_boundary() {
    let source = "_old_";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let projection = MarkdownProjection::build(&document, None);
    assert_eq!(0, projection.source_to_display(0));
    assert_eq!(0, projection.source_to_display(1));
    assert_eq!(3, projection.source_to_display(4));
    assert_eq!(3, projection.source_to_display(5));
}

#[test]
fn active_nested_inline_reveals_only_the_selected_syntax() {
    let source = "[an _em_](target) and **bold**";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let offset = source.find("em").unwrap();
    let emphasis = document.inline_node_at(offset).unwrap();
    let active = MarkdownProjection::build(&document, Some(emphasis.id));
    assert_eq!("an _em_ and bold", active.text);
}

#[test]
fn active_linked_image_reveals_the_original_wrapper() {
    let source = "Before [![logo](logo.png)](logo.png) after";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let offset = source.find("logo").unwrap();
    let image = document.inline_node_at(offset).unwrap();
    let inactive = MarkdownProjection::build(&document, None);
    let active = MarkdownProjection::build(&document, Some(image.id));
    assert_eq!("Before logo after", inactive.text);
    assert_eq!(source, active.text);
}

#[test]
fn table_inline_nodes_can_be_activated() {
    let source = "| Name |\n| --- |\n| _value_ |\n";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let offset = source.find("value").unwrap();
    let inline = document.inline_node_at(offset).unwrap();
    let active = MarkdownProjection::build(&document, Some(inline.id));
    assert!(active.text.contains("_value_"));
}

#[test]
fn edit_spanning_hidden_syntax_is_rejected_instead_of_deleting_markers() {
    let document = SourceMarkdownDocument::parse("_one_ and [two](target)").unwrap();
    let projection = MarkdownProjection::build(&document, None);
    assert_eq!("one and two", projection.text);
    assert!(projection.edit_for_value("changed").is_none());
}

#[test]
fn inactive_projection_exposes_semantic_style_spans() {
    let source = "_italic_ **bold** `code` [link](target) ~~gone~~";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let projection = MarkdownProjection::build(&document, None);
    assert_eq!("italic bold code link gone", projection.text);
    for style in [
        ProjectionStyle::Emphasis,
        ProjectionStyle::Strong,
        ProjectionStyle::InlineCode,
        ProjectionStyle::Link,
        ProjectionStyle::Delete,
    ] {
        assert!(
            projection.styles.iter().any(|span| span.style == style),
            "missing style {style:?}"
        );
    }
}

#[test]
fn projection_style_spans_keep_their_source_node_identity() {
    let source = "$one$ and $two$";
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let projection = MarkdownProjection::build(&document, None);
    let math = projection
        .styles
        .iter()
        .filter(|span| span.style == ProjectionStyle::InlineMath)
        .collect::<Vec<_>>();
    assert_eq!(2, math.len());
    assert_ne!(math[0].node_id, math[1].node_id);
}

#[test]
fn raw_html_and_frontmatter_stay_as_node_level_source() {
    let source = concat!(
        "---\ntitle: Demo\n---\n\n",
        "<section>html</section>\n\n",
        "::: custom-extension\nraw body\n:::\n",
    );
    let document = SourceMarkdownDocument::parse(source).unwrap();
    let projection = MarkdownProjection::build(&document, None);
    assert_eq!(source, projection.text);
    assert!(!document.compatibility.fully_editable);
    assert_eq!(3, document.compatibility.source_only_nodes.len());
}
