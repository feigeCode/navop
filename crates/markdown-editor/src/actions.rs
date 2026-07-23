use gpui::{App, KeyBinding, actions};

pub const EDITOR_CONTEXT: &str = "MarkdownEditor";
pub const INPUT_CONTEXT: &str = "MarkdownEditor > Input";

actions!(
    markdown_editor,
    [
        UndoSourceEdit,
        RedoSourceEdit,
        SelectAll,
        ToggleBold,
        ToggleItalic,
        ToggleUnderline,
        ToggleStrike,
        ToggleInlineCode,
        SetParagraph,
        SetHeading1,
        SetHeading2,
        SetHeading3,
        SetHeading4,
        SetHeading5,
        SetHeading6,
        ToggleBulletList,
        ToggleOrderedList,
        ToggleTaskList,
        ToggleQuote,
        ToggleCodeBlock,
        MoveBlockUp,
        MoveBlockDown,
        DuplicateBlock,
        DeleteBlock,
        InsertTableRowAbove,
        InsertTableRowBelow,
        DeleteTableRow,
        InsertTableColumnLeft,
        InsertTableColumnRight,
        DeleteTableColumn,
        AlignTableColumnLeft,
        AlignTableColumnCenter,
        AlignTableColumnRight,
        DeleteActiveImageBackward,
        DeleteActiveImageForward
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys(default_keybindings());
}

fn default_keybindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-z", UndoSourceEdit, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-shift-z", RedoSourceEdit, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-y", RedoSourceEdit, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-a", SelectAll, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-b", ToggleBold, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-i", ToggleItalic, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-u", ToggleUnderline, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-shift-x", ToggleStrike, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-e", ToggleInlineCode, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-0", SetParagraph, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-1", SetHeading1, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-2", SetHeading2, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-3", SetHeading3, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-4", SetHeading4, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-5", SetHeading5, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-6", SetHeading6, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-shift-8", ToggleBulletList, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-shift-7", ToggleOrderedList, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-shift-t", ToggleTaskList, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-shift-q", ToggleQuote, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-alt-c", ToggleCodeBlock, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-shift-up", MoveBlockUp, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-shift-down", MoveBlockDown, Some(INPUT_CONTEXT)),
        KeyBinding::new("secondary-d", DuplicateBlock, Some(INPUT_CONTEXT)),
        KeyBinding::new(
            "secondary-shift-backspace",
            DeleteBlock,
            Some(INPUT_CONTEXT),
        ),
        KeyBinding::new("secondary-alt-up", InsertTableRowAbove, Some(INPUT_CONTEXT)),
        KeyBinding::new(
            "secondary-alt-down",
            InsertTableRowBelow,
            Some(INPUT_CONTEXT),
        ),
        KeyBinding::new(
            "secondary-alt-shift-up",
            DeleteTableRow,
            Some(INPUT_CONTEXT),
        ),
        KeyBinding::new(
            "secondary-alt-left",
            InsertTableColumnLeft,
            Some(INPUT_CONTEXT),
        ),
        KeyBinding::new(
            "secondary-alt-right",
            InsertTableColumnRight,
            Some(INPUT_CONTEXT),
        ),
        KeyBinding::new(
            "secondary-alt-shift-left",
            DeleteTableColumn,
            Some(INPUT_CONTEXT),
        ),
        KeyBinding::new("secondary-alt-l", AlignTableColumnLeft, Some(INPUT_CONTEXT)),
        KeyBinding::new(
            "secondary-alt-e",
            AlignTableColumnCenter,
            Some(INPUT_CONTEXT),
        ),
        KeyBinding::new(
            "secondary-alt-r",
            AlignTableColumnRight,
            Some(INPUT_CONTEXT),
        ),
        KeyBinding::new("backspace", DeleteActiveImageBackward, Some(INPUT_CONTEXT)),
        KeyBinding::new("delete", DeleteActiveImageForward, Some(INPUT_CONTEXT)),
    ]
}
