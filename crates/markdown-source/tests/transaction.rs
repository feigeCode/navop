use markdown_source::{
    PatchError, SourceEdit, SourceEditOrigin, SourceHistory, SourceMarkdownDocument,
    SourceTransaction,
};

fn title_edit(document: &SourceMarkdownDocument) -> SourceTransaction {
    SourceTransaction {
        edits: vec![SourceEdit::new(2..7, "New Title", document.revision)],
        origin: SourceEditOrigin::RichTextTyping,
        allowed_ranges: vec![2..7],
        selection_before: markdown_source::SourceSelection::default(),
        selection_after: markdown_source::SourceSelection::default(),
    }
}

#[test]
fn transaction_changes_only_the_allowed_source_range() {
    let document = SourceMarkdownDocument::parse("# Title\n\n_italic_\n").unwrap();
    let applied = document.apply_transaction(&title_edit(&document)).unwrap();
    assert_eq!("# New Title\n\n_italic_\n", applied.document.source);
    assert_eq!(1, applied.document.revision);
    assert_eq!("Title", applied.inverse_edits[0].replacement);
}

#[test]
fn stale_and_overlapping_edits_are_rejected() {
    let document = SourceMarkdownDocument::parse("abcdef").unwrap();
    let stale = SourceTransaction {
        edits: vec![SourceEdit::new(0..1, "x", 99)],
        origin: SourceEditOrigin::SourceEditor,
        allowed_ranges: vec![0..1],
        selection_before: markdown_source::SourceSelection::default(),
        selection_after: markdown_source::SourceSelection::default(),
    };
    assert_eq!(
        PatchError::StaleRevision,
        document.apply_transaction(&stale).unwrap_err()
    );

    let overlapping = SourceTransaction {
        edits: vec![SourceEdit::new(0..3, "x", 0), SourceEdit::new(2..4, "y", 0)],
        origin: SourceEditOrigin::SourceEditor,
        allowed_ranges: vec![0..4],
        selection_before: markdown_source::SourceSelection::default(),
        selection_after: markdown_source::SourceSelection::default(),
    };
    assert_eq!(
        PatchError::OverlappingEdits,
        document.apply_transaction(&overlapping).unwrap_err()
    );
}

#[test]
fn undo_and_redo_restore_exact_markdown_spelling() {
    let document = SourceMarkdownDocument::parse("# Title\n\n_italic_\n").unwrap();
    let mut history = SourceHistory::new(document.clone());
    history.apply(&title_edit(&document)).unwrap();
    assert!(history.undo().unwrap().is_some());
    assert_eq!(document.source, history.document().source);
    assert!(history.redo().unwrap().is_some());
    assert_eq!("# New Title\n\n_italic_\n", history.document().source);
}

#[test]
fn undo_and_redo_restore_source_selections() {
    let document = SourceMarkdownDocument::parse("before").unwrap();
    let mut history = SourceHistory::new(document.clone());
    let transaction = SourceTransaction {
        edits: vec![SourceEdit::new(0..6, "after", document.revision)],
        origin: SourceEditOrigin::RichTextTyping,
        allowed_ranges: vec![0..6],
        selection_before: markdown_source::SourceSelection { anchor: 1, head: 4 },
        selection_after: markdown_source::SourceSelection { anchor: 5, head: 5 },
    };
    history.apply(&transaction).unwrap();
    assert_eq!(
        transaction.selection_before,
        history.undo().unwrap().unwrap()
    );
    assert_eq!(
        transaction.selection_after,
        history.redo().unwrap().unwrap()
    );
    assert_eq!(
        transaction.selection_before,
        history.undo().unwrap().unwrap()
    );
    assert_eq!(
        transaction.selection_after,
        history.redo().unwrap().unwrap()
    );
}

fn insertion(
    document: &SourceMarkdownDocument,
    offset: usize,
    value: &str,
    origin: SourceEditOrigin,
) -> SourceTransaction {
    SourceTransaction {
        edits: vec![SourceEdit::new(offset..offset, value, document.revision)],
        origin,
        allowed_ranges: vec![offset..offset],
        selection_before: markdown_source::SourceSelection {
            anchor: offset,
            head: offset,
        },
        selection_after: markdown_source::SourceSelection {
            anchor: offset + value.len(),
            head: offset + value.len(),
        },
    }
}

#[test]
fn continuous_typing_is_undone_and_redone_as_one_step() {
    let document = SourceMarkdownDocument::parse("").unwrap();
    let mut history = SourceHistory::new(document);
    history
        .apply(&insertion(
            history.document(),
            0,
            "你",
            SourceEditOrigin::RichTextTyping,
        ))
        .unwrap();
    history
        .apply(&insertion(
            history.document(),
            "你".len(),
            "好",
            SourceEditOrigin::RichTextTyping,
        ))
        .unwrap();
    history
        .apply(&insertion(
            history.document(),
            "你好".len(),
            "🙂",
            SourceEditOrigin::RichTextTyping,
        ))
        .unwrap();

    assert_eq!("你好🙂", history.document().source);
    assert!(history.undo().unwrap().is_some());
    assert_eq!("", history.document().source);
    assert!(history.undo().unwrap().is_none());
    assert!(history.redo().unwrap().is_some());
    assert_eq!("你好🙂", history.document().source);
}

#[test]
fn paste_is_a_typing_history_barrier() {
    let document = SourceMarkdownDocument::parse("").unwrap();
    let mut history = SourceHistory::new(document);
    history
        .apply(&insertion(
            history.document(),
            0,
            "a",
            SourceEditOrigin::RichTextTyping,
        ))
        .unwrap();
    history
        .apply(&insertion(
            history.document(),
            1,
            "pasted",
            SourceEditOrigin::Paste,
        ))
        .unwrap();

    history.undo().unwrap();
    assert_eq!("a", history.document().source);
    history.undo().unwrap();
    assert_eq!("", history.document().source);
}

#[test]
fn selection_discontinuity_prevents_typing_coalescing() {
    let document = SourceMarkdownDocument::parse("ab").unwrap();
    let mut history = SourceHistory::new(document);
    let first = SourceTransaction {
        edits: vec![SourceEdit::new(1..1, "x", history.document().revision)],
        origin: SourceEditOrigin::RichTextTyping,
        allowed_ranges: vec![1..1],
        selection_before: markdown_source::SourceSelection { anchor: 1, head: 1 },
        selection_after: markdown_source::SourceSelection { anchor: 2, head: 2 },
    };
    history.apply(&first).unwrap();
    let second = SourceTransaction {
        edits: vec![SourceEdit::new(0..0, "y", history.document().revision)],
        origin: SourceEditOrigin::RichTextTyping,
        allowed_ranges: vec![0..0],
        selection_before: markdown_source::SourceSelection { anchor: 0, head: 0 },
        selection_after: markdown_source::SourceSelection { anchor: 1, head: 1 },
    };
    history.apply(&second).unwrap();

    history.undo().unwrap();
    assert_eq!("axb", history.document().source);
    history.undo().unwrap();
    assert_eq!("ab", history.document().source);
}
