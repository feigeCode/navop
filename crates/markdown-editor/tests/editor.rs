use gpui::{
    AppContext, Bounds, Modifiers, TestAppContext, VisualTestContext, WindowBounds, WindowOptions,
    point, px, size,
};
use gpui_component::{Root, highlighter::HighlightTheme, input::Position};
use markdown_editor::{MarkdownEditor, MarkdownEditorTheme};
use markdown_source::{BlockMoveDirection, SourceBlockKind, TableCellAddress};

#[gpui::test]
fn cursor_reveals_only_the_active_inline_source(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "Before _italic_ and **bold** after";
    let (window, editor) = cx.update(|cx| {
        let mut editor = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                let entity =
                    cx.new(|cx| MarkdownEditor::new(source, test_theme(), window, cx).unwrap());
                editor = Some(entity.clone());
                cx.new(|cx| Root::new(entity, window, cx))
            })
            .unwrap();
        (window, editor.unwrap())
    });
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.run_until_parked();
    assert_eq!(
        "Before italic and bold after",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
    );

    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 9), window, cx);
    });
    cx.run_until_parked();

    assert_eq!(
        "Before _italic_ and bold after",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
    );
    assert_eq!(
        source,
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn cursor_reveals_inline_code_source_until_another_node_becomes_active(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "Run `cargo test` then **ship**";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.run_until_parked();
    assert_eq!(
        "Run cargo test then ship",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
    );

    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 7), window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        "Run `cargo test` then ship",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
    );

    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 23), window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        "Run cargo test then **ship**",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
    );
    assert_eq!(
        source,
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn projected_edit_updates_only_inline_content_and_can_undo(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (window, editor) = cx.update(|cx| {
        let mut editor = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                let entity = cx.new(|cx| {
                    MarkdownEditor::new("Use _old_ here", test_theme(), window, cx).unwrap()
                });
                editor = Some(entity.clone());
                cx.new(|cx| Root::new(entity, window, cx))
            })
            .unwrap();
        (window, editor.unwrap())
    });
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_selected_range(4..7, false, window, cx);
    });
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(
            editor
                .edit_projected_value("Use _new_ here", window, cx)
                .unwrap()
        );
    });
    cx.run_until_parked();
    assert_eq!(
        "Use _new_ here",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.undo(window, cx).unwrap());
    });
    assert_eq!(
        "Use _old_ here",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
    assert_eq!(
        "old",
        editor.read_with(&cx, |editor, cx| {
            editor.input_state().read(cx).selected_text_string()
        })
    );
}

#[gpui::test]
fn unsafe_edit_across_hidden_markers_is_rejected(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (window, editor) = cx.update(|cx| {
        let mut editor = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                let entity = cx.new(|cx| {
                    MarkdownEditor::new("_one_ and [two](target)", test_theme(), window, cx)
                        .unwrap()
                });
                editor = Some(entity.clone());
                cx.new(|cx| Root::new(entity, window, cx))
            })
            .unwrap();
        (window, editor.unwrap())
    });
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.run_until_parked();
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(!editor.edit_projected_value("changed", window, cx).unwrap());
    });

    assert_eq!(
        "_one_ and [two](target)",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn source_undo_shortcut_routes_to_markdown_history(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        markdown_editor::init(cx);
    });
    let (window, editor) = cx.update(|cx| {
        let mut editor = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                let entity = cx.new(|cx| {
                    MarkdownEditor::new("Use _old_ here", test_theme(), window, cx).unwrap()
                });
                editor = Some(entity.clone());
                cx.new(|cx| Root::new(entity, window, cx))
            })
            .unwrap();
        (window, editor.unwrap())
    });
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    editor.update_in(&mut cx, |editor, window, cx| {
        editor.focus(window, cx);
    });
    cx.run_until_parked();
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(
            editor
                .edit_projected_value("Use new here", window, cx)
                .unwrap()
        );
    });

    #[cfg(target_os = "macos")]
    cx.simulate_keystrokes("cmd-z");
    #[cfg(not(target_os = "macos"))]
    cx.simulate_keystrokes("ctrl-z");
    cx.run_until_parked();

    assert_eq!(
        "Use _old_ here",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn source_mode_edit_and_undo_share_the_authoritative_history(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (window, editor) = open_editor("Before _old_ after", cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(
            editor
                .apply_source_value(
                    "Before _new_ after",
                    markdown_source::SourceSelection {
                        anchor: 12,
                        head: 12,
                    },
                    window,
                    cx,
                )
                .unwrap()
        );
        assert_eq!("Before _new_ after", editor.source());
        assert_eq!(
            Some(markdown_source::SourceSelection { anchor: 0, head: 0 }),
            editor.undo_source_mode(window, cx).unwrap()
        );
    });
    assert_eq!(
        "Before _old_ after",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn backspace_deletes_the_full_self_linked_image_source(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        markdown_editor::init(cx);
    });
    let source = "Before [![logo](logo.png)](logo.png) after";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 9), window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("backspace");
    cx.run_until_parked();
    assert_eq!(
        "Before  after",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn image_property_editor_updates_alt_and_path_without_losing_outer_link(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "Before [![Old](old.png)](outer.png) after";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 9), window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        Some(("Old".to_owned(), "old.png".to_owned())),
        editor.read_with(&cx, |editor, _| editor.active_image_properties())
    );
    editor.update_in(&mut cx, |editor, window, cx| {
        editor.set_active_image_property_values("New", "new.png", window, cx);
        assert!(editor.save_active_image_properties(window, cx).unwrap());
    });
    assert_eq!(
        "Before [![New](new.png)](outer.png) after",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn table_cell_edit_preserves_pipes_alignment_and_other_cells(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "| A | B |\n| :--- | ---: |\n| one | two |\n";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    let block_id = editor.read_with(&cx, |editor, _| {
        let document = markdown_source::SourceMarkdownDocument::parse(editor.source()).unwrap();
        assert!(matches!(document.blocks[0].kind, SourceBlockKind::Table(_)));
        document.blocks[0].id
    });
    editor.update_in(&mut cx, |editor, window, cx| {
        editor
            .edit_table_cell(
                TableCellAddress {
                    block_id,
                    row: 2,
                    column: 0,
                },
                "changed",
                window,
                cx,
            )
            .unwrap();
    });
    assert_eq!(
        "| A | B |\n| :--- | ---: |\n| changed | two |\n",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn clicking_a_table_cell_edits_only_its_mapped_content(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "| A | B |\n| :--- | ---: |\n| one | two |\n";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    let block_id = editor.read_with(&cx, |editor, _| {
        markdown_source::SourceMarkdownDocument::parse(editor.source())
            .unwrap()
            .blocks[0]
            .id
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let selector = Box::leak(format!("markdown-table-cell-{}-2-0", block_id.0).into_boxed_str());
    let bounds = cx
        .debug_bounds(selector)
        .expect("table cell must be rendered");
    cx.simulate_click(
        point(bounds.left() + px(4.), bounds.top() + px(4.)),
        Modifiers::none(),
    );
    assert_eq!(
        Some(TableCellAddress {
            block_id,
            row: 2,
            column: 0,
        }),
        editor.read_with(&cx, |editor, _| editor.active_table_cell())
    );
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.edit_projected_value("changed", window, cx).unwrap());
    });
    assert_eq!(
        "| A | B |\n| :--- | ---: |\n| changed | two |\n",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn clearing_an_active_table_cell_preserves_table_layout(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "| A | B |\n| :--- | ---: |\n| one | two |\n";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    let block_id = editor.read_with(&cx, |editor, _| {
        markdown_source::SourceMarkdownDocument::parse(editor.source())
            .unwrap()
            .blocks[0]
            .id
    });
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_table_cell(
            TableCellAddress {
                block_id,
                row: 2,
                column: 1,
            },
            window,
            cx,
        ));
        assert!(editor.clear_active_table_cell(window, cx).unwrap());
    });
    assert_eq!(
        "| A | B |\n| :--- | ---: |\n| one |  |\n",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn activating_a_block_edits_only_that_source_range(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "# Keep this\n\nUse _old_ here";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    let paragraph_id = editor.read_with(&cx, |editor, _| {
        markdown_source::SourceMarkdownDocument::parse(editor.source())
            .unwrap()
            .blocks[1]
            .id
    });
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(paragraph_id, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(
        "Use old here",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
    );

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(
            editor
                .edit_projected_value("Use new here", window, cx)
                .unwrap()
        );
    });
    assert_eq!(
        "# Keep this\n\nUse _new_ here",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn clicking_a_rendered_block_switches_only_that_block_to_editing(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "# Rendered heading\n\nParagraph";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    let heading_id = editor.read_with(&cx, |editor, _| {
        markdown_source::SourceMarkdownDocument::parse(editor.source())
            .unwrap()
            .blocks[0]
            .id
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds(Box::leak(
            format!("markdown-preview-block-{}", heading_id.0).into_boxed_str(),
        ))
        .expect("heading preview must be rendered");
    cx.simulate_click(
        point(bounds.left() + px(4.), bounds.top() + px(4.)),
        Modifiers::none(),
    );
    assert_eq!(
        Some(heading_id),
        editor.read_with(&cx, |editor, _| editor.active_block())
    );
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-active-block-{}", heading_id.0).into_boxed_str(),
        ))
        .is_some()
    );
    assert_eq!(
        "Rendered heading",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
    );
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-block-up-{}", heading_id.0).into_boxed_str(),
        ))
        .is_none()
    );
}

#[gpui::test]
fn long_documents_virtualize_blocks_and_reveal_the_activated_tail(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = (0..200)
        .map(|index| format!("Paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let source = Box::leak(source.into_boxed_str());
    let document = markdown_source::SourceMarkdownDocument::parse(&*source).unwrap();
    let first_id = document.blocks[0].id;
    let last_id = document.blocks.last().unwrap().id;
    let (window, editor) = open_editor_with_size(source, size(px(600.), px(260.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-preview-block-{}", first_id.0).into_boxed_str(),
        ))
        .is_some()
    );
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-preview-block-{}", last_id.0).into_boxed_str(),
        ))
        .is_none()
    );
    assert!(cx.debug_bounds("markdown-editor-scrollbar").is_some());

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(last_id, window, cx));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-active-block-{}", last_id.0).into_boxed_str(),
        ))
        .is_some()
    );
}

#[gpui::test]
fn raw_block_can_be_edited_without_rewriting_neighboring_blocks(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "# Keep\n\n::: custom\nold\n:::\n\nAfter";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    let raw_id = editor.read_with(&cx, |editor, _| {
        markdown_source::SourceMarkdownDocument::parse(editor.source())
            .unwrap()
            .blocks
            .iter()
            .find(|block| matches!(block.kind, SourceBlockKind::RawMarkdown))
            .unwrap()
            .id
    });
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(raw_id, window, cx));
    });
    cx.run_until_parked();
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(
            editor
                .edit_projected_value("::: custom\nnew\n:::", window, cx)
                .unwrap()
        );
    });
    assert_eq!(
        "# Keep\n\n::: custom\nnew\n:::\n\nAfter",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn active_block_structure_actions_are_source_transactions(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "first\n\nsecond";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    let second_id = editor.read_with(&cx, |_, _| {
        markdown_source::SourceMarkdownDocument::parse(source)
            .unwrap()
            .blocks[1]
            .id
    });
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(second_id, window, cx));
    });
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(
            editor
                .move_active_block(BlockMoveDirection::Up, window, cx)
                .unwrap()
        );
    });
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.toggle_active_blockquote(window, cx).unwrap());
    });
    assert_eq!(
        "> second\n\nfirst",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.undo(window, cx).unwrap());
        assert!(editor.undo(window, cx).unwrap());
    });
    assert_eq!(
        source,
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn enter_splits_an_ordered_list_with_the_next_source_marker(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        markdown_editor::init(cx);
    });
    let (window, editor) = open_editor("2. one", cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 6), window, cx);
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();

    assert_eq!(
        "2. one\n3. ",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn shift_enter_in_a_list_inserts_a_plain_newline(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        markdown_editor::init(cx);
    });
    let (window, editor) = open_editor("2. one", cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 6), window, cx);
    });
    cx.simulate_keystrokes("shift-enter");
    cx.run_until_parked();

    assert_eq!(
        "2. one\n",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn an_empty_document_renders_and_accepts_input(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (window, editor) = open_editor("", cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(cx.debug_bounds("markdown-empty-document").is_some());

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.edit_projected_value("Hello", window, cx).unwrap());
    });
    assert_eq!(
        "Hello",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn markdown_format_shortcuts_apply_source_transactions(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        markdown_editor::init(cx);
    });
    let (window, editor) = open_editor("format me", cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_selected_range(0..6, false, window, cx);
    });
    cx.simulate_keystrokes("secondary-b");
    cx.run_until_parked();
    assert_eq!(
        "**format** me",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn markdown_block_shortcuts_change_heading_and_move_blocks(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        markdown_editor::init(cx);
    });
    let source = "first\n\nsecond";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    let second = markdown_source::SourceMarkdownDocument::parse(source)
        .unwrap()
        .blocks[1]
        .id;
    editor.update_in(&mut cx, |editor, window, cx| {
        editor.activate_block(second, window, cx);
    });
    cx.simulate_keystrokes("secondary-2");
    cx.run_until_parked();
    assert_eq!(
        "first\n\n## second",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
    cx.simulate_keystrokes("secondary-shift-up");
    cx.run_until_parked();
    assert_eq!(
        "## second\n\nfirst",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

fn open_editor(
    source: &'static str,
    cx: &mut TestAppContext,
) -> (gpui::AnyWindowHandle, gpui::Entity<MarkdownEditor>) {
    cx.update(|cx| {
        let mut editor = None;
        let window = cx
            .open_window(WindowOptions::default(), |window, cx| {
                let entity =
                    cx.new(|cx| MarkdownEditor::new(source, test_theme(), window, cx).unwrap());
                editor = Some(entity.clone());
                cx.new(|cx| Root::new(entity, window, cx))
            })
            .unwrap();
        (window.into(), editor.unwrap())
    })
}

fn open_editor_with_size(
    source: &'static str,
    window_size: gpui::Size<gpui::Pixels>,
    cx: &mut TestAppContext,
) -> (gpui::AnyWindowHandle, gpui::Entity<MarkdownEditor>) {
    cx.update(|cx| {
        let mut editor = None;
        let window_bounds = Bounds::centered(None, window_size, cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(window_bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    let entity =
                        cx.new(|cx| MarkdownEditor::new(source, test_theme(), window, cx).unwrap());
                    editor = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                },
            )
            .unwrap();
        (window.into(), editor.unwrap())
    })
}

fn test_theme() -> MarkdownEditorTheme {
    MarkdownEditorTheme {
        background: gpui::rgb(0x111111).into(),
        foreground: gpui::rgb(0xffffff).into(),
        muted_foreground: gpui::rgb(0x999999).into(),
        border: gpui::rgb(0x333333).into(),
        primary: gpui::rgb(0x4488ff).into(),
        highlight_theme: HighlightTheme::default_dark(),
    }
}
