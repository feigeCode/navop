use gpui::{
    AppContext, Bounds, Modifiers, TestAppContext, VisualTestContext, WindowBounds, WindowOptions,
    point, px, size,
};
use gpui_component::{Root, highlighter::HighlightTheme, input::Position};
use markdown_editor::{MarkdownEditor, MarkdownEditorTheme};
use markdown_source::{
    BlockMoveDirection, SourceBlockKind, SourceSelection, TableCellAddress, TableInsertPosition,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[gpui::test]
fn cursor_reveals_only_the_active_inline_source(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "Before _italic_ and **bold** after";
    let italic_id = markdown_source::SourceMarkdownDocument::parse(source)
        .unwrap()
        .inline_node_at(source.find("italic").unwrap())
        .unwrap()
        .id;
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
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-active-inline-markers-{}", italic_id.0).into_boxed_str(),
        ))
        .is_none()
    );
    assert_eq!(
        source,
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn nested_inline_edit_reveals_only_the_innermost_source(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "Before **bold _nested_ text** after";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 13), window, cx);
    });
    cx.run_until_parked();

    assert_eq!(
        "Before bold _nested_ text after",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
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

    let ship = editor.read_with(&cx, |editor, _| {
        editor.projected_text().find("ship").unwrap() + 1
    });
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, ship as u32), window, cx);
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
fn real_inline_markers_reflow_without_overlay_or_inner_scroll(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "This deliberately long paragraph approaches the editor wrap boundary before a **strong inline node** near the end.\n\nFollowing block";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let first_id = document.blocks[0].id;
    let following_id = document.blocks[1].id;
    let strong_id = document
        .inline_node_at(source.find("strong").unwrap())
        .unwrap()
        .id;
    let (window, editor) = open_editor_with_size(source, size(px(560.), px(400.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(first_id, window, cx));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let first_selector = Box::leak(format!("markdown-block-frame-{}", first_id.0).into_boxed_str());
    let following_selector =
        Box::leak(format!("markdown-block-frame-{}", following_id.0).into_boxed_str());
    let scroll_before = editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset());
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    let strong_display = editor.read_with(&cx, |editor, _| {
        editor.projected_text().find("strong").unwrap() + 2
    });
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, strong_display as u32), window, cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let first = cx.debug_bounds(first_selector).unwrap();
    let following = cx.debug_bounds(following_selector).unwrap();
    assert!(following.top() >= first.bottom());
    assert_eq!(
        scroll_before,
        editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset())
    );
    assert!(cx.debug_bounds("editor-scrollbar").is_none());
    assert!(
        editor
            .read_with(&cx, |editor, _| editor.projected_text().to_owned())
            .contains("**strong inline node**")
    );
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-active-inline-markers-{}", strong_id.0).into_boxed_str(),
        ))
        .is_none()
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
fn stale_source_selection_inside_a_multibyte_char_does_not_panic(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (window, editor) = open_editor("第一段新内容\n\n第二段", cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| {
        // '新' occupies bytes 9..12; a stale source-mode cursor at byte 10 lands
        // inside the character and must snap to a boundary instead of panicking
        // while the active block is synced.
        let value = "第一段新内容\n\n第二段v2";
        assert!(value.get(10..11).is_none());
        let selection = SourceSelection {
            anchor: 10,
            head: 10,
        };
        assert!(
            editor
                .apply_source_value(value, selection, window, cx)
                .unwrap()
        );
    });
    cx.run_until_parked();
    let (cursor, text) = editor.read_with(&cx, |editor, cx| {
        (
            editor.input_state().read(cx).selected_range().end,
            editor.projected_text().to_owned(),
        )
    });
    assert!(
        text.is_char_boundary(cursor),
        "synced cursor {cursor} must stay on a char boundary of {text:?}"
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
    let surface_selector = Box::leak(
        format!("markdown-table-cell-edit-surface-{}-2-0", block_id.0).into_boxed_str(),
    );
    let surface_before = cx
        .debug_bounds(surface_selector)
        .expect("table cell edit surface must be mounted before activation");
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-table-cell-input-slot-{}-2-0", block_id.0).into_boxed_str(),
        ))
        .is_some(),
        "table cell input must be mounted before activation"
    );
    cx.simulate_click(
        point(bounds.left() + px(4.), bounds.top() + px(4.)),
        Modifiers::none(),
    );
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert_eq!(
        Some(TableCellAddress {
            block_id,
            row: 2,
            column: 0,
        }),
        editor.read_with(&cx, |editor, _| editor.active_table_cell())
    );
    assert!(
        cx.debug_bounds("markdown-active-table-input-slot").is_some(),
        "activation must expose the already-mounted table cell input"
    );
    assert_eq!(
        surface_before,
        cx.debug_bounds(surface_selector)
            .expect("table cell edit surface must remain mounted after activation")
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
fn wrapped_table_rows_push_following_content_below_the_table(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = concat!(
        "# 接口清单\n\n",
        "| 接口 | 当前用途 | 处理结论 |\n",
        "| --- | --- | --- |\n",
        "| POST /ai-manager/dashboard/publish | 从服务器本地绝对路径发布看板 | 保留当前用户的默认看板，不允许普通浏览器直接调用 |\n",
        "| POST /ai-manager/dashboard/save/resolve | 解决同名冲突：覆盖或另存为新看板 | 复用并增强权限、默认项、版本和修改时间字段 |\n\n",
        "## 后续标题\n\n后续正文",
    );
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let table_id = document.blocks[1].id;
    let following_id = document.blocks[2].id;
    let (window, _editor) = open_editor_with_size(source, size(px(760.), px(700.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let table = cx
        .debug_bounds(Box::leak(
            format!("markdown-table-{}", table_id.0).into_boxed_str(),
        ))
        .unwrap();
    let following = cx
        .debug_bounds(Box::leak(
            format!("markdown-preview-block-{}", following_id.0).into_boxed_str(),
        ))
        .unwrap();
    assert!(following.top() >= table.bottom());
}

#[gpui::test]
fn active_table_cell_keeps_row_height_and_has_no_clear_overlay(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "| 接口 | 说明 |\n| --- | --- |\n| POST /ai-manager/dashboard/publish | 这是需要换行的很长中文说明，不允许普通浏览器直接调用 |\n";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let block_id = document.blocks[0].id;
    let address = TableCellAddress {
        block_id,
        row: 2,
        column: 0,
    };
    let (window, editor) = open_editor_with_size(source, size(px(620.), px(420.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let selector = Box::leak(format!("markdown-table-cell-{}-2-0", block_id.0).into_boxed_str());
    let before = cx.debug_bounds(selector).unwrap();
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_table_cell(address, window, cx));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    let text_bounds = input
        .read_with(&cx, |input, _| input.laid_out_text_bounds())
        .expect("active table input must be laid out");
    assert!(
        text_bounds.size.width >= px(240.),
        "active table input width collapsed to {:?}",
        text_bounds.size.width
    );
    assert_eq!(
        before.size.height,
        cx.debug_bounds(selector).unwrap().size.height
    );
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-table-clear-{}-2-0", block_id.0).into_boxed_str(),
        ))
        .is_none()
    );
}

#[gpui::test]
fn activating_a_table_cell_places_the_cursor_at_the_content_end(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "| Name | Value |\n| --- | --- |\n| first | editable content |";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let address = TableCellAddress {
        block_id: document.blocks[0].id,
        row: 2,
        column: 1,
    };
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_table_cell(address, window, cx));
    });
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());

    assert_eq!(
        "editable content".len().."editable content".len(),
        input.read_with(&cx, |input, _| input.selected_range())
    );
}

#[gpui::test]
fn clicking_a_table_cell_places_the_cursor_near_the_clicked_text(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "| Name | Value |\n| --- | --- |\n| first | editable content |";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let block_id = document.blocks[0].id;
    let (window, editor) = open_editor_with_size(source, size(px(700.), px(360.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let cell = cx
        .debug_bounds(Box::leak(
            format!("markdown-table-cell-{}-2-1", block_id.0).into_boxed_str(),
        ))
        .unwrap();
    cx.simulate_click(
        point(cell.right() - px(22.), cell.center().y),
        Modifiers::none(),
    );
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    let cursor = input.read_with(&cx, |input, _| input.selected_range().end);

    assert!(cursor > "editable".len());
}

#[gpui::test]
fn active_table_toolbar_is_an_overlay_and_structure_edits_are_undoable(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "# Heading\n\n| Name | Value |\n| --- | --- |\n| first | one |\n\nFollowing";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let table_id = document.blocks[1].id;
    let following_id = document.blocks[2].id;
    let address = TableCellAddress {
        block_id: table_id,
        row: 2,
        column: 1,
    };
    let (window, editor) = open_editor_with_size(source, size(px(760.), px(520.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let table_selector = Box::leak(format!("markdown-table-{}", table_id.0).into_boxed_str());
    let following_selector =
        Box::leak(format!("markdown-preview-block-{}", following_id.0).into_boxed_str());
    let table_before = cx.debug_bounds(table_selector).unwrap();
    let following_before = cx.debug_bounds(following_selector).unwrap();

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_table_cell(address, window, cx));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-table-toolbar-{}", table_id.0).into_boxed_str(),
        ))
        .is_some()
    );
    let table_after = cx.debug_bounds(table_selector).unwrap();
    let following_after = cx.debug_bounds(following_selector).unwrap();
    assert_eq!(table_before.origin, table_after.origin);
    assert!(following_after.top() >= table_after.bottom());
    assert!((following_after.top() - following_before.top()).abs() <= px(24.));

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(
            editor
                .insert_active_table_row(TableInsertPosition::After, window, cx)
                .unwrap()
        );
    });
    assert!(editor.read_with(&cx, |editor, _| editor.source().contains("|  |  |")));
    assert!(editor.read_with(&cx, |editor, _| editor.active_table_cell().is_some()));
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.undo(window, cx).unwrap());
    });
    assert_eq!(
        source,
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn table_grid_picker_highlights_rectangle_and_click_resizes_real_table(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "| A | B |\n| :--- | ---: |\n| one | two |";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let table_id = document.blocks[0].id;
    let address = TableCellAddress {
        block_id: table_id,
        row: 2,
        column: 0,
    };
    let (window, editor) = open_editor_with_size(source, size(px(760.), px(520.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_table_cell(address, window, cx));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let trigger = cx
        .debug_bounds("markdown-table-grid-trigger")
        .expect("table size trigger must be visible");
    cx.simulate_click(trigger.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("markdown-table-grid-label-empty").is_some(),
        "picker must show the placeholder label before any hover"
    );
    let target = cx
        .debug_bounds("markdown-table-size-4-5")
        .expect("6×6 picker must expose the 5×4 cell");
    cx.simulate_mouse_move(target.center(), None, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        Some((4, 5)),
        editor.read_with(&cx, |editor, _| editor.table_grid_hover())
    );
    assert!(
        cx.debug_bounds("markdown-table-grid-label-5x4").is_some(),
        "hover must show the `5 × 4` size label"
    );
    assert!(cx.debug_bounds("markdown-table-grid-label-empty").is_none());
    for (rows, columns) in [(1, 1), (4, 5)] {
        assert!(
            cx.debug_bounds(Box::leak(
                format!("markdown-table-size-{rows}-{columns}").into_boxed_str(),
            ))
            .is_some()
        );
    }
    cx.simulate_click(target.center(), Modifiers::none());
    cx.run_until_parked();

    let resized = editor.read_with(&cx, |editor, _| editor.source().to_owned());
    let table = match &markdown_source::SourceMarkdownDocument::parse(resized)
        .unwrap()
        .blocks[0]
        .kind
    {
        SourceBlockKind::Table(table) => table.clone(),
        _ => panic!("resized block must remain a table"),
    };
    assert_eq!(5, table.rows[0].cells.len());
    assert_eq!(5, table.rows.len(), "header + delimiter + 3 body rows");
    assert_eq!(
        None,
        editor.read_with(&cx, |editor, _| editor.table_grid_hover())
    );
}

#[gpui::test]
fn active_table_alignment_button_reflects_current_column(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "| A | B |\n| :--- | ---: |\n| one | two |";
    let table_id = markdown_source::SourceMarkdownDocument::parse(source)
        .unwrap()
        .blocks[0]
        .id;
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_table_cell(
            TableCellAddress {
                block_id: table_id,
                row: 2,
                column: 1,
            },
            window,
            cx,
        ));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    assert!(cx.debug_bounds("table-align-right").is_some());
    let right = cx.debug_bounds("table-align-right").unwrap();
    cx.simulate_click(right.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(editor.read_with(&cx, |editor, _| editor.source().contains("---:")));
}

#[gpui::test]
fn virtualized_wrapped_table_updates_its_item_height_before_following_content(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_component::init);
    let table = concat!(
        "| 接口 | 当前用途 | 处理结论 |\n",
        "| --- | --- | --- |\n",
        "| POST /ai-manager/dashboard/publish | 从服务器本地绝对路径发布看板 | 保留当前用户的默认看板，不允许普通浏览器直接调用 |\n",
        "| POST /ai-manager/dashboard/save/resolve | 解决同名冲突：覆盖或另存为新看板 | 复用并增强权限、默认项、版本和修改时间字段 |",
    );
    let source = format!(
        "# 接口清单\n\n{table}\n\n## 后续标题\n\n{}",
        (0..90)
            .map(|index| format!("尾部段落 {index}"))
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    let source = Box::leak(source.into_boxed_str());
    let document = markdown_source::SourceMarkdownDocument::parse(&*source).unwrap();
    let table_id = document.blocks[1].id;
    let following_id = document.blocks[2].id;
    let (window, editor) = open_editor_with_size(source, size(px(760.), px(700.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    assert!(editor.read_with(&cx, |editor, _| editor.uses_virtual_layout()));
    let table = cx
        .debug_bounds(Box::leak(
            format!("markdown-table-{}", table_id.0).into_boxed_str(),
        ))
        .unwrap();
    let following = cx
        .debug_bounds(Box::leak(
            format!("markdown-preview-block-{}", following_id.0).into_boxed_str(),
        ))
        .unwrap();
    assert!(following.top() >= table.bottom());
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
    let first = cx
        .debug_bounds(Box::leak(
            format!("markdown-preview-block-{}", first_id.0).into_boxed_str(),
        ))
        .expect("first virtual block must render");
    assert!(first.origin.y >= px(36.));
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-preview-block-{}", last_id.0).into_boxed_str(),
        ))
        .is_none()
    );
    assert!(cx.debug_bounds("markdown-editor-scrollbar").is_some());
    assert!(editor.read_with(&cx, |editor, _| editor.uses_virtual_layout()));
    assert_ne!(
        px(0.),
        editor.read_with(&cx, |editor, _| editor.vertical_scroll_range())
    );
    let initial_offset = editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset());

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
    assert_ne!(
        initial_offset,
        editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset())
    );
}

#[gpui::test]
fn clicking_a_visible_virtual_list_keeps_the_document_scroll_offset(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = (0..200)
        .map(|index| {
            if index == 101 {
                "- First item\n- Second item".to_owned()
            } else {
                format!("Paragraph {index}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let source = Box::leak(source.into_boxed_str());
    let document = markdown_source::SourceMarkdownDocument::parse(&*source).unwrap();
    let centered_id = document.blocks[100].id;
    let list_id = document.blocks[101].id;
    let following_id = document.blocks[102].id;
    let (window, editor) = open_editor_with_size(source, size(px(600.), px(260.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(centered_id, window, cx));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    editor.update_in(&mut cx, |editor, window, cx| {
        editor.deactivate_block(window, cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let preview = cx
        .debug_bounds(Box::leak(
            format!("markdown-preview-block-{}", list_id.0).into_boxed_str(),
        ))
        .expect("the list next to the centered block must be visible");
    let before = editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset());
    cx.simulate_click(
        point(preview.left() + px(80.), preview.top() + px(12.)),
        Modifiers::none(),
    );
    cx.run_until_parked();
    let after = editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset());
    let active_frame = cx
        .debug_bounds(Box::leak(
            format!("markdown-block-frame-{}", list_id.0).into_boxed_str(),
        ))
        .expect("active list frame must remain visible");
    let following_frame = cx
        .debug_bounds(Box::leak(
            format!("markdown-block-frame-{}", following_id.0).into_boxed_str(),
        ))
        .expect("the block after the active list must remain visible");
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let settled_offset = editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset());
    let settled_active_frame = cx
        .debug_bounds(Box::leak(
            format!("markdown-block-frame-{}", list_id.0).into_boxed_str(),
        ))
        .expect("active list frame must remain visible after settling");
    let settled_following_frame = cx
        .debug_bounds(Box::leak(
            format!("markdown-block-frame-{}", following_id.0).into_boxed_str(),
        ))
        .expect("following block must remain visible after settling");

    assert_eq!(
        Some(list_id),
        editor.read_with(&cx, |editor, _| editor.active_block())
    );
    assert_eq!(before, after);
    assert_eq!(after, settled_offset);
    assert_eq!(active_frame.size.height, settled_active_frame.size.height);
    assert_eq!(following_frame.top(), settled_following_frame.top());
}

#[gpui::test]
fn standard_document_scrollbar_track_changes_the_real_offset(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = (0..40)
        .map(|index| format!("Paragraph {index}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let source = Box::leak(source.into_boxed_str());
    let (window, editor) = open_editor_with_size(source, size(px(600.), px(260.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    assert!(!editor.read_with(&cx, |editor, _| editor.uses_virtual_layout()));
    let scrollbar = cx.debug_bounds("markdown-editor-scrollbar").unwrap();
    let before = editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset());
    cx.simulate_click(
        point(
            scrollbar.right() - px(4.),
            scrollbar.top() + scrollbar.size.height * 0.8,
        ),
        Modifiers::none(),
    );
    cx.run_until_parked();
    let after = editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset());

    assert_ne!(before, after);
}

#[gpui::test]
fn clicking_a_visible_standard_list_keeps_the_document_scroll_offset(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = (0..31)
        .map(|index| {
            if index % 3 == 0 {
                format!("- First item {index}\n- Second item {index}")
            } else {
                format!("Paragraph {index}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let source = Box::leak(source.into_boxed_str());
    let document = markdown_source::SourceMarkdownDocument::parse(&*source).unwrap();
    let (window, editor) = open_editor_with_size(source, size(px(600.), px(260.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let scrollbar = cx.debug_bounds("markdown-editor-scrollbar").unwrap();
    cx.simulate_click(
        point(scrollbar.right() - px(4.), scrollbar.center().y),
        Modifiers::none(),
    );
    cx.run_until_parked();
    let (list_id, preview) = document
        .blocks
        .iter()
        .filter(|block| matches!(block.kind, SourceBlockKind::UnorderedList))
        .filter_map(|block| {
            cx.debug_bounds(Box::leak(
                format!("markdown-preview-block-{}", block.id.0).into_boxed_str(),
            ))
            .filter(|bounds| bounds.top() >= px(0.) && bounds.bottom() <= px(260.))
            .map(|bounds| (block.id, bounds))
        })
        .next()
        .expect("a list must be visible after scrolling");
    let before = editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset());
    cx.simulate_click(
        point(preview.left() + px(80.), preview.top() + px(12.)),
        Modifiers::none(),
    );
    cx.run_until_parked();
    let after = editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset());

    assert_eq!(
        Some(list_id),
        editor.read_with(&cx, |editor, _| editor.active_block())
    );
    assert_eq!(before, after);
}

#[gpui::test]
fn very_tall_documents_virtualize_before_the_block_count_threshold(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = (0..12)
        .map(|index| {
            (0..50)
                .map(|line| format!("Paragraph {index}, line {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let source = Box::leak(source.into_boxed_str());
    let (window, editor) = open_editor_with_size(source, size(px(600.), px(260.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    assert!(editor.read_with(&cx, |editor, _| editor.uses_virtual_layout()));
    assert_ne!(
        px(0.),
        editor.read_with(&cx, |editor, _| editor.vertical_scroll_range())
    );
}

#[gpui::test]
fn marker_reveal_shifts_following_blocks_only_by_content_height(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    // 84 ASCII chars put the hidden text exactly at the wrap boundary of a
    // 560px window, so revealing the four `**` marker chars adds one line.
    let source = format!("{} **strong node**\n\nFollowing block", "x".repeat(84));
    let source: &'static str = Box::leak(source.into_boxed_str());
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let first_id = document.blocks[0].id;
    let following_id = document.blocks[1].id;
    let (window, editor) = open_editor_with_size(source, size(px(560.), px(400.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let first_selector = Box::leak(format!("markdown-block-frame-{}", first_id.0).into_boxed_str());
    let following_selector =
        Box::leak(format!("markdown-block-frame-{}", following_id.0).into_boxed_str());

    // Preview baseline: fonts, wrapping and block offsets before any editing.
    let preview_first = cx.debug_bounds(first_selector).unwrap();
    let preview_following = cx.debug_bounds(following_selector).unwrap();
    let preview_scroll = editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset());

    // Activate with the cursor outside the strong node: markers stay hidden and
    // the layout must match the preview exactly.
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(first_id, window, cx));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let hidden_first = cx.debug_bounds(first_selector).unwrap();
    let hidden_following = cx.debug_bounds(following_selector).unwrap();
    assert_eq!(preview_first.size.height, hidden_first.size.height);
    assert_eq!(preview_following.origin.y, hidden_following.origin.y);
    assert_eq!(
        preview_scroll,
        editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset())
    );
    assert!(
        !editor
            .read_with(&cx, |editor, _| editor.projected_text().to_owned())
            .contains("**")
    );

    // Reveal the strong markers: the following block may only shift by the
    // exact content-height delta of the active input.
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    let strong_display = editor.read_with(&cx, |editor, _| {
        editor.projected_text().find("strong").unwrap() + 2
    });
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, strong_display as u32), window, cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(
        editor
            .read_with(&cx, |editor, _| editor.projected_text().to_owned())
            .contains("**strong node**")
    );
    let revealed_first = cx.debug_bounds(first_selector).unwrap();
    let revealed_following = cx.debug_bounds(following_selector).unwrap();
    assert_eq!(
        revealed_following.origin.y - hidden_following.origin.y,
        revealed_first.size.height - hidden_first.size.height,
        "marker reveal must shift following blocks by exactly the input height delta"
    );
    assert_eq!(
        px(24.),
        revealed_first.size.height - hidden_first.size.height,
        "revealing four marker chars must add exactly one 24px line"
    );
    assert_eq!(
        preview_scroll,
        editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset())
    );

    // Hide the markers again: the layout returns to the pre-reveal baseline.
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 0), window, cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let rehidden_first = cx.debug_bounds(first_selector).unwrap();
    let rehidden_following = cx.debug_bounds(following_selector).unwrap();
    assert_eq!(hidden_first.size.height, rehidden_first.size.height);
    assert_eq!(hidden_following.origin.y, rehidden_following.origin.y);
    assert_eq!(
        preview_scroll,
        editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset())
    );
}

#[gpui::test]
fn activating_a_block_keeps_following_blocks_stable(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "# Heading\n\nFirst paragraph\n\nFollowing paragraph";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let first_id = document.blocks[1].id;
    let following_id = document.blocks[2].id;
    let (window, editor) = open_editor_with_size(source, size(px(1000.), px(600.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let frame = Box::leak(format!("markdown-block-frame-{}", first_id.0).into_boxed_str());
    let frame_before = cx.debug_bounds(frame).unwrap();
    let selector = Box::leak(format!("markdown-preview-block-{}", following_id.0).into_boxed_str());
    let before = cx.debug_bounds(selector).unwrap();
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(first_id, window, cx));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let after = cx.debug_bounds(selector).unwrap();
    assert_eq!(before.origin.y, after.origin.y);
    assert_eq!(frame_before, cx.debug_bounds(frame).unwrap());
}

#[gpui::test]
fn standard_document_uses_natural_preview_height_without_estimated_blank_space(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_component::init);
    let source = "First paragraph\n\nSecond paragraph";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let (window, _editor) = open_editor_with_size(source, size(px(900.), px(500.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let first = cx
        .debug_bounds(Box::leak(
            format!("markdown-block-frame-{}", document.blocks[0].id.0).into_boxed_str(),
        ))
        .unwrap();

    assert!(first.size.height < px(30.));
}

#[gpui::test]
fn activating_structured_blocks_uses_content_driven_height_and_stable_markers(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_component::init);
    let source = "- First **item**\n- Second item\n\n> Quoted text\n\n# Heading\n\nFollowing";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let following_id = document.blocks[3].id;
    let structured_ids = document.blocks[..3]
        .iter()
        .map(|block| block.id)
        .collect::<Vec<_>>();
    let (window, editor) = open_editor_with_size(source, size(px(1000.), px(700.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let following =
        Box::leak(format!("markdown-preview-block-{}", following_id.0).into_boxed_str());
    for block_id in structured_ids {
        let frame = Box::leak(format!("markdown-block-frame-{}", block_id.0).into_boxed_str());
        editor.update_in(&mut cx, |editor, window, cx| {
            assert!(editor.activate_block(block_id, window, cx));
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let active = cx.debug_bounds(frame).unwrap();
        let next = cx.debug_bounds(following).unwrap();
        assert!(active.size.height >= px(24.));
        assert!(next.top() >= active.bottom());
        assert!(cx.debug_bounds("editor-scrollbar").is_none());
    }
    let list_id = document.blocks[0].id;
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-active-list-markers-{}", list_id.0).into_boxed_str(),
        ))
        .is_none()
    );
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(list_id, window, cx));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-active-list-markers-{}", list_id.0).into_boxed_str(),
        ))
        .is_some()
    );
    assert!(
        !editor
            .read_with(&cx, |editor, _| editor.projected_text().to_owned())
            .contains("- ")
    );
}

#[gpui::test]
fn clicking_a_list_item_places_the_cursor_on_that_visual_line(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "- First item\n- Second item";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let list_id = document.blocks[0].id;
    let (window, editor) = open_editor_with_size(source, size(px(800.), px(400.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let input_slot = cx
        .debug_bounds(Box::leak(
            format!("markdown-block-input-slot-{}", list_id.0).into_boxed_str(),
        ))
        .expect("the list input must be mounted before activation");
    cx.simulate_click(
        point(input_slot.left() + px(4.), input_slot.top() + px(30.)),
        Modifiers::none(),
    );
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    assert_eq!(
        11..11,
        input.read_with(&cx, |input, _| input.selected_range())
    );
    assert!(cx.debug_bounds("editor-scrollbar").is_none());
}

#[gpui::test]
fn ordered_and_task_lists_keep_visual_markers_while_editing(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "9. Nine\n10. Ten\n\n- [ ] Todo\n- [x] Done\n\nFollowing";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let (window, editor) = open_editor_with_size(source, size(px(800.), px(500.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    for block in &document.blocks[..2] {
        let frame = Box::leak(format!("markdown-block-frame-{}", block.id.0).into_boxed_str());
        editor.update_in(&mut cx, |editor, window, cx| {
            assert!(editor.activate_block(block.id, window, cx));
        });
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(cx.debug_bounds(frame).unwrap().size.height >= px(48.));
        assert!(
            cx.debug_bounds(Box::leak(
                format!("markdown-active-list-markers-{}", block.id.0).into_boxed_str(),
            ))
            .is_some()
        );
    }

    assert_eq!(
        "Todo\nDone",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
    );
}

#[gpui::test]
fn active_task_list_lays_out_every_marker_anchor(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "- First\n- [ ] Todo\n- [x] Done";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let list_id = document.blocks[0].id;
    let (window, editor) = open_editor_with_size(source, size(px(800.), px(400.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(list_id, window, cx));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    let text_bounds = input
        .read_with(&cx, |input, _| input.laid_out_text_bounds())
        .expect("active list input must be laid out");
    assert!(
        text_bounds.size.width >= px(640.),
        "active list input width collapsed to {:?}",
        text_bounds.size.width
    );
    let overlay = cx
        .debug_bounds(Box::leak(
            format!("markdown-active-list-markers-{}", list_id.0).into_boxed_str(),
        ))
        .unwrap();

    let mut anchors = Vec::new();
    for offset in [0, 6, 11] {
        let anchor = input
            .read_with(&cx, |input, _| input.range_to_bounds(&(offset..offset)))
            .unwrap_or_else(|| panic!("marker at display offset {offset} must be laid out"));
        assert!(overlay.top() <= anchor.top());
        assert!(overlay.bottom() >= anchor.bottom());
        anchors.push(anchor);
    }
    assert!((anchors[0].left() - anchors[1].left()).abs() <= px(1.));
    assert!((anchors[1].left() - anchors[2].left()).abs() <= px(1.));
    assert!(anchors[0].top() < anchors[1].top());
    assert!(anchors[1].top() < anchors[2].top());
}

#[gpui::test]
fn clicking_code_content_maps_to_content_lines_not_fence_lines(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "```rust\nfirst();\nsecond();\n```";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let code_id = document.blocks[0].id;
    let (window, editor) = open_editor_with_size(source, size(px(800.), px(400.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let input_slot = cx
        .debug_bounds(Box::leak(
            format!("markdown-block-input-slot-{}", code_id.0).into_boxed_str(),
        ))
        .expect("the code input must be mounted before activation");
    cx.simulate_click(
        point(input_slot.left() + px(4.), input_slot.top() + px(30.)),
        Modifiers::none(),
    );
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    assert_eq!(
        9..9,
        input.read_with(&cx, |input, _| input.selected_range())
    );
}

#[gpui::test]
fn markdown_document_uses_a_centered_reading_column(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (window, _) = open_editor_with_size(
        "A comfortably sized reading column",
        size(px(1400.), px(700.)),
        cx,
    );
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let column = cx.debug_bounds("markdown-document-column").unwrap();
    assert!(column.size.width <= px(860.));
    assert!(column.origin.x > px(200.));
}

#[gpui::test]
fn fenced_code_uses_code_editor_input_and_preserves_newlines(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        markdown_editor::init(cx);
    });
    let source = "Before\n\n```rust\nfn main() {}\n```\n\nAfter";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let code_id = document.blocks[1].id;
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(code_id, window, cx));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 6), window, cx);
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let expected = format!(
        "Before\n\n{ticks}rust\nfn mai\nn() {{}}\n{ticks}\n\nAfter",
        ticks = char::from(96).to_string().repeat(3),
    );
    assert_eq!(
        expected,
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn fenced_code_supports_tab_indentation(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        markdown_editor::init(cx);
    });
    let source = "```rust\nlet value = 1;\n```";
    let code_id = markdown_source::SourceMarkdownDocument::parse(source)
        .unwrap()
        .blocks[0]
        .id;
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(code_id, window, cx));
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(
        "```rust\n  let value = 1;\n```",
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 4), window, cx);
    });
    cx.simulate_keystrokes("shift-tab");
    cx.run_until_parked();
    assert_eq!(
        source,
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
}

#[gpui::test]
fn mermaid_preview_uses_async_svg_renderer_and_opens_source_on_click(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "```mermaid\ngraph TD\nA --> B\n```";
    let block_id = markdown_source::SourceMarkdownDocument::parse(source)
        .unwrap()
        .blocks[0]
        .id;
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, _, cx| {
        editor.set_block_render_provider(Some(svg_render_provider()), cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let selector = Box::leak(format!("markdown-rendered-block-{}", block_id.0).into_boxed_str());
    assert!(cx.debug_bounds(selector).is_some());

    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(block_id, window, cx));
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-active-placeholder-{}", block_id.0).into_boxed_str(),
        ))
        .is_none()
    );
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-active-block-{}", block_id.0).into_boxed_str(),
        ))
        .is_some()
    );
}

#[gpui::test]
fn clicking_an_artifact_activates_its_mounted_input_without_moving_content(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_component::init);
    let source = "```mermaid\ngraph TD\nA --> B\n```\n\nFollowing paragraph";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let artifact_id = document.blocks[0].id;
    let following_id = document.blocks[1].id;
    let (window, editor) = open_editor_with_size(source, size(px(760.), px(520.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, _, cx| {
        editor.set_block_render_provider(Some(svg_render_provider_with_height(180.)), cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let shell_selector =
        Box::leak(format!("markdown-artifact-shell-{}", artifact_id.0).into_boxed_str());
    let rendered_layer_selector =
        Box::leak(format!("markdown-artifact-rendered-layer-{}", artifact_id.0).into_boxed_str());
    let rendered_output_selector =
        Box::leak(format!("markdown-rendered-block-{}", artifact_id.0).into_boxed_str());
    let rendered_image_selector =
        Box::leak(format!("markdown-rendered-image-bounds-{}", artifact_id.0).into_boxed_str());
    let input_slot_selector =
        Box::leak(format!("markdown-block-input-slot-{}", artifact_id.0).into_boxed_str());
    let following_selector =
        Box::leak(format!("markdown-block-frame-{}", following_id.0).into_boxed_str());

    let shell_before = cx
        .debug_bounds(shell_selector)
        .expect("artifact must render through a stable shell");
    let rendered_layer = cx
        .debug_bounds(rendered_layer_selector)
        .expect("inactive artifact render layer must be visible");
    let rendered_output = cx
        .debug_bounds(rendered_output_selector)
        .expect("inactive artifact output must be visible");
    let rendered_image = cx
        .debug_bounds(rendered_image_selector)
        .expect("inactive artifact image bounds must be laid out");
    assert!(
        cx.debug_bounds(input_slot_selector).is_some(),
        "artifact input must be laid out before its first activation"
    );
    let following_before = cx.debug_bounds(following_selector).unwrap();
    let scroll_before = editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset());
    assert!(
        rendered_output.bottom() <= shell_before.bottom(),
        "artifact output must be contained by its shell: shell={shell_before:?}, output={rendered_output:?}"
    );
    assert!(
        rendered_image.bottom() <= rendered_output.bottom(),
        "artifact image must be contained by its output: output={rendered_output:?}, image={rendered_image:?}"
    );
    assert!(
        rendered_output.bottom() <= following_before.top(),
        "artifact output must not paint over the following block: output={rendered_output:?}, following={following_before:?}"
    );

    cx.simulate_click(rendered_layer.center(), Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    assert_eq!(
        Some(artifact_id),
        editor.read_with(&cx, |editor, _| editor.active_block())
    );
    assert_eq!(
        shell_before,
        cx.debug_bounds(shell_selector)
            .expect("artifact shell must remain mounted after activation")
    );
    assert_eq!(
        following_before,
        cx.debug_bounds(following_selector)
            .expect("following block must remain in place")
    );
    assert_eq!(
        scroll_before,
        editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset())
    );
    assert!(
        cx.debug_bounds(input_slot_selector).is_some(),
        "activation must reveal the input that was already mounted"
    );
}

#[gpui::test]
fn clicking_native_html_activates_its_mounted_input_without_moving_content(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_component::init);
    let source = "<section><h2>Native heading</h2><p>Rendered <strong>HTML</strong> content.</p></section>\n\nFollowing paragraph";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    assert!(matches!(document.blocks[0].kind, SourceBlockKind::Html));
    let html_id = document.blocks[0].id;
    let following_id = document.blocks[1].id;
    let (window, editor) = open_editor_with_size(source, size(px(760.), px(520.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let shell_selector =
        Box::leak(format!("markdown-html-shell-{}", html_id.0).into_boxed_str());
    let native_layer_selector =
        Box::leak(format!("markdown-html-native-layer-{}", html_id.0).into_boxed_str());
    let native_content_selector =
        Box::leak(format!("markdown-html-native-content-{}", html_id.0).into_boxed_str());
    let input_slot_selector =
        Box::leak(format!("markdown-block-input-slot-{}", html_id.0).into_boxed_str());
    let following_selector =
        Box::leak(format!("markdown-block-frame-{}", following_id.0).into_boxed_str());

    let shell_before = cx
        .debug_bounds(shell_selector)
        .expect("HTML must render through a stable shell");
    let native_layer = cx
        .debug_bounds(native_layer_selector)
        .expect("inactive HTML native layer must be visible");
    let native_content = cx
        .debug_bounds(native_content_selector)
        .expect("inactive HTML native content must be visible");
    assert!(
        cx.debug_bounds(input_slot_selector).is_some(),
        "HTML input must be laid out before its first activation"
    );
    let following_before = cx.debug_bounds(following_selector).unwrap();
    let scroll_before = editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset());
    assert!(
        native_content.bottom() <= shell_before.bottom(),
        "native HTML must be contained by its shell: shell={shell_before:?}, native={native_content:?}"
    );
    assert!(
        native_content.bottom() <= following_before.top(),
        "native HTML must not paint over the following block: native={native_content:?}, following={following_before:?}"
    );

    cx.simulate_click(native_layer.center(), Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    assert_eq!(
        Some(html_id),
        editor.read_with(&cx, |editor, _| editor.active_block())
    );
    assert_eq!(
        shell_before,
        cx.debug_bounds(shell_selector)
            .expect("HTML shell must remain mounted after activation")
    );
    assert_eq!(
        following_before,
        cx.debug_bounds(following_selector)
            .expect("following block must remain in place")
    );
    assert_eq!(
        scroll_before,
        editor.read_with(&cx, |editor, _| editor.vertical_scroll_offset())
    );
    assert!(
        cx.debug_bounds(input_slot_selector).is_some(),
        "activation must reveal the HTML input that was already mounted"
    );
}

#[gpui::test]
fn combined_document_keeps_every_preview_capability(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "# 组合回归\n\n正文包含 **粗体** 与 $e^{i\\pi}+1=0$ 行内公式。\n\n```mermaid\ngraph LR\n  A[输入] --> B[渲染]\n```\n\n$$\n\\frac{a+b}{c}\n$$\n\n![示例图片](missing-combination-regression.png)\n\n```rust\nfn main() {\n    println!(\"markdown\");\n}\n```\n\n| 名称 | 状态 |\n| :--- | ---: |\n| 数学公式 | 完成 |\n";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let paragraph_id = document.blocks[1].id;
    let mermaid_id = document.blocks[2].id;
    let math_id = document.blocks[3].id;
    let image_id = document.blocks[4].id;
    let code_id = document.blocks[5].id;
    let table_id = document.blocks[6].id;
    assert!(document.blocks[4]
        .inline_nodes
        .iter()
        .any(|node| matches!(
            node.kind,
            markdown_source::SourceInlineKind::Image { .. }
        )));
    let (window, editor) = open_editor_with_size(source, size(px(960.), px(720.)), cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, _, cx| {
        editor.set_block_render_provider(Some(svg_render_provider()), cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let selector = |prefix: &str, id: markdown_source::SourceNodeId| {
        Box::leak(format!("{prefix}-{}", id.0).into_boxed_str())
    };
    // Block math and Mermaid both render asynchronous SVG previews.
    assert!(cx.debug_bounds(selector("markdown-rendered-block", mermaid_id)).is_some());
    assert!(cx.debug_bounds(selector("markdown-rendered-block", math_id)).is_some());
    assert!(cx.debug_bounds(selector("markdown-render-error", mermaid_id)).is_none());
    assert!(cx.debug_bounds(selector("markdown-render-error", math_id)).is_none());
    // The inline formula keeps its rendered preview alongside plain text.
    assert_eq!(
        1,
        editor.read_with(&cx, |editor, _| editor.active_inline_math_preview_count())
    );
    assert!(cx.debug_bounds(selector("markdown-block-frame", paragraph_id)).is_some());
    // The image paragraph, fenced code and table all keep their rich previews.
    assert!(cx.debug_bounds(selector("markdown-preview-block", image_id)).is_some());
    assert!(cx.debug_bounds(selector("markdown-preview-block", code_id)).is_some());
    assert!(cx.debug_bounds(selector("markdown-table", table_id)).is_some());
    // Every block stays in document order without overlap.
    let mut previous_bottom = None;
    for id in [paragraph_id, mermaid_id, math_id, image_id, code_id, table_id] {
        let frame = cx.debug_bounds(selector("markdown-block-frame", id)).unwrap();
        if let Some(bottom) = previous_bottom {
            assert!(frame.top() >= bottom, "block {id:?} overlaps a previous block");
        }
        previous_bottom = Some(frame.bottom());
    }
}

#[gpui::test]
fn math_and_mermaid_provider_dispatch_uses_the_background_executor(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = include_str!("../src/editor/render/block_renderer.rs");

    assert_eq!(
        1,
        source
            .matches("cx.background_spawn(async move { provider(request).await })")
            .count(),
        "all shared math and Mermaid renders must run on the background executor"
    );
    assert!(
        !source.contains("let result = provider(request).await;"),
        "render providers must not be polled from a foreground GPUI task"
    );
}

#[gpui::test]
fn identical_math_blocks_share_one_render_task_and_cached_artifact(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "$$\nx + y\n$$\n\nBetween\n\n$$\nx + y\n$$";
    let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
    let first_id = document.blocks[0].id;
    let second_id = document.blocks[2].id;
    let calls = Arc::new(AtomicUsize::new(0));
    let provider_calls = calls.clone();
    let provider = Arc::new(move |_| {
        provider_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(Some(svg_artifact(64.))) })
            as futures::future::BoxFuture<
                'static,
                Result<Option<markdown_editor::MarkdownBlockRenderArtifact>, String>,
            >
    });
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, _, cx| {
        editor.set_block_render_provider(Some(provider), cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    assert_eq!(1, calls.load(Ordering::SeqCst));
    for block_id in [first_id, second_id] {
        assert!(
            cx.debug_bounds(Box::leak(
                format!("markdown-rendered-block-{}", block_id.0).into_boxed_str(),
            ))
            .is_some(),
            "both blocks must consume the shared artifact"
        );
    }
}

#[gpui::test]
fn failed_block_shows_source_fallback_and_retry_button_recovers(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "```mermaid\ngraph TD\nA --> B\n```";
    let block_id = markdown_source::SourceMarkdownDocument::parse(source)
        .unwrap()
        .blocks[0]
        .id;
    let calls = Arc::new(AtomicUsize::new(0));
    let provider_calls = calls.clone();
    let provider = Arc::new(move |_| {
        let call = provider_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if call == 0 {
                Err("Mermaid 语法错误".to_owned())
            } else {
                Ok(Some(svg_artifact(64.)))
            }
        })
            as futures::future::BoxFuture<
                'static,
                Result<Option<markdown_editor::MarkdownBlockRenderArtifact>, String>,
            >
    });
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, _, cx| {
        editor.set_block_render_provider(Some(provider), cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    let error_selector =
        Box::leak(format!("markdown-render-error-{}", block_id.0).into_boxed_str());
    let retry_selector =
        Box::leak(format!("markdown-render-retry-{}", block_id.0).into_boxed_str());
    assert!(cx.debug_bounds(error_selector).is_some());
    let retry = cx
        .debug_bounds(retry_selector)
        .expect("failed render must expose a retry entry");
    cx.simulate_click(retry.center(), Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    assert_eq!(2, calls.load(Ordering::SeqCst));
    assert!(cx.debug_bounds(error_selector).is_none());
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-rendered-block-{}", block_id.0).into_boxed_str(),
        ))
        .is_some()
    );
}

#[gpui::test]
fn pending_mermaid_render_does_not_block_edits_or_overwrite_new_output(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "```mermaid\ngraph TD\nA --> B\n```";
    let updated_source = "Intro\n\n```mermaid\ngraph TD\nX --> Y\n```";
    let (release_old, old_result) = futures::channel::oneshot::channel::<
        Result<Option<markdown_editor::MarkdownBlockRenderArtifact>, String>,
    >();
    let old_result = Arc::new(Mutex::new(Some(old_result)));
    let pending_result = old_result.clone();
    let pending_provider = Arc::new(move |_| {
        let receiver = pending_result
            .lock()
            .unwrap()
            .take()
            .expect("the pending provider should be requested once");
        Box::pin(async move {
            receiver
                .await
                .unwrap_or_else(|_| Err("pending renderer was dropped".to_owned()))
        })
            as futures::future::BoxFuture<
                'static,
                Result<Option<markdown_editor::MarkdownBlockRenderArtifact>, String>,
            >
    });
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, _, cx| {
        editor.set_block_render_provider(Some(pending_provider), cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    editor.update_in(&mut cx, |editor, window, cx| {
        editor.replace_source(updated_source, window, cx).unwrap();
        editor.set_block_render_provider(Some(svg_render_provider_with_height(180.)), cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert_eq!(
        updated_source,
        editor.read_with(&cx, |editor, _| editor.source().to_owned())
    );
    let block_id = markdown_source::SourceMarkdownDocument::parse(updated_source)
        .unwrap()
        .blocks[1]
        .id;
    let selector = Box::leak(format!("markdown-rendered-block-{}", block_id.0).into_boxed_str());
    let replacement_height = cx
        .debug_bounds(selector)
        .expect("replacement render should be visible")
        .size
        .height;

    release_old
        .send(Ok(Some(svg_artifact(64.))))
        .expect("pending render should still be awaiting its result");
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    assert_eq!(
        replacement_height,
        cx.debug_bounds(selector)
            .expect("stale completion must not remove the replacement render")
            .size
            .height
    );
}

#[gpui::test]
fn inline_math_uses_svg_preview_and_real_source_markers_when_activated(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "Euler: $e^{i\\pi} + 1 = 0$.";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, _, cx| {
        editor.set_block_render_provider(Some(svg_render_provider()), cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert_eq!(
        1,
        editor.read_with(&cx, |editor, _| editor.active_inline_math_preview_count())
    );
    assert_eq!(
        "Euler: e^{i\\pi} + 1 = 0.",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
    );

    editor.update_in(&mut cx, |editor, window, cx| editor.focus(window, cx));
    cx.run_until_parked();
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 10), window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        "Euler: $e^{i\\pi} + 1 = 0$.",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
    );
    let math_id = markdown_source::SourceMarkdownDocument::parse(source)
        .unwrap()
        .inline_node_at(source.find("e^").unwrap())
        .unwrap()
        .id;
    assert!(
        cx.debug_bounds(Box::leak(
            format!("markdown-active-inline-markers-{}", math_id.0).into_boxed_str(),
        ))
        .is_none()
    );
}

#[gpui::test]
fn active_paragraph_keeps_other_inline_math_rendered(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "First $one$ and second $two$.";
    let block_id = markdown_source::SourceMarkdownDocument::parse(source)
        .unwrap()
        .blocks[0]
        .id;
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, _, cx| {
        editor.set_block_render_provider(Some(svg_render_provider()), cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    editor.update_in(&mut cx, |editor, window, cx| {
        assert!(editor.activate_block(block_id, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(
        2,
        editor.read_with(&cx, |editor, _| editor.active_inline_math_preview_count())
    );
    let input = editor.read_with(&cx, |editor, _| editor.input_state());
    input.update_in(&mut cx, |input, window, cx| {
        input.set_cursor_position(Position::new(0, 8), window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        "First $one$ and second two.",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
    );
    assert_eq!(
        1,
        editor.read_with(&cx, |editor, _| editor.active_inline_math_preview_count())
    );
}

#[gpui::test]
fn failed_inline_math_render_keeps_readable_source_text(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = "Keep $unrendered$ readable.";
    let (window, editor) = open_editor(source, cx);
    let mut cx = VisualTestContext::from_window(window, cx);
    let provider = Arc::new(|_| {
        Box::pin(async { Ok(None) })
            as futures::future::BoxFuture<
                'static,
                Result<Option<markdown_editor::MarkdownBlockRenderArtifact>, String>,
            >
    });
    editor.update_in(&mut cx, |editor, _, cx| {
        editor.set_block_render_provider(Some(provider), cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();

    assert_eq!(
        "Keep unrendered readable.",
        editor.read_with(&cx, |editor, _| editor.projected_text().to_owned())
    );
    assert_eq!(
        0,
        editor.read_with(&cx, |editor, _| editor.active_inline_math_preview_count())
    );
}

#[gpui::test]
fn virtual_markdown_requests_renderers_only_for_visible_blocks(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let source = (0..100)
        .map(|index| format!("$$\nx_{index}\n$$"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let source: &'static str = Box::leak(source.into_boxed_str());
    let (window, editor) = open_editor_with_size(source, size(px(600.), px(260.)), cx);
    let calls = Arc::new(AtomicUsize::new(0));
    let provider_calls = calls.clone();
    let provider = Arc::new(move |_| {
        provider_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(None) })
            as futures::future::BoxFuture<
                'static,
                Result<Option<markdown_editor::MarkdownBlockRenderArtifact>, String>,
            >
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    editor.update_in(&mut cx, |editor, _, cx| {
        editor.set_block_render_provider(Some(provider), cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(calls.load(Ordering::SeqCst) < 20);
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

fn svg_render_provider() -> markdown_editor::MarkdownBlockRenderProvider {
    svg_render_provider_with_height(64.)
}

fn svg_render_provider_with_height(height: f32) -> markdown_editor::MarkdownBlockRenderProvider {
    Arc::new(move |_| Box::pin(async move { Ok(Some(svg_artifact(height))) }))
}

fn svg_artifact(height: f32) -> markdown_editor::MarkdownBlockRenderArtifact {
    markdown_editor::MarkdownBlockRenderArtifact {
        media_type: "image/svg+xml".to_owned(),
        bytes: format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="{height}"><rect width="120" height="{height}" fill="#4488ff"/></svg>"##
        )
        .into_bytes(),
        intrinsic_width: Some(120.),
        intrinsic_height: Some(height),
    }
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
