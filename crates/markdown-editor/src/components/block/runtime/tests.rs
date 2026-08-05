use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::projection::{
    ExpandedInlineProjection, expanded_display_cursor_offset_for_clean,
    expanded_display_offset_for_clean,
};
use crate::components::markdown::code_highlight::{CodeHighlightPaint, CodeLanguageKey};
use crate::components::markdown::inline::{
    InlineFragment, InlineInsertionAttributes, InlineLinkHit, InlineScript, InlineStyle,
    InlineTextTree,
};
use crate::components::markdown::link::parse_link_reference_definitions;
use crate::components::{
    Block, BlockKind, BlockRecord, DeleteBack, IndentBlock, Newline, TableCellPosition,
};
use crate::i18n::I18nManager;
use crate::theme::ThemeManager;
use crate::{
    CodeHighlightProvider, CodeHighlightResult as HostCodeHighlightResult, CodeHighlightService,
    CodeHighlightSpan as HostCodeHighlightSpan, CodeHighlightStyle, EditorHostServices,
};
use gpui::{
    AppContext, EntityInputHandler, FontStyle, FontWeight, Modifiers, MouseButton, MouseMoveEvent,
    TestAppContext, point, px, rgba,
};

fn assert_only_code_range(block: &Block, expected: Range<usize>) {
    let code_ranges = block
        .inline_spans()
        .iter()
        .filter(|span| span.style.code)
        .map(|span| span.range.clone())
        .collect::<Vec<_>>();
    assert_eq!(code_ranges, vec![expected]);
}

#[gpui::test]
async fn tab_inserts_character_in_paragraph(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph("ab")));

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.move_to(1, block_cx);
            block.on_indent_block(&IndentBlock, window, block_cx);
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "a    b");
        assert_eq!(block.selected_range, 5..5);
    });
}

#[gpui::test]
async fn tab_inserts_character_in_code_block(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::with_plain_text(BlockKind::CodeBlock { language: None }, "ab"),
        )
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.move_to(1, block_cx);
            block.on_indent_block(&IndentBlock, window, block_cx);
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "a    b");
        assert_eq!(block.selected_range, 5..5);
    });
}

#[test]
fn expanded_code_cursor_offset_stays_before_closing_backtick() {
    let fragments = vec![InlineFragment {
        text: "123".to_string(),
        style: InlineStyle {
            code: true,
            ..InlineStyle::default()
        },
        html_style: None,
        link: None,
        footnote: None,
        math: None,
    }];

    assert_eq!(expanded_display_offset_for_clean(&fragments, 0), 1);
    assert_eq!(expanded_display_offset_for_clean(&fragments, 3), 5);
    assert_eq!(expanded_display_cursor_offset_for_clean(&fragments, 0), 1);
    assert_eq!(expanded_display_cursor_offset_for_clean(&fragments, 3), 4);
}

#[test]
fn expanded_code_cursor_offset_keeps_plain_text_boundaries() {
    let fragments = vec![
        InlineFragment {
            text: "a".to_string(),
            style: InlineStyle::default(),
            html_style: None,
            link: None,
            footnote: None,
            math: None,
        },
        InlineFragment {
            text: "bc".to_string(),
            style: InlineStyle {
                code: true,
                ..InlineStyle::default()
            },
            html_style: None,
            link: None,
            footnote: None,
            math: None,
        },
    ];

    assert_eq!(expanded_display_cursor_offset_for_clean(&fragments, 1), 1);
    assert_eq!(expanded_display_cursor_offset_for_clean(&fragments, 3), 4);
}

#[test]
fn expanded_footnote_projection_maps_only_to_utf8_boundaries() {
    for id in ["éa", "aé", "€a", "a€", "中a", "a中", "é😀"] {
        let raw = format!("[^{id}]");
        let mut tree = InlineTextTree::from_markdown(&raw);
        tree.apply_footnote_reference_state(|candidate| {
            assert_eq!(candidate, id);
            Some((12, 0))
        });
        let clean = tree.visible_text();
        assert_eq!(clean, "¹²");

        let projection =
            ExpandedInlineProjection::build(&tree.fragments, 0..0, None).expect("projection");
        let display = projection.cache.visible_text();
        assert_eq!(display, raw);
        assert_eq!(projection.clean_to_display_cursor.len(), clean.len() + 1);
        assert_eq!(projection.display_to_clean.len(), display.len() + 1);
        assert_eq!(projection.clean_to_display_cursor.first(), Some(&2));
        assert_eq!(
            projection.clean_to_display_cursor.last(),
            Some(&(display.len() - 1))
        );
        assert_eq!(projection.display_to_clean.first(), Some(&0));
        assert_eq!(projection.display_to_clean.last(), Some(&clean.len()));
        assert!(
            projection
                .clean_to_display_cursor
                .windows(2)
                .all(|pair| pair[0] <= pair[1])
        );
        assert!(
            projection
                .display_to_clean
                .windows(2)
                .all(|pair| pair[0] <= pair[1])
        );

        for mapped in &projection.clean_to_display_cursor {
            assert!(
                display.is_char_boundary(*mapped),
                "clean-to-display offset {mapped} is not a boundary in {display:?}"
            );
        }
        for mapped in &projection.display_to_clean {
            assert!(
                clean.is_char_boundary(*mapped),
                "display-to-clean offset {mapped} is not a boundary in {clean:?}"
            );
        }
    }
}

#[test]
fn adjacent_expanded_footnotes_share_the_same_markdown_boundary() {
    let raw = "[^é][^中]";
    let mut tree = InlineTextTree::from_markdown(raw);
    let mut ordinal = 0;
    tree.apply_footnote_reference_state(|_| {
        ordinal += 1;
        Some((ordinal, 0))
    });
    let clean = tree.visible_text();
    assert_eq!(clean, "¹²");

    let projection =
        ExpandedInlineProjection::build(&tree.fragments, 0..clean.len(), None).expect("projection");
    assert_eq!(projection.cache.visible_text(), raw);
    assert_eq!(projection.footnote_runs.len(), 2);

    let first = &projection.footnote_runs[0];
    let second = &projection.footnote_runs[1];
    let shared_display_offset = first.display_range.end;
    assert_eq!(shared_display_offset, second.display_range.start);
    assert_eq!(
        projection
            .footnote_run_containing_offset(shared_display_offset)
            .expect("shared endpoint")
            .clean_range,
        first.clean_range
    );

    let map = tree.markdown_offset_map();
    let first_markdown_end = map.visible_to_markdown_offset(first.clean_range.start)
        + first.footnote.raw_markdown().len();
    let second_markdown_start = map.visible_to_markdown_offset(second.clean_range.start);
    assert_eq!(first_markdown_end, second_markdown_start);
}

#[test]
fn typing_inside_manual_backticks_keeps_cursor_inside_code_span() {
    let tree = InlineTextTree::plain("``");
    let result = tree.replace_visible_range(1..1, "1", InlineInsertionAttributes::default());

    assert_eq!(result.tree.visible_text(), "1");
    assert_eq!(
        result.tree.fragments,
        vec![InlineFragment {
            text: "1".to_string(),
            style: InlineStyle {
                code: true,
                ..InlineStyle::default()
            },
            html_style: None,
            link: None,
            footnote: None,
            math: None,
        }]
    );

    let clean_cursor = result.map_offset(2);
    assert_eq!(clean_cursor, 1);
    assert_eq!(
        expanded_display_cursor_offset_for_clean(&result.tree.fragments, clean_cursor),
        2
    );
}

#[gpui::test]
async fn enter_inside_multiline_inline_code_inserts_hard_line_without_splitting(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("`line 1\nline 2`"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        let offset = "line 1\n".len();
        block.selected_range = offset..offset;
        cx.notify();
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.on_newline(&Newline, window, block_cx);
        });
    });

    block.read_with(cx, |block, _cx| {
        let text = "line 1\n\nline 2";
        assert_eq!(block.kind(), BlockKind::Paragraph);
        assert_eq!(block.display_text(), text);
        assert_eq!(block.selected_range, "line 1\n\n".len().."line 1\n\n".len());
        assert!(
            block
                .inline_spans()
                .iter()
                .any(|span| { span.style.code && span.range == (0..text.len()) })
        );
    });
}

#[gpui::test]
async fn inline_math_focus_stays_rendered_rich_and_keeps_links(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("**bold** $x^2$ [repo](https://example.com)"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        // The math source is shown inline (`$x^2$`) while bold and the link stay
        // collapsed; the block never falls back to raw Markdown editing.
        assert_eq!(block.display_text(), "bold $x^2$ repo");

        // Focusing with the caret inside the math keeps the rendered-rich
        // projection rather than dumping the whole block to raw source, so the
        // link in the same block keeps its link attribute.
        let caret = "bold $".len();
        block.move_to(caret, cx);
        block.sync_inline_projection_for_focus(true);
        assert!(!block.uses_raw_text_editing());
        assert!(block.record.title.has_mixed_inline_visuals());
        assert!(block.record.title.has_inline_links());
        assert!(
            block.inline_spans().iter().any(|span| span.link.is_some()),
            "link must stay styled while editing the math in the same block"
        );
        assert_eq!(
            block.record.title.serialize_markdown(),
            "**bold** $x^2$ [repo](https://example.com)"
        );
    });
}

#[gpui::test]
async fn script_spans_focus_stay_rendered_rich(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("x^2^ and H~2~O"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        assert_eq!(block.display_text(), "x2 and H2O");
        assert_eq!(block.inline_spans()[0].style.script, InlineScript::Normal);
        assert_eq!(
            block.inline_spans()[1].style.script,
            InlineScript::Superscript
        );
        assert!(!block.uses_raw_text_editing());
        assert_eq!(block.display_text(), "x2 and H2O");
        assert_eq!(block.record.title.serialize_markdown(), "x^2^ and H~2~O");
    });
}

#[gpui::test]
async fn link_anchor_emphasis_delimiters_are_revealed_when_caret_inside(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("[**bold**](https://example.com)"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        // Collapsed, only the styled anchor text is shown.
        assert_eq!(block.display_text(), "bold");

        // With the caret inside the bold anchor text, the projection reveals both
        // the link syntax and the anchor's own `**` emphasis markers, so they can
        // be edited instead of staying invisible.
        block.move_to(2, cx);
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "[**bold**](https://example.com)");
    });
}

#[gpui::test]
async fn mermaid_block_uses_raw_text_editing(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let markdown = "```mermaid\nflowchart LR\nA --> B\n```";
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::mermaid(markdown)));

    block.update(cx, |block, _cx| {
        assert_eq!(block.kind(), BlockKind::MermaidBlock);
        assert!(block.uses_raw_text_editing());
        assert_eq!(block.display_text(), markdown);
        assert_eq!(block.record.markdown_line(0, None), markdown);
    });
}

#[gpui::test]
async fn enter_inside_projected_inline_code_inserts_hard_line_without_splitting(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("`line 1\nline 2`"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        let offset = "line 1\n".len();
        block.selected_range = offset..offset;
        block.sync_inline_projection_for_focus(true);
        cx.notify();
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.on_newline(&Newline, window, block_cx);
        });
    });

    block.read_with(cx, |block, _cx| {
        let text = "line 1\n\nline 2";
        assert_eq!(block.kind(), BlockKind::Paragraph);
        assert_eq!(block.record.title.visible_text(), text);
        assert!(
            block
                .record
                .title
                .render_cache()
                .spans()
                .iter()
                .any(|span| span.style.code && span.range == (0..text.len()))
        );
    });
}

#[gpui::test]
async fn enter_outside_inline_code_still_splits_paragraph(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("alpha beta"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = "alpha".len().."alpha".len();
        cx.notify();
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.on_newline(&Newline, window, block_cx);
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.kind(), BlockKind::Paragraph);
        assert_eq!(block.display_text(), "alpha");
        assert_eq!(block.selected_range, "alpha".len().."alpha".len());
    });
}

#[gpui::test]
async fn enter_inside_comment_block_inserts_hard_line_without_splitting(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::comment("<!--\n**not bold** [not link](https://example.com)\n-->"),
        )
    });

    block.update(cx, |block, cx| {
        let offset = "<!--\n".len();
        block.selected_range = offset..offset;
        cx.notify();
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.on_newline(&Newline, window, block_cx);
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.kind(), BlockKind::Comment);
        assert_eq!(
            block.display_text(),
            "<!--\n\n**not bold** [not link](https://example.com)\n-->"
        );
        assert_eq!(block.inline_spans().len(), 1);
        assert_eq!(block.inline_spans()[0].range, 0..block.display_text().len());
        assert_eq!(block.inline_spans()[0].style, InlineStyle::default());
    });
}

#[gpui::test]
async fn paragraph_shortcut_creates_task_item_directly(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

    block.update(cx, |block, cx| {
        block.apply_title_edit(
            InlineTextTree::plain("- [x] task"),
            10,
            None,
            None,
            None,
            false,
            cx,
        );
    });

    let kind = block.read_with(cx, |block, _cx| block.kind());
    let text = block.read_with(cx, |block, _cx| block.display_text().to_string());
    assert_eq!(kind, BlockKind::TaskListItem { checked: true });
    assert_eq!(text, "task");
}

#[gpui::test]
async fn paragraph_shortcut_creates_parenthesized_numbered_list_directly(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

    block.update(cx, |block, cx| {
        block.apply_title_edit(
            InlineTextTree::plain("1) item"),
            7,
            None,
            None,
            None,
            false,
            cx,
        );
    });

    let kind = block.read_with(cx, |block, _cx| block.kind());
    let text = block.read_with(cx, |block, _cx| block.display_text().to_string());
    assert_eq!(kind, BlockKind::NumberedListItem);
    assert_eq!(text, "item");
}

#[gpui::test]
async fn bullet_shortcut_upgrades_to_task_item_after_box_prefix(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

    block.update(cx, |block, cx| {
        block.apply_title_edit(InlineTextTree::plain("- "), 2, None, None, None, false, cx);
    });
    let kind = block.read_with(cx, |block, _cx| block.kind());
    assert_eq!(kind, BlockKind::BulletedListItem);

    block.update(cx, |block, cx| {
        block.apply_title_edit(
            InlineTextTree::plain("[ ] "),
            4,
            None,
            None,
            None,
            false,
            cx,
        );
    });

    let kind = block.read_with(cx, |block, _cx| block.kind());
    let text = block.read_with(cx, |block, _cx| block.display_text().to_string());
    assert_eq!(kind, BlockKind::TaskListItem { checked: false });
    assert_eq!(text, "");
}

#[gpui::test]
async fn heading_shortcut_marker_is_transient_runtime_state(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

    block.update(cx, |block, cx| {
        block.apply_title_edit(InlineTextTree::plain("# "), 2, None, None, None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.kind(), BlockKind::Heading { level: 1 });
        assert_eq!(block.display_text(), "");
        assert_eq!(block.selected_range, 0..0);
        assert_eq!(block.heading_shortcut_marker(), Some(1));
        assert_eq!(block.record.markdown_line(0, None), "# ");
    });

    block.update(cx, |block, cx| {
        block.apply_title_edit(
            InlineTextTree::plain("Title"),
            5,
            None,
            None,
            None,
            false,
            cx,
        );
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "Title");
        assert_eq!(block.selected_range, 5..5);
        assert_eq!(block.heading_shortcut_marker(), Some(1));
        assert_eq!(block.record.markdown_line(0, None), "# Title");
    });

    block.update(cx, |block, cx| block.convert_to_paragraph(cx));
    block.read_with(cx, |block, _cx| {
        assert_eq!(block.kind(), BlockKind::Paragraph);
        assert_eq!(block.heading_shortcut_marker(), None);
    });

    let loaded_heading = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::with_plain_text(BlockKind::Heading { level: 1 }, "Existing"),
        )
    });
    loaded_heading.read_with(cx, |block, _cx| {
        assert_eq!(block.heading_shortcut_marker(), None);
        assert_eq!(block.record.markdown_line(0, None), "# Existing");
    });
}

#[gpui::test]
async fn heading_shortcut_marker_supports_all_atx_levels_without_offsetting_text(
    cx: &mut TestAppContext,
) {
    for level in 1..=6 {
        let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));
        let shortcut = format!("{} ", "#".repeat(level as usize));

        block.update(cx, |block, cx| {
            block.apply_title_edit(
                InlineTextTree::plain(shortcut.clone()),
                shortcut.len(),
                Some(0..shortcut.len()),
                Some(shortcut.len()..shortcut.len()),
                None,
                false,
                cx,
            );
        });

        block.read_with(cx, |block, _cx| {
            assert_eq!(block.kind(), BlockKind::Heading { level });
            assert_eq!(block.display_text(), "");
            assert_eq!(block.selected_range, 0..0);
            assert_eq!(block.marked_range, Some(0..0));
            assert_eq!(block.heading_shortcut_marker(), Some(level));
            assert_eq!(block.record.markdown_line(0, None), shortcut);
        });
    }
}

#[gpui::test]
async fn inline_code_projection_only_expands_touched_span(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a `code` b"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a code b"
    );

    block.update(cx, |block, _cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a `code` b"
    );

    block.update(cx, |block, _cx| {
        block.selected_range = 9..9;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a code b"
    );
}

#[gpui::test]
async fn inline_code_projection_expands_only_the_selected_code_span(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("`one` and `two`"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 1..1;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "`one` and two"
    );

    block.update(cx, |block, _cx| {
        block.selected_range = 10..10;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "one and `two`"
    );
}

#[gpui::test]
async fn bold_projection_only_expands_touched_span(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a **bold** b"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a bold b"
    );

    block.update(cx, |block, _cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a **bold** b"
    );
}

#[gpui::test]
async fn bold_projection_expands_only_the_selected_bold_span(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("**one** and **two**"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 1..1;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "**one** and two"
    );

    block.update(cx, |block, _cx| {
        block.clear_inline_projection();
        block.selected_range = "one and ".len().."one and ".len();
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "one and **two**"
    );
}

#[gpui::test]
async fn bold_projection_expands_selected_range_and_html_strong(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a **bold** b"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 2..6;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a **bold** b"
    );

    let html_block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("<strong>bold</strong>"),
            ),
        )
    });

    html_block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        html_block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "**bold**"
    );
}

#[gpui::test]
async fn bold_projection_marker_edit_unwraps_bold_style(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("**bold**"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "**bold**");
        block.replace_text_in_visible_range(0..2, "", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "bold");
        assert_eq!(block.record.title.serialize_markdown(), "bold");
        assert!(
            block
                .record
                .title
                .render_cache()
                .spans()
                .iter()
                .all(|span| !span.style.bold)
        );
    });
}

#[gpui::test]
async fn bold_projection_insertion_inside_span_preserves_bold_style(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("**bold**"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "**bold**");
        block.replace_text_in_visible_range(3..3, "X", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "**bXold**");
        assert_eq!(block.record.title.serialize_markdown(), "**bXold**");
        assert!(block.record.title.render_cache().spans()[0].style.bold);
    });
}

#[gpui::test]
async fn italic_projection_only_expands_touched_span(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a *italic* b"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a italic b"
    );

    block.update(cx, |block, _cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a *italic* b"
    );
}

#[gpui::test]
async fn italic_projection_marker_edit_unwraps_italic_style(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::from_markdown("*it*")),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "*it*");
        block.replace_text_in_visible_range(0..1, "", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "it");
        assert_eq!(block.record.title.serialize_markdown(), "it");
        assert!(
            block
                .record
                .title
                .render_cache()
                .spans()
                .iter()
                .all(|span| !span.style.italic)
        );
    });
}

#[gpui::test]
async fn typing_closing_italic_marker_places_caret_after_marker(cx: &mut TestAppContext) {
    // `*italic` is literal until the closing `*` is typed; afterwards the caret
    // must land *after* the closing marker so further typing stays plain.
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("*italic"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 7..7;
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "*italic");
        block.replace_text_in_visible_range(7..7, "*", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "*italic*");
        assert_eq!(block.cursor_offset(), "*italic*".len());
        assert_eq!(
            block.collapsed_caret_affinity,
            super::CollapsedCaretAffinity::OuterEnd
        );
    });
}

#[gpui::test]
async fn typing_closing_bold_marker_places_caret_after_marker(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("**bold*"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 7..7;
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "**bold*");
        block.replace_text_in_visible_range(7..7, "*", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "**bold**");
        assert_eq!(block.cursor_offset(), "**bold**".len());
        assert_eq!(
            block.collapsed_caret_affinity,
            super::CollapsedCaretAffinity::OuterEnd
        );
    });
}

#[gpui::test]
async fn typing_inside_span_keeps_default_affinity(cx: &mut TestAppContext) {
    // Inserting an ordinary character inside a bold span must not jump the
    // caret outside the span.
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("**bold**"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "**bold**");
        // Insert "X" inside the bold word (display offset 3 = after "**b").
        block.replace_text_in_visible_range(3..3, "X", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "**bXold**");
        assert_eq!(
            block.collapsed_caret_affinity,
            super::CollapsedCaretAffinity::Default
        );
    });
}

#[gpui::test]
async fn typing_bold_markers_char_by_char_produces_bold_not_italic(cx: &mut TestAppContext) {
    // Typing `**bold**` one character at a time must yield bold, not italic.
    // The clean parse is committed on each keystroke, so the intermediate
    // `**bold*` must not collapse to a literal `*` plus an italic `bold`.
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::from_markdown("")),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
        for ch in "**bold**".chars() {
            let caret = block.cursor_offset();
            block.replace_text_in_visible_range(caret..caret, &ch.to_string(), None, false, cx);
        }
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.record.title.visible_text(), "bold");
        assert_eq!(block.record.title.serialize_markdown(), "**bold**");
        assert!(
            block
                .record
                .title
                .render_cache()
                .spans()
                .iter()
                .all(|span| span.style.bold && !span.style.italic),
            "typed `**bold**` must be bold, not italic"
        );
    });
}

#[gpui::test]
async fn typing_after_closing_italic_marker_inserts_plain_text(cx: &mut TestAppContext) {
    // After typing `*italic*` the caret sits after the closing `*`, so further
    // typing must be plain text rather than being absorbed back into the span.
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::from_markdown("")),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
        for ch in "*italic* x".chars() {
            let caret = block.cursor_offset();
            block.replace_text_in_visible_range(caret..caret, &ch.to_string(), None, false, cx);
        }
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.record.title.visible_text(), "italic x");
        assert_eq!(block.record.title.serialize_markdown(), "*italic* x");
        // The trailing " x" must be a plain (non-italic) fragment.
        let trailing_is_italic = block
            .record
            .title
            .fragments
            .iter()
            .any(|fragment| fragment.text.contains('x') && fragment.style.italic);
        assert!(
            !trailing_is_italic,
            "text after closing `*` must not be italic"
        );
    });
}

#[gpui::test]
async fn typing_after_closing_bold_marker_inserts_plain_text(cx: &mut TestAppContext) {
    // Same as above for bold: typing past the closing `**` must be plain.
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::from_markdown("")),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
        for ch in "**bold** more".chars() {
            let caret = block.cursor_offset();
            block.replace_text_in_visible_range(caret..caret, &ch.to_string(), None, false, cx);
        }
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.record.title.visible_text(), "bold more");
        assert_eq!(block.record.title.serialize_markdown(), "**bold** more");
        let trailing_is_bold = block
            .record
            .title
            .fragments
            .iter()
            .any(|fragment| fragment.text.contains("more") && fragment.style.bold);
        assert!(
            !trailing_is_bold,
            "text after closing `**` must not be bold"
        );
    });
}

#[gpui::test]
async fn strikethrough_projection_only_expands_touched_span(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a ~~gone~~ b"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a gone b"
    );

    block.update(cx, |block, _cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a ~~gone~~ b"
    );
}

#[gpui::test]
async fn script_projection_expands_only_touched_span(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("x^2^ and H~2~O"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "x2 and H2O"
    );

    block.update(cx, |block, _cx| {
        block.selected_range = 1..1;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "x^2^ and H2O"
    );

    block.update(cx, |block, _cx| {
        block.clear_inline_projection();
        block.selected_range = "x2 and H".len().."x2 and H".len();
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "x2 and H~2~O"
    );
}

#[gpui::test]
async fn standalone_script_projection_uses_html_marker_fallback(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("<sup>2</sup> and <sub>n</sub>"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "<sup>2</sup> and n"
    );

    block.update(cx, |block, _cx| {
        block.clear_inline_projection();
        block.selected_range = "2 and ".len().."2 and ".len();
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "2 and <sub>n</sub>"
    );
}

#[gpui::test]
async fn script_projection_marker_edit_unwraps_script_style(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::from_markdown("x^2^")),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 1..1;
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "x^2^");
        block.replace_text_in_visible_range(1..2, "", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "x2");
        assert_eq!(block.record.title.serialize_markdown(), "x2");
        assert!(
            block
                .inline_spans()
                .iter()
                .all(|span| span.style.script == InlineScript::Normal)
        );
    });
}

#[gpui::test]
async fn subscript_projection_marker_edit_unwraps_script_style(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::from_markdown("H~2~O")),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 1..1;
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "H~2~O");
        block.replace_text_in_visible_range(1..2, "", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "H2O");
        assert_eq!(block.record.title.serialize_markdown(), "H2O");
        assert!(
            block
                .record
                .title
                .render_cache()
                .spans()
                .iter()
                .all(|span| span.style.script == InlineScript::Normal)
        );
    });
}

#[gpui::test]
async fn script_projection_insertion_inside_span_preserves_script_style(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::from_markdown("x^2^")),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 1..1;
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "x^2^");
        block.replace_text_in_visible_range(3..3, "3", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "x^23^");
        assert_eq!(block.record.title.serialize_markdown(), "x^23^");
        assert_eq!(
            block.record.title.render_cache().spans()[1].style.script,
            InlineScript::Superscript
        );
    });
}

#[gpui::test]
async fn inline_code_projection_right_escape_stays_outside_after_rebuild(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a `123` b"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 5..5;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(block.read_with(cx, |block, _cx| block.cursor_offset()), 6);

    block.update(cx, |block, _cx| {
        let (target, affinity) = block
            .projected_move_right_target(block.cursor_offset())
            .expect("inner end should jump to outer end");
        block.assign_collapsed_selection_offset(target, affinity, None);
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a `123` b"
    );
    assert_eq!(block.read_with(cx, |block, _cx| block.cursor_offset()), 7);

    block.update(cx, |block, _cx| {
        let target = block.next_boundary(block.cursor_offset());
        block.move_to_with_preferred_x(target, None, _cx);
    });
    assert_eq!(block.read_with(cx, |block, _cx| block.cursor_offset()), 8);
}

#[gpui::test]
async fn inline_code_projection_left_escape_stays_outside_after_rebuild(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a `123` b"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(block.read_with(cx, |block, _cx| block.cursor_offset()), 3);

    block.update(cx, |block, _cx| {
        let (target, affinity) = block
            .projected_move_left_target(block.cursor_offset())
            .expect("inner start should jump to outer start");
        block.assign_collapsed_selection_offset(target, affinity, None);
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(block.read_with(cx, |block, _cx| block.cursor_offset()), 2);

    block.update(cx, |block, _cx| {
        let target = block.previous_boundary(block.cursor_offset());
        block.move_to_with_preferred_x(target, None, _cx);
    });
    assert_eq!(block.read_with(cx, |block, _cx| block.cursor_offset()), 1);
}

#[gpui::test]
async fn strikethrough_projection_right_escape_stays_outside_after_rebuild(
    cx: &mut TestAppContext,
) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a ~~123~~ b"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 5..5;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(block.read_with(cx, |block, _cx| block.cursor_offset()), 7);

    block.update(cx, |block, _cx| {
        let (target, affinity) = block
            .projected_move_right_target(block.cursor_offset())
            .expect("inner end should jump to outer end");
        block.assign_collapsed_selection_offset(target, affinity, None);
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(block.read_with(cx, |block, _cx| block.cursor_offset()), 9);

    block.update(cx, |block, _cx| {
        let target = block.next_boundary(block.cursor_offset());
        block.move_to_with_preferred_x(target, None, _cx);
    });
    assert_eq!(block.read_with(cx, |block, _cx| block.cursor_offset()), 10);
}

#[gpui::test]
async fn strikethrough_projection_left_escape_stays_outside_after_rebuild(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a ~~bc~~ d"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(block.read_with(cx, |block, _cx| block.cursor_offset()), 4);

    block.update(cx, |block, _cx| {
        let (target, affinity) = block
            .projected_move_left_target(block.cursor_offset())
            .expect("expected projected move left target");
        block.assign_collapsed_selection_offset(target, affinity, None);
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(block.read_with(cx, |block, _cx| block.cursor_offset()), 2);

    block.update(cx, |block, _cx| {
        let target = block.previous_boundary(block.cursor_offset());
        block.move_to_with_preferred_x(target, None, _cx);
    });
    assert_eq!(block.read_with(cx, |block, _cx| block.cursor_offset()), 1);
}

#[gpui::test]
async fn word_start_boundaries_step_over_whole_words(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("hello world foo"),
            ),
        )
    });

    block.read_with(cx, |block, _cx| {
        // Word starts are at offsets 0 ("hello"), 6 ("world"), 12 ("foo").
        assert_eq!(block.next_word_start(0), 6);
        assert_eq!(block.next_word_start(3), 6);
        assert_eq!(block.next_word_start(6), 12);
        assert_eq!(block.next_word_start(12), 15);

        assert_eq!(block.previous_word_start(15), 12);
        assert_eq!(block.previous_word_start(12), 6);
        assert_eq!(block.previous_word_start(7), 6);
        assert_eq!(block.previous_word_start(6), 0);
        assert_eq!(block.previous_word_start(0), 0);
    });
}

#[gpui::test]
async fn inline_link_projection_only_expands_touched_span(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a [link](https://example.com) b"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a link b"
    );

    block.update(cx, |block, _cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a [link](https://example.com) b"
    );
}

#[gpui::test]
async fn reference_style_link_resolves_and_expands_preserving_raw_syntax(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        let mut block = Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("[reference link][ref-link]"),
            ),
        );
        block.set_runtime_context(
            None,
            Arc::default(),
            Arc::new(parse_link_reference_definitions(
                "[ref-link]: https://example.com",
            )),
            Arc::default(),
        );
        block
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "reference link"
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.inline_link_at(0).map(str::to_string)),
        Some("https://example.com".to_string())
    );

    block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "[reference link][ref-link]"
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.record.title.serialize_markdown()),
        "[reference link][ref-link]"
    );
}

#[gpui::test]
async fn reference_style_link_hit_exposes_raw_prompt_and_resolved_open_target(
    cx: &mut TestAppContext,
) {
    let block = cx.new(|cx| {
        let mut block = Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("[reference link][ref-links]"),
            ),
        );
        block.set_runtime_context(
            None,
            Arc::default(),
            Arc::new(parse_link_reference_definitions(
                "[ref-links]: https://example.com",
            )),
            Arc::default(),
        );
        block
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.inline_link_hit_at(0).cloned()),
        Some(InlineLinkHit {
            prompt_target: "ref-links".to_string(),
            open_target: "https://example.com".to_string(),
        })
    );
}

#[gpui::test]
async fn inline_link_with_title_expands_title_but_opens_destination(cx: &mut TestAppContext) {
    let markdown = "[ABC](https://abc.com \"https://abc.com\")";
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown(markdown),
            ),
        )
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.inline_link_hit_at(0).cloned()),
        Some(InlineLinkHit {
            prompt_target: "https://abc.com".to_string(),
            open_target: "https://abc.com".to_string(),
        })
    );

    block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        markdown
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.record.title.serialize_markdown()),
        markdown
    );
}

#[gpui::test]
async fn autolink_expands_with_angle_brackets_when_touched(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("<https://example.com>"),
            ),
        )
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "https://example.com"
    );

    block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "<https://example.com>"
    );
}

#[gpui::test]
async fn projected_reference_target_stays_link_hit_testable(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        let mut block = Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("[reference link][ref-link]"),
            ),
        );
        block.set_runtime_context(
            None,
            Arc::default(),
            Arc::new(parse_link_reference_definitions(
                "[ref-link]: https://example.com",
            )),
            Arc::default(),
        );
        block
    });

    let target_offset = block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
        block
            .display_text()
            .find("ref-link")
            .expect("projection should expose reference target")
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block
            .inline_link_hit_at(target_offset)
            .cloned()),
        Some(InlineLinkHit {
            prompt_target: "ref-link".to_string(),
            open_target: "https://example.com".to_string(),
        })
    );
}

#[gpui::test]
async fn projected_reference_syntax_maps_full_delimiter_range_back_to_markdown(
    cx: &mut TestAppContext,
) {
    let block = cx.new(|cx| {
        let mut block = Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("[reference link][ref-link]"),
            ),
        );
        block.set_runtime_context(
            None,
            Arc::default(),
            Arc::new(parse_link_reference_definitions(
                "[ref-link]: https://example.com",
            )),
            Arc::default(),
        );
        block
    });

    let display_len = block.update(cx, |block, _cx| {
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
        block.display_text().len()
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| {
            block.current_range_to_markdown_range(0..display_len)
        }),
        0.."[reference link][ref-link]".len()
    );
}

#[gpui::test]
async fn editing_link_destination_inside_projection_preserves_link(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a [link](https://example.com) b"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
        let expanded = block.display_text().to_string();
        let insert_at = expanded
            .find("example.com")
            .expect("expanded link should expose its destination");
        block.replace_text_in_visible_range(insert_at..insert_at, "docs.", None, false, cx);
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.record.title.serialize_markdown()),
        "a [link](https://docs.example.com) b"
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "a [link](https://docs.example.com) b"
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.inline_link_at(3).map(str::to_string)),
        Some("https://docs.example.com".to_string())
    );
}

#[gpui::test]
async fn typing_after_inline_link_preserves_link(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("[Link](https://example.com/) x"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        // Place the caret past the link (in the trailing text) so the edit does
        // not touch the link's projected run, then type. This is the case that
        // previously re-parsed from collapsed text and dropped the link.
        block.selected_range = 0..0;
        block.sync_inline_projection_for_focus(true);
        let end = block.display_text().len();
        block.selected_range = end..end;
        block.sync_inline_projection_for_focus(true);
        block.replace_text_in_visible_range(end..end, "y", None, false, cx);
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.record.title.serialize_markdown()),
        "[Link](https://example.com/) xy"
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.inline_link_at(0).map(str::to_string)),
        Some("https://example.com/".to_string())
    );
}

#[gpui::test]
async fn deleting_adjacent_text_preserves_reference_style_link_syntax(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        let mut block = Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("[ref][ref-link]a"),
            ),
        );
        block.set_runtime_context(
            None,
            Arc::default(),
            Arc::new(parse_link_reference_definitions(
                "[ref-link]: https://example.com",
            )),
            Arc::default(),
        );
        block
    });

    block.update(cx, |block, cx| {
        block.replace_text_in_visible_range(3..4, "", None, false, cx);
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.record.title.serialize_markdown()),
        "[ref][ref-link]"
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "ref"
    );
}

#[gpui::test]
async fn deleting_adjacent_text_preserves_autolink_syntax(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("<ref2>a"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        block.replace_text_in_visible_range(4..5, "", None, false, cx);
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.record.title.serialize_markdown()),
        "<ref2>"
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "ref2"
    );
}

#[gpui::test]
async fn link_projection_preserves_cursor_inside_destination_after_rebuild(
    cx: &mut TestAppContext,
) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a [link](https://example.com) b"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
        let expanded = block.display_text().to_string();
        let destination_offset = expanded
            .find("example.com")
            .expect("expanded link should expose destination text");
        block.move_to_with_preferred_x(destination_offset, None, cx);
        block.sync_inline_projection_for_focus(true);
    });

    let destination_offset = block.read_with(cx, |block, _cx| {
        block
            .display_text()
            .find("example.com")
            .expect("expanded link should expose destination text")
    });
    assert_eq!(
        block.read_with(cx, |block, _cx| block.cursor_offset()),
        destination_offset
    );
}

#[gpui::test]
async fn link_projection_preserves_selection_inside_destination_after_rebuild(
    cx: &mut TestAppContext,
) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a [link](https://example.com) b"),
            ),
        )
    });

    let selected_range = block.update(cx, |block, _cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
        let expanded = block.display_text().to_string();
        let destination_offset = expanded
            .find("example.com")
            .expect("expanded link should expose destination text");
        let selected_range = destination_offset..destination_offset + "example".len();
        block.selected_range = selected_range.clone();
        block.selection_reversed = false;
        block.sync_inline_projection_for_focus(true);
        selected_range
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.selected_range.clone()),
        selected_range
    );
}

#[gpui::test]
async fn link_middle_delimiter_click_snaps_to_destination_start(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a [link](https://example.com) b"),
            ),
        )
    });

    let destination_offset = block.update(cx, |block, cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
        let expanded = block.display_text().to_string();
        let middle = expanded
            .find("](")
            .expect("expanded link should expose middle delimiter");
        let destination_offset = expanded
            .find("https://")
            .expect("expanded link should expose destination start");
        let click_target = block.pointer_target_offset(middle + 1);
        block.move_to_with_preferred_x(click_target, None, cx);
        block.sync_inline_projection_for_focus(true);
        destination_offset
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.cursor_offset()),
        destination_offset
    );
}

#[gpui::test]
async fn reversed_selection_survives_projection_focus_refresh(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("alpha beta"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 1..7;
        block.selection_reversed = true;
        block.sync_inline_projection_for_focus(true);
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.selected_range.clone()),
        1..7
    );
    assert!(block.read_with(cx, |block, _cx| block.selection_reversed));
}

#[gpui::test]
async fn reversed_selection_survives_render_cache_refresh(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("alpha beta"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 1..7;
        block.selection_reversed = true;
        block.sync_render_cache();
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.selected_range.clone()),
        1..7
    );
    assert!(block.read_with(cx, |block, _cx| block.selection_reversed));
}

#[gpui::test]
async fn reversed_selection_survives_clear_inline_projection(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("`code`"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
        block.selected_range = 1..5;
        block.selection_reversed = true;
        block.clear_inline_projection();
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "code"
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.selected_range.clone()),
        0..4
    );
    assert!(block.read_with(cx, |block, _cx| block.selection_reversed));
}

#[gpui::test]
async fn reversed_selection_inside_link_destination_survives_focus_refresh(
    cx: &mut TestAppContext,
) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("a [link](https://example.com) b"),
            ),
        )
    });

    let expected = block.update(cx, |block, _cx| {
        block.selected_range = 2..2;
        block.sync_inline_projection_for_focus(true);
        let expanded = block.display_text().to_string();
        let destination_offset = expanded
            .find("example.com")
            .expect("expanded link should expose destination text");
        let expected = destination_offset..destination_offset + "example".len();
        block.selected_range = expected.clone();
        block.selection_reversed = true;
        block.sync_inline_projection_for_focus(true);
        expected
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.selected_range.clone()),
        expected
    );
    assert!(block.read_with(cx, |block, _cx| block.selection_reversed));
}

#[gpui::test]
async fn ime_selected_text_range_reports_reversed_for_right_to_left_selection(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("alpha beta"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 1..7;
        block.selection_reversed = true;
    });

    let selection = cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::selected_text_range(block, false, window, block_cx)
                .expect("selection")
        })
    });

    assert_eq!(selection.range, 1..7);
    assert!(selection.reversed);
}

#[gpui::test]
async fn ime_replace_text_replaces_right_to_left_selection_in_source_raw_mode(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        let mut block = Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("alpha beta"),
            ),
        );
        block.set_source_raw_mode();
        block
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 1..7;
        block.selection_reversed = true;
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "Z", window, block_cx,
            );
        });
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "aZeta"
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.selected_range.clone()),
        2..2
    );
    assert!(!block.read_with(cx, |block, _cx| block.selection_reversed));
}

#[gpui::test]
async fn source_document_mode_enables_line_numbers(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        let mut block = Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::plain("a\nb")),
        );
        block.set_source_document_mode();
        block
    });

    block.read_with(cx, |block, _cx| {
        assert!(block.is_source_raw_mode());
        assert!(block.show_source_line_numbers());
    });
}

#[gpui::test]
async fn source_raw_mode_does_not_enable_line_numbers(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        let mut block = Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::plain("raw")),
        );
        block.set_source_raw_mode();
        block
    });

    block.read_with(cx, |block, _cx| {
        assert!(block.is_source_raw_mode());
        assert!(!block.show_source_line_numbers());
    });
}

#[gpui::test]
async fn ime_replace_and_mark_text_replaces_right_to_left_selection_in_table_cell(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        let mut block = Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::from_markdown("alpha")),
        );
        block.set_table_cell_mode(
            TableCellPosition { row: 0, column: 0 },
            crate::components::TableColumnAlignment::Left,
        );
        block
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 1..4;
        block.selection_reversed = true;
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "XY",
                Some(0..1),
                window,
                block_cx,
            );
        });
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "aXYa"
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.selected_range.clone()),
        1..2
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.marked_range.clone()),
        Some(1..3)
    );
    assert!(!block.read_with(cx, |block, _cx| block.selection_reversed));
}

#[gpui::test]
async fn ime_commit_inside_inline_code_preserves_code_style(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("aaa`hello world`aaa"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        let cursor = "aaahello".len();
        block.selected_range = cursor..cursor;
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "ni",
                Some(2..2),
                window,
                block_cx,
            );
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "你", window, block_cx,
            );
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "aaahello你 worldaaa");
        assert_eq!(
            block.record.title.serialize_markdown(),
            "aaa`hello你 world`aaa"
        );
        assert_only_code_range(block, "aaa".len().."aaahello你 world".len());
    });
}

#[gpui::test]
async fn ime_commit_inside_projected_inline_code_preserves_code_style(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("aaa`hello world`aaa"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        let cursor = "aaahello".len();
        block.selected_range = cursor..cursor;
        block.sync_inline_projection_for_focus(true);
        assert_eq!(block.display_text(), "aaa`hello world`aaa");
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::replace_and_mark_text_in_range(
                block,
                None,
                "ni",
                Some(2..2),
                window,
                block_cx,
            );
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "你", window, block_cx,
            );
        });
    });

    block.update(cx, |block, _cx| {
        assert_eq!(
            block.record.title.serialize_markdown(),
            "aaa`hello你 world`aaa"
        );
        block.clear_inline_projection();
        assert_eq!(block.display_text(), "aaahello你 worldaaa");
        assert_only_code_range(block, "aaa".len().."aaahello你 world".len());
    });
}

#[gpui::test]
async fn replacing_selection_inside_inline_code_preserves_code_style(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("aaa`hello world`aaa"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        let start = "aaahello ".len();
        let end = "aaahello world".len();
        block.selected_range = start..end;
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "你", window, block_cx,
            );
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "aaahello 你aaa");
        assert_eq!(block.record.title.serialize_markdown(), "aaa`hello 你`aaa");
        assert_only_code_range(block, "aaa".len().."aaahello 你".len());
    });
}

#[gpui::test]
async fn replacing_selection_across_inline_code_boundary_stays_plain(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("aaa`hello`bbb"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = "aaahel".len().."aaahellobb".len();
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::replace_text_in_range(
                block, None, "你", window, block_cx,
            );
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.display_text(), "aaahel你b");
        assert_eq!(block.record.title.serialize_markdown(), "aaa`hel`你b");
        assert_only_code_range(block, "aaa".len().."aaahel".len());
    });
}

#[test]
fn ime_utf16_ranges_keep_multilingual_boundaries() {
    let text = "中文😀かな";
    let emoji_utf8 = "中文".len().."中文😀".len();
    assert_eq!(Block::utf16_range_to_utf8_in(text, &(2..4)), emoji_utf8);
    assert_eq!(Block::utf8_range_to_utf16_in(text, &emoji_utf8), 2..4);
}

#[gpui::test]
async fn ime_replace_text_handles_cjk_and_emoji_utf16_ranges(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        let mut block = Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::plain("中文😀かな".to_string()),
            ),
        );
        block.set_source_raw_mode();
        block
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::replace_text_in_range(
                block,
                Some(2..4),
                "語",
                window,
                block_cx,
            );
        });
    });

    assert_eq!(
        block.read_with(cx, |block, _cx| block.display_text().to_string()),
        "中文語かな"
    );
    assert_eq!(
        block.read_with(cx, |block, _cx| block.selected_range.clone()),
        "中文語".len().."中文語".len()
    );
}

#[gpui::test]
async fn ime_selection_ignores_editor_external_selection(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("alpha beta"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.selected_range = 1..1;
        block.editor_selection_range = Some(0..block.visible_len());
    });

    let selection = cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            <Block as EntityInputHandler>::selected_text_range(block, false, window, block_cx)
                .expect("selection")
        })
    });

    assert_eq!(selection.range, 1..1);
    assert!(!selection.reversed);
}

#[gpui::test]
async fn focusing_rendered_image_does_not_auto_expand(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("![diagram](./assets/diagram.png)"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.sync_render_cache();
        assert!(block.showing_rendered_image());
        assert!(!block.image_edit_expanded);

        assert!(!block.sync_image_focus_state(true));
        assert!(block.showing_rendered_image());
        assert!(!block.image_edit_expanded);
    });
}

#[gpui::test]
async fn requested_rendered_image_expansion_enters_raw_markdown_editing(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("![diagram](./assets/diagram.png)"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.sync_render_cache();
        block.request_image_edit_expansion();
        assert!(block.sync_image_focus_state(true));
        assert!(block.image_edit_expanded);
        assert!(!block.showing_rendered_image());
        assert_eq!(block.cursor_offset(), block.visible_len());
    });
}

#[gpui::test]
async fn blurred_valid_rendered_image_recovers_image_presentation(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("![diagram](./assets/diagram.png)"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.sync_render_cache();
        block.request_image_edit_expansion();
        assert!(block.sync_image_focus_state(true));
        assert!(block.image_edit_expanded);

        assert!(block.sync_image_focus_state(false));
        assert!(!block.image_edit_expanded);
        assert!(block.showing_rendered_image());
    });
}

#[gpui::test]
async fn broken_rendered_image_syntax_blurs_back_to_plain_text(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("![diagram](./assets/diagram.png)"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.sync_render_cache();
        block.request_image_edit_expansion();
        assert!(block.sync_image_focus_state(true));

        block
            .record
            .set_title(InlineTextTree::from_markdown("not an image anymore"));
        block.sync_render_cache();
        assert!(!block.sync_image_focus_state(false));
        assert!(block.image_runtime().is_none());
        assert!(!block.image_edit_expanded);
        assert!(!block.showing_rendered_image());
        assert_eq!(block.display_text(), "not an image anymore");
    });
}

#[gpui::test]
async fn code_block_cache_tracks_rust_language_and_available_spans(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("fn main() {\n    let value: i32 = 42;\n}\n"),
            ),
        )
    });

    // Highlighting is exclusively host/WASM-provided. A standalone block has
    // no host service, so it keeps the source plain instead of running a
    // native tree-sitter fallback.
    assert!(block.read_with(cx, |block, _cx| block.code_highlight_result().is_none()));
}

#[gpui::test]
async fn code_block_cache_updates_when_language_changes(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("fn main() {\n    let value = 42;\n}\n"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.record.kind = BlockKind::CodeBlock {
            language: Some("text".into()),
        };
        block.sync_render_cache();
    });

    assert!(block.read_with(cx, |block, _cx| block.code_highlight_result().is_none()));
}

#[gpui::test]
async fn host_code_highlighter_wins_and_normalizes_overlapping_ranges(cx: &mut TestAppContext) {
    let provider: CodeHighlightProvider = Arc::new(|_request| {
        Ok(HostCodeHighlightResult {
            spans: vec![
                HostCodeHighlightSpan {
                    range: 2..5,
                    style: CodeHighlightStyle {
                        color: Some(rgba(0x00ff00ff).into()),
                        ..CodeHighlightStyle::default()
                    },
                },
                HostCodeHighlightSpan {
                    range: 0..4,
                    style: CodeHighlightStyle {
                        color: Some(rgba(0xff0000ff).into()),
                        font_weight: Some(FontWeight::BOLD),
                        font_style: Some(FontStyle::Italic),
                    },
                },
                HostCodeHighlightSpan {
                    range: 1..2,
                    style: CodeHighlightStyle::default(),
                },
                HostCodeHighlightSpan {
                    range: 99..120,
                    style: CodeHighlightStyle::default(),
                },
            ],
        })
    });
    let services = EditorHostServices::default()
        .with_code_highlighter(CodeHighlightService::new(provider, Arc::new(|| 7)));
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("unknown-language".into()),
                },
                InlineTextTree::plain("éabc"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.set_host_services(Arc::new(services));
    });

    block.read_with(cx, |block, _cx| {
        let highlight = block
            .code_highlight_result()
            .expect("the host should provide a plain-text highlight result");
        assert_eq!(highlight.language, CodeLanguageKey::PlainText);
        assert_eq!(
            highlight
                .spans
                .iter()
                .map(|span| span.range.clone())
                .collect::<Vec<_>>(),
            vec![0..4, 4..5]
        );
        assert!(matches!(
            &highlight.spans[0].paint,
            CodeHighlightPaint::Host(CodeHighlightStyle {
                font_weight: Some(FontWeight::BOLD),
                font_style: Some(FontStyle::Italic),
                ..
            })
        ));
    });
}

#[gpui::test]
async fn host_code_highlighter_error_falls_back_to_plain_source(cx: &mut TestAppContext) {
    let provider: CodeHighlightProvider =
        Arc::new(|_request| Err("host highlighter unavailable".to_string()));
    let services = EditorHostServices::default()
        .with_code_highlighter(CodeHighlightService::new(provider, Arc::new(|| 1)));
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("fn main() {}"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.set_host_services(Arc::new(services));
    });

    block.read_with(cx, |block, _cx| {
        assert!(block.code_highlight_result().is_none());
    });
}

#[gpui::test]
async fn host_code_highlight_refreshes_when_registry_revision_changes(cx: &mut TestAppContext) {
    let revision = Arc::new(AtomicU64::new(1));
    let provider_revision = revision.clone();
    let provider: CodeHighlightProvider = Arc::new(move |_request| {
        let weight = if provider_revision.load(Ordering::SeqCst) == 1 {
            FontWeight::NORMAL
        } else {
            FontWeight::BOLD
        };
        Ok(HostCodeHighlightResult {
            spans: vec![HostCodeHighlightSpan {
                range: 0..2,
                style: CodeHighlightStyle {
                    font_weight: Some(weight),
                    ..CodeHighlightStyle::default()
                },
            }],
        })
    });
    let service_revision = revision.clone();
    let services = EditorHostServices::default().with_code_highlighter(CodeHighlightService::new(
        provider,
        Arc::new(move || service_revision.load(Ordering::SeqCst)),
    ));
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("text".into()),
                },
                InlineTextTree::plain("ab"),
            ),
        )
    });

    block.update(cx, |block, _cx| {
        block.set_host_services(Arc::new(services));
    });
    revision.store(2, Ordering::SeqCst);
    block.update(cx, |block, _cx| {
        block.sync_code_highlight_registry_revision();
    });

    block.read_with(cx, |block, _cx| {
        let Some(CodeHighlightPaint::Host(style)) = block
            .code_highlight_result()
            .and_then(|result| result.spans.first())
            .map(|span| &span.paint)
        else {
            panic!("expected a refreshed host highlight span");
        };
        assert_eq!(style.font_weight, Some(FontWeight::BOLD));
    });
}

#[gpui::test]
async fn code_block_language_setter_updates_highlight_without_changing_content(
    cx: &mut TestAppContext,
) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("print('hello')"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        let range = 0..block.code_language_text().len();
        block.replace_code_language_text_in_range(range, "python", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.code_language_text(), "python");
        assert_eq!(block.display_text(), "print('hello')");
        assert!(block.code_highlight_result().is_none());
    });
}

#[gpui::test]
async fn code_block_language_accepts_unknown_language_as_plain_rendering(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock {
                    language: Some("rust".into()),
                },
                InlineTextTree::plain("fn main() {}"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        let range = 0..block.code_language_text().len();
        block.replace_code_language_text_in_range(range, "unknown-lang", None, false, cx);
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.code_language_text(), "unknown-lang");
        assert!(block.code_highlight_result().is_none());
    });
}

#[gpui::test]
async fn non_dragging_mouse_move_ends_stale_text_selection(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
    });
    let (block, cx) = cx.add_window_view(|_window, cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::plain("hello world")),
        )
    });

    let event = MouseMoveEvent {
        position: point(px(8.0), px(8.0)),
        pressed_button: None,
        modifiers: Modifiers::default(),
    };
    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.is_selecting = true;
            block.selected_range = 3..7;
            block.marked_range = Some(4..6);

            block.on_mouse_move(&event, window, cx);

            assert!(!block.is_selecting);
            assert_eq!(block.selected_range, 3..7);
            assert_eq!(block.marked_range, Some(4..6));
        });
    });
}

#[gpui::test]
async fn dragging_mouse_move_keeps_text_selection_session_active(cx: &mut TestAppContext) {
    cx.update(|cx| {
        I18nManager::init(cx);
        ThemeManager::init(cx);
    });
    let (block, cx) = cx.add_window_view(|_window, cx| {
        Block::with_record(
            cx,
            BlockRecord::new(BlockKind::Paragraph, InlineTextTree::plain("hello world")),
        )
    });

    let event = MouseMoveEvent {
        position: point(px(8.0), px(8.0)),
        pressed_button: Some(MouseButton::Left),
        modifiers: Modifiers::default(),
    };
    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.is_selecting = true;
            block.on_mouse_move(&event, window, cx);
            assert!(block.is_selecting);
        });
    });
}

#[gpui::test]
async fn code_block_without_language_keeps_plain_rendering(cx: &mut TestAppContext) {
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::CodeBlock { language: None },
                InlineTextTree::plain("no highlighting here"),
            ),
        )
    });

    assert!(block.read_with(cx, |block, _cx| block.code_highlight_result().is_none()));
}

#[gpui::test]
async fn editing_link_anchor_in_math_block_matches_plain_paragraph(cx: &mut TestAppContext) {
    // A block mixing inline math with a link is "source preserving", which used
    // to route its link edits through the markdown-space path. That path assumed
    // the anchor label began right after `[`, so the anchor's own emphasis
    // markers shifted the mapping and edits landed on the wrong character. Inline
    // links now edit through the link projection in every block, so deleting a
    // revealed anchor delimiter touches the delimiter, not a label character.
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("$x^2$ [**bold**](https://e.com)"),
            ),
        )
    });

    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.move_to("$x^2$ bo".len(), cx);
            block.sync_inline_projection_for_focus(true);

            // Caret just past the revealed opening `**` of the bold anchor.
            let projected = block.display_text().to_string();
            assert_eq!(projected, "$x^2$ [**bold**](https://e.com)");
            let after_open = projected.find("[**").unwrap() + "[**".len();
            block.selected_range = after_open..after_open;

            block.on_delete_back(&DeleteBack, window, cx);

            let markdown = block.record.title.serialize_markdown();
            assert!(
                markdown.starts_with("$x^2$ "),
                "math source preserved: {markdown:?}"
            );
            assert!(
                markdown.contains("bold"),
                "anchor label must stay intact, only the delimiter is edited: {markdown:?}"
            );
        });
    });
}

#[gpui::test]
async fn completing_link_in_math_block_places_caret_after_closing_paren(cx: &mut TestAppContext) {
    // A block mixing math with a link edits in markdown space. Typing the closing
    // `)` completes the link, and the caret must land just past it (like a plain
    // paragraph) rather than inside the anchor before `]`.
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("$x$ [link](google.com"),
            ),
        )
    });

    cx.update(|window, cx| {
        block.update(cx, |block, cx| {
            block.move_to(block.visible_len(), cx);
            block.sync_inline_projection_for_focus(true);
            block.replace_text_in_range(None, ")", window, cx);
            block.sync_inline_projection_for_focus(true);

            assert_eq!(
                block.record.title.serialize_markdown(),
                "$x$ [link](google.com)"
            );
            assert_eq!(block.display_text(), "$x$ [link](google.com)");
            let end = block.visible_len();
            assert_eq!(block.selected_range, end..end);
        });
    });
}

#[gpui::test]
async fn rtl_selection_across_trailing_link_keeps_block_end_anchor(cx: &mut TestAppContext) {
    // A link sitting at the very end of a block that also contains inline math
    // stays expanded while the projection is rebuilt on every render. Dragging a
    // selection right-to-left from the block end across the link used to collapse
    // the anchor onto the closing `]` of the anchor text, because the trailing
    // `](url)` delimiters all share one clean offset and the remap snapped back
    // to the inner cursor position. The anchor must stay at the block end.
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("$x$ [link](google.com)"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        block.move_to(block.visible_len(), cx);
        block.sync_inline_projection_for_focus(true);
        let end = block.visible_len();
        assert_eq!(block.display_text(), "$x$ [link](google.com)");

        // Start an RTL selection at the block end and drag the head left,
        // re-syncing the projection after each move like the render loop does.
        block.move_to(end, cx);
        block.sync_inline_projection_for_focus(true);
        for target in (0..end).rev() {
            block.select_to(target, cx);
            block.sync_inline_projection_for_focus(true);
            assert_eq!(
                block.selected_range,
                target..end,
                "RTL selection anchor must stay at the block end (head {target})"
            );
            assert!(block.selection_reversed);
        }
    });
}

#[gpui::test]
async fn typing_destination_into_empty_link_parens_keeps_caret_inside(cx: &mut TestAppContext) {
    // Fixes an edge case where batched auto-pair macro `()+Left` caused first character typed
    // into `()` of link to snap the caret past `)` with rest of URL landing outside the link.
    let block = cx.new(|cx| {
        Block::with_record(
            cx,
            BlockRecord::new(
                BlockKind::Paragraph,
                InlineTextTree::from_markdown("[GitHub]"),
            ),
        )
    });

    block.update(cx, |block, cx| {
        // Auto-pair the `()` after the label, then drop the caret between them.
        block.selected_range = 8..8;
        block.sync_inline_projection_for_focus(true);
        block.replace_text_in_visible_range(8..8, "()", None, false, cx);
        block.sync_inline_projection_for_focus(true);
        let between = block.display_text().find(')').expect("closing paren");
        block.selected_range = between..between;
        block.sync_inline_projection_for_focus(true);
        for ch in "https://github.com".chars() {
            let at = block.selected_range.clone();
            block.replace_text_in_visible_range(at, &ch.to_string(), None, false, cx);
            block.sync_inline_projection_for_focus(true);
        }
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(
            block.record.title.serialize_markdown(),
            "[GitHub](https://github.com)"
        );
        assert_eq!(
            block.inline_link_at(1).map(str::to_string),
            Some("https://github.com".to_string())
        );
        // Caret stays inside `()`, just before the closing `)`.
        let close = block.display_text().find(')').expect("closing paren");
        assert_eq!(block.selected_range, close..close);
    });
}
