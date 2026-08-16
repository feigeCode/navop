use gpui::{Action, App, KeyBinding};
use markdown_editor::{
    BlockDown, BlockUp, BoldSelection, CodeSelection, Copy, Cut, Delete, DeleteBack, DeleteBlock,
    DismissTransientUi, DuplicateBlock, End, ExitCodeBlock, FocusNext, FocusPrev, Home,
    IndentBlock, ItalicSelection, JumpToBottom, JumpToTop, MoveBlockDown, MoveBlockUp, MoveLeft,
    MoveRight, Newline, OutdentBlock, PageDown, PageUp, Paste, Redo, SelectAll, SelectEnd,
    SelectHome, SelectLeft, SelectRight, SetHeading1, SetHeading2, SetHeading3, SetHeading4,
    SetHeading5, SetHeading6, SetParagraph, StrikethroughSelection, ToggleBulletList,
    ToggleCodeBlock, ToggleOrderedList, ToggleQuote, ToggleTaskList, ToggleViewMode,
    UnderlineSelection, Undo, WordDeleteBack, WordDeleteForward, WordMoveLeft, WordMoveRight,
    WordSelectLeft, WordSelectRight,
};
use one_core::keybindings::rebind_keybindings;

const BLOCK_CONTEXT: Option<&str> = Some("BlockEditor");
const GLOBAL_CONTEXT: Option<&str> = None;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotesShortcutDescriptor {
    pub command_id: String,
    pub title: String,
    pub default_keys: Vec<String>,
}

macro_rules! bind_commands {
    ($bindings:expr, $cx:expr, $(($id:expr, $title:expr, $keys:expr, $context:expr, $action:expr),)*) => {
        $(append_bindings(&mut $bindings, $cx, $id, $keys, $context, $action);)*
    };
}

macro_rules! collect_descriptors {
    ($descriptors:expr, $(($id:expr, $title:expr, $keys:expr, $context:expr, $action:expr),)*) => {
        $descriptors.extend([$(descriptor($id, $title, $keys)),*]);
    };
}

macro_rules! command_list {
    ($visitor:ident $(, $args:expr)*) => {
        $visitor! {
            $($args,)*
            ("edit.undo", "Undo", &["secondary-z"], BLOCK_CONTEXT, Undo),
            ("edit.redo", "Redo", &["secondary-shift-z", "secondary-y"], BLOCK_CONTEXT, Redo),
            ("edit.select_all", "Select All", &["secondary-a"], BLOCK_CONTEXT, SelectAll),
            ("edit.newline", "Insert Newline", &["enter"], BLOCK_CONTEXT, Newline),
            ("edit.delete_backward", "Delete Backward", &["backspace"], BLOCK_CONTEXT, DeleteBack),
            ("edit.delete_forward", "Delete Forward", &["delete"], BLOCK_CONTEXT, Delete),
            (
                "edit.delete_word_backward",
                "Delete Word Backward",
                &["ctrl-backspace", "alt-backspace"],
                BLOCK_CONTEXT,
                WordDeleteBack
            ),
            (
                "edit.delete_word_forward",
                "Delete Word Forward",
                &["ctrl-delete", "alt-delete"],
                BLOCK_CONTEXT,
                WordDeleteForward
            ),
            ("edit.copy", "Copy", &["secondary-c"], BLOCK_CONTEXT, Copy),
            ("edit.cut", "Cut", &["secondary-x"], BLOCK_CONTEXT, Cut),
            ("edit.paste", "Paste", &["secondary-v"], BLOCK_CONTEXT, Paste),
            (
                "navigation.focus_previous",
                "Focus Previous Line",
                &["up"],
                BLOCK_CONTEXT,
                FocusPrev
            ),
            (
                "navigation.focus_next",
                "Focus Next Line",
                &["down"],
                BLOCK_CONTEXT,
                FocusNext
            ),
            ("navigation.move_left", "Move Left", &["left"], BLOCK_CONTEXT, MoveLeft),
            ("navigation.move_right", "Move Right", &["right"], BLOCK_CONTEXT, MoveRight),
            (
                "navigation.move_word_left",
                "Move Word Left",
                &["ctrl-left", "alt-left"],
                BLOCK_CONTEXT,
                WordMoveLeft
            ),
            (
                "navigation.move_word_right",
                "Move Word Right",
                &["ctrl-right", "alt-right"],
                BLOCK_CONTEXT,
                WordMoveRight
            ),
            ("navigation.home", "Move to Line Start", &["home"], BLOCK_CONTEXT, Home),
            ("navigation.end", "Move to Line End", &["end"], BLOCK_CONTEXT, End),
            (
                "navigation.block_up",
                "Focus Previous Block",
                &["ctrl-up", "alt-up"],
                BLOCK_CONTEXT,
                BlockUp
            ),
            (
                "navigation.block_down",
                "Focus Next Block",
                &["ctrl-down", "alt-down"],
                BLOCK_CONTEXT,
                BlockDown
            ),
            (
                "navigation.page_up",
                "Page Up",
                &["pageup"],
                GLOBAL_CONTEXT,
                PageUp
            ),
            (
                "navigation.page_down",
                "Page Down",
                &["pagedown"],
                GLOBAL_CONTEXT,
                PageDown
            ),
            (
                "navigation.jump_to_top",
                "Jump to Top",
                &["ctrl-home", "cmd-up"],
                GLOBAL_CONTEXT,
                JumpToTop
            ),
            (
                "navigation.jump_to_bottom",
                "Jump to Bottom",
                &["ctrl-end", "cmd-down"],
                GLOBAL_CONTEXT,
                JumpToBottom
            ),
            (
                "selection.extend_left",
                "Extend Selection Left",
                &["shift-left"],
                BLOCK_CONTEXT,
                SelectLeft
            ),
            (
                "selection.extend_right",
                "Extend Selection Right",
                &["shift-right"],
                BLOCK_CONTEXT,
                SelectRight
            ),
            (
                "selection.extend_word_left",
                "Extend Selection by Word Left",
                &["ctrl-shift-left", "alt-shift-left"],
                BLOCK_CONTEXT,
                WordSelectLeft
            ),
            (
                "selection.extend_word_right",
                "Extend Selection by Word Right",
                &["ctrl-shift-right", "alt-shift-right"],
                BLOCK_CONTEXT,
                WordSelectRight
            ),
            (
                "selection.extend_home",
                "Extend Selection to Line Start",
                &["shift-home"],
                BLOCK_CONTEXT,
                SelectHome
            ),
            (
                "selection.extend_end",
                "Extend Selection to Line End",
                &["shift-end"],
                BLOCK_CONTEXT,
                SelectEnd
            ),
            (
                "format.toggle_bold",
                "Bold",
                &["secondary-b"],
                BLOCK_CONTEXT,
                BoldSelection
            ),
            (
                "format.toggle_italic",
                "Italic",
                &["secondary-i"],
                BLOCK_CONTEXT,
                ItalicSelection
            ),
            (
                "format.toggle_underline",
                "Underline",
                &["secondary-u"],
                BLOCK_CONTEXT,
                UnderlineSelection
            ),
            (
                "format.toggle_strike",
                "Strikethrough",
                &["secondary-shift-x"],
                BLOCK_CONTEXT,
                StrikethroughSelection
            ),
            (
                "format.toggle_inline_code",
                "Inline Code",
                &["secondary-`"],
                BLOCK_CONTEXT,
                CodeSelection
            ),
            (
                "block.set_paragraph",
                "Paragraph",
                &["secondary-0"],
                BLOCK_CONTEXT,
                SetParagraph
            ),
            (
                "block.set_heading_1",
                "Heading 1",
                &["secondary-1"],
                BLOCK_CONTEXT,
                SetHeading1
            ),
            (
                "block.set_heading_2",
                "Heading 2",
                &["secondary-2"],
                BLOCK_CONTEXT,
                SetHeading2
            ),
            (
                "block.set_heading_3",
                "Heading 3",
                &["secondary-3"],
                BLOCK_CONTEXT,
                SetHeading3
            ),
            (
                "block.set_heading_4",
                "Heading 4",
                &["secondary-4"],
                BLOCK_CONTEXT,
                SetHeading4
            ),
            (
                "block.set_heading_5",
                "Heading 5",
                &["secondary-5"],
                BLOCK_CONTEXT,
                SetHeading5
            ),
            (
                "block.set_heading_6",
                "Heading 6",
                &["secondary-6"],
                BLOCK_CONTEXT,
                SetHeading6
            ),
            (
                "block.toggle_bullet_list",
                "Bullet List",
                &["secondary-shift-8"],
                BLOCK_CONTEXT,
                ToggleBulletList
            ),
            (
                "block.toggle_ordered_list",
                "Ordered List",
                &["secondary-shift-7"],
                BLOCK_CONTEXT,
                ToggleOrderedList
            ),
            (
                "block.toggle_task_list",
                "Task List",
                &["secondary-shift-t"],
                BLOCK_CONTEXT,
                ToggleTaskList
            ),
            (
                "block.toggle_quote",
                "Quote",
                &["secondary-shift-q"],
                BLOCK_CONTEXT,
                ToggleQuote
            ),
            (
                "block.toggle_code",
                "Code Block",
                &["secondary-alt-c"],
                BLOCK_CONTEXT,
                ToggleCodeBlock
            ),
            (
                "block.move_up",
                "Move Block Up",
                &["secondary-shift-up"],
                BLOCK_CONTEXT,
                MoveBlockUp
            ),
            (
                "block.move_down",
                "Move Block Down",
                &["secondary-shift-down"],
                BLOCK_CONTEXT,
                MoveBlockDown
            ),
            (
                "block.duplicate_selected",
                "Duplicate Block",
                &["secondary-d"],
                BLOCK_CONTEXT,
                DuplicateBlock
            ),
            (
                "block.delete_current",
                "Delete Current Block",
                &["secondary-shift-backspace"],
                BLOCK_CONTEXT,
                DeleteBlock
            ),
            (
                "block.indent",
                "Indent Block",
                &["tab"],
                BLOCK_CONTEXT,
                IndentBlock
            ),
            (
                "block.outdent",
                "Outdent Block",
                &["shift-tab"],
                BLOCK_CONTEXT,
                OutdentBlock
            ),
            (
                "block.exit_code_block",
                "Exit Code Block",
                &["secondary-enter"],
                BLOCK_CONTEXT,
                ExitCodeBlock
            ),
            (
                "view.dismiss_transient_ui",
                "Dismiss Transient UI",
                &["escape"],
                GLOBAL_CONTEXT,
                DismissTransientUi
            ),
            (
                "view.toggle_source_mode",
                "Toggle Source Mode",
                &["secondary-/"],
                GLOBAL_CONTEXT,
                ToggleViewMode
            ),
        }
    };
}

pub fn init(cx: &mut App) {
    refresh(cx);
}

pub fn refresh(cx: &mut App) {
    let mut bindings = Vec::new();
    command_list!(bind_commands, bindings, cx);
    cx.bind_keys(bindings);
}

pub fn descriptors() -> Vec<NotesShortcutDescriptor> {
    let mut descriptors = Vec::new();
    command_list!(collect_descriptors, descriptors);
    descriptors
}

fn append_bindings<A: Action + Clone>(
    bindings: &mut Vec<KeyBinding>,
    cx: &App,
    id: &'static str,
    defaults: &'static [&'static str],
    context: Option<&'static str>,
    action: A,
) {
    bindings.extend(rebind_keybindings(cx, id, defaults, context, action));
}

fn descriptor(
    id: &'static str,
    title: &'static str,
    defaults: &'static [&'static str],
) -> NotesShortcutDescriptor {
    NotesShortcutDescriptor {
        command_id: id.to_owned(),
        title: title.to_owned(),
        default_keys: defaults.iter().map(|key| (*key).to_owned()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Keystroke;

    #[test]
    fn all_markdown_shortcuts_have_valid_unique_commands_and_keys() {
        let descriptors = descriptors();
        assert_eq!(descriptors.len(), 57);

        let mut ids = std::collections::HashSet::new();
        for descriptor in descriptors {
            assert!(ids.insert(descriptor.command_id));
            for key in descriptor.default_keys {
                Keystroke::parse(&key).unwrap();
            }
        }
    }

    #[test]
    fn markdown_shortcuts_include_typora_style_source_mode_and_editing_commands() {
        let descriptors = descriptors();
        let by_id = descriptors
            .into_iter()
            .map(|descriptor| (descriptor.command_id.clone(), descriptor))
            .collect::<std::collections::HashMap<_, _>>();

        assert_eq!(
            by_id["view.toggle_source_mode"].default_keys,
            vec!["secondary-/".to_string()]
        );
        assert_eq!(
            by_id["format.toggle_bold"].default_keys,
            vec!["secondary-b".to_string()]
        );
        assert_eq!(
            by_id["format.toggle_inline_code"].default_keys,
            vec!["secondary-`".to_string()]
        );
        assert_eq!(by_id["block.indent"].default_keys, vec!["tab".to_string()]);
        assert_eq!(
            by_id["format.toggle_strike"].default_keys,
            vec!["secondary-shift-x".to_string()]
        );
        assert_eq!(
            by_id["block.set_heading_1"].default_keys,
            vec!["secondary-1".to_string()]
        );
        assert_eq!(
            by_id["block.toggle_bullet_list"].default_keys,
            vec!["secondary-shift-8".to_string()]
        );
        assert_eq!(
            by_id["block.move_up"].default_keys,
            vec!["secondary-shift-up".to_string()]
        );
        assert_eq!(
            by_id["block.duplicate_selected"].default_keys,
            vec!["secondary-d".to_string()]
        );
        assert_eq!(
            by_id["block.delete_current"].default_keys,
            vec!["secondary-shift-backspace".to_string()]
        );
        assert!(by_id.contains_key("edit.copy"));
        assert!(by_id.contains_key("selection.extend_word_right"));
    }
}
