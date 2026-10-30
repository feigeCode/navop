//! Action definitions and key bindings for the embedded block editor.
//!
//! Text-editing actions are scoped to the `"BlockEditor"` key context on each
//! block. Viewport-level editor commands use global bindings so they remain
//! available while focus is on another embedded editor control.

use std::collections::{BTreeMap, BTreeSet};

use gpui::*;
actions!(
    velotype,
    [
        Newline,
        DeleteBack,
        Delete,
        WordDeleteBack,
        WordDeleteForward,
        FocusPrev,
        FocusNext,
        MoveLeft,
        MoveRight,
        WordMoveLeft,
        WordMoveRight,
        Home,
        End,
        BlockUp,
        BlockDown,
        PageUp,
        PageDown,
        JumpToTop,
        JumpToBottom,
        SelectLeft,
        SelectRight,
        WordSelectLeft,
        WordSelectRight,
        SelectHome,
        SelectEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        BoldSelection,
        ItalicSelection,
        UnderlineSelection,
        StrikethroughSelection,
        CodeSelection,
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
        IndentBlock,
        OutdentBlock,
        ExitCodeBlock,
        DismissTransientUi,
        ToggleViewMode,
    ]
);

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ShortcutCommand {
    Newline,
    DeleteBack,
    Delete,
    WordDeleteBack,
    WordDeleteForward,
    FocusPrev,
    FocusNext,
    MoveLeft,
    MoveRight,
    WordMoveLeft,
    WordMoveRight,
    Home,
    End,
    BlockUp,
    BlockDown,
    PageUp,
    PageDown,
    JumpToTop,
    JumpToBottom,
    SelectLeft,
    SelectRight,
    WordSelectLeft,
    WordSelectRight,
    SelectHome,
    SelectEnd,
    SelectAll,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    BoldSelection,
    ItalicSelection,
    UnderlineSelection,
    StrikethroughSelection,
    CodeSelection,
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
    IndentBlock,
    OutdentBlock,
    ExitCodeBlock,
    DismissTransientUi,
    ToggleViewMode,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ShortcutDefinition {
    pub(crate) command: ShortcutCommand,
    pub(crate) id: &'static str,
    pub(crate) default_keys: &'static [&'static str],
    pub(crate) context: Option<&'static str>,
}

const BLOCK_CONTEXT: Option<&str> = Some("BlockEditor");
/// 编辑器根节点的 key context：无 context 的绑定在 gpui 里优先级最高，
/// Esc 若绑成全局会压过对话框/列表自己的 Cancel，所以只在编辑器内生效。
pub(crate) const EDITOR_CONTEXT: &str = "MarkdownEditor";
const SELECT_ALL_ID: &str = "select_all";
const LEGACY_SELECT_ALL_IDS: &[&str] = &[
    "select_all_source_text",
    "select_focused_block_text_rendered",
];

const SHORTCUT_DEFINITIONS: &[ShortcutDefinition] = &[
    ShortcutDefinition {
        command: ShortcutCommand::Newline,
        id: "newline",
        default_keys: &["enter"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::DeleteBack,
        id: "delete_back",
        default_keys: &["backspace"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::Delete,
        id: "delete",
        default_keys: &["delete"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::WordDeleteBack,
        id: "word_delete_back",
        default_keys: &["ctrl-backspace", "alt-backspace"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::WordDeleteForward,
        id: "word_delete_forward",
        default_keys: &["ctrl-delete", "alt-delete"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::FocusPrev,
        id: "focus_prev",
        default_keys: &["up"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::FocusNext,
        id: "focus_next",
        default_keys: &["down"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::MoveLeft,
        id: "move_left",
        default_keys: &["left"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::MoveRight,
        id: "move_right",
        default_keys: &["right"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::WordMoveLeft,
        id: "word_move_left",
        default_keys: &["ctrl-left", "alt-left"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::WordMoveRight,
        id: "word_move_right",
        default_keys: &["ctrl-right", "alt-right"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::Home,
        id: "home",
        default_keys: &["home"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::End,
        id: "end",
        default_keys: &["end"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::BlockUp,
        id: "block_up",
        default_keys: &["ctrl-up", "alt-up"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::BlockDown,
        id: "block_down",
        default_keys: &["ctrl-down", "alt-down"],
        context: BLOCK_CONTEXT,
    },
    // Page scroll and document jumps operate on the editor viewport rather than
    // a single block, so they use global bindings (no context) and stay active
    // in both Rendered and Source mode.
    ShortcutDefinition {
        command: ShortcutCommand::PageUp,
        id: "page_up",
        default_keys: &["pageup"],
        context: None,
    },
    ShortcutDefinition {
        command: ShortcutCommand::PageDown,
        id: "page_down",
        default_keys: &["pagedown"],
        context: None,
    },
    ShortcutDefinition {
        command: ShortcutCommand::JumpToTop,
        id: "jump_to_top",
        default_keys: &["ctrl-home", "cmd-up"],
        context: None,
    },
    ShortcutDefinition {
        command: ShortcutCommand::JumpToBottom,
        id: "jump_to_bottom",
        default_keys: &["ctrl-end", "cmd-down"],
        context: None,
    },
    ShortcutDefinition {
        command: ShortcutCommand::SelectLeft,
        id: "select_left",
        default_keys: &["shift-left"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::SelectRight,
        id: "select_right",
        default_keys: &["shift-right"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::WordSelectLeft,
        id: "word_select_left",
        default_keys: &["ctrl-shift-left", "alt-shift-left"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::WordSelectRight,
        id: "word_select_right",
        default_keys: &["ctrl-shift-right", "alt-shift-right"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::SelectHome,
        id: "select_home",
        default_keys: &["shift-home"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::SelectEnd,
        id: "select_end",
        default_keys: &["shift-end"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::SelectAll,
        id: SELECT_ALL_ID,
        default_keys: &["cmd-a", "ctrl-a"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::Copy,
        id: "copy",
        default_keys: &["cmd-c", "ctrl-c"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::Cut,
        id: "cut",
        default_keys: &["cmd-x", "ctrl-x"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::Paste,
        id: "paste",
        default_keys: &["cmd-v", "ctrl-v"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::Undo,
        id: "undo",
        default_keys: &["cmd-z", "ctrl-z"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::Redo,
        id: "redo",
        default_keys: &["cmd-shift-z", "ctrl-y"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::BoldSelection,
        id: "bold_selection",
        default_keys: &["cmd-b", "ctrl-b"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::ItalicSelection,
        id: "italic_selection",
        default_keys: &["cmd-i", "ctrl-i"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::UnderlineSelection,
        id: "underline_selection",
        default_keys: &["cmd-u", "ctrl-u"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::StrikethroughSelection,
        id: "strikethrough_selection",
        default_keys: &["cmd-shift-x", "ctrl-shift-x"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::CodeSelection,
        id: "code_selection",
        default_keys: &["cmd-`", "ctrl-`"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::SetParagraph,
        id: "set_paragraph",
        default_keys: &["cmd-0", "ctrl-0"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::SetHeading1,
        id: "set_heading_1",
        default_keys: &["cmd-1", "ctrl-1"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::SetHeading2,
        id: "set_heading_2",
        default_keys: &["cmd-2", "ctrl-2"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::SetHeading3,
        id: "set_heading_3",
        default_keys: &["cmd-3", "ctrl-3"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::SetHeading4,
        id: "set_heading_4",
        default_keys: &["cmd-4", "ctrl-4"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::SetHeading5,
        id: "set_heading_5",
        default_keys: &["cmd-5", "ctrl-5"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::SetHeading6,
        id: "set_heading_6",
        default_keys: &["cmd-6", "ctrl-6"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::ToggleBulletList,
        id: "toggle_bullet_list",
        default_keys: &["cmd-shift-8", "ctrl-shift-8"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::ToggleOrderedList,
        id: "toggle_ordered_list",
        default_keys: &["cmd-shift-7", "ctrl-shift-7"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::ToggleTaskList,
        id: "toggle_task_list",
        default_keys: &["cmd-shift-t", "ctrl-shift-t"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::ToggleQuote,
        id: "toggle_quote",
        default_keys: &["cmd-shift-q", "ctrl-shift-q"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::ToggleCodeBlock,
        id: "toggle_code_block",
        default_keys: &["cmd-alt-c", "ctrl-alt-c"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::MoveBlockUp,
        id: "move_block_up",
        default_keys: &["cmd-shift-up", "ctrl-shift-up"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::MoveBlockDown,
        id: "move_block_down",
        default_keys: &["cmd-shift-down", "ctrl-shift-down"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::DuplicateBlock,
        id: "duplicate_block",
        default_keys: &["cmd-d", "ctrl-d"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::DeleteBlock,
        id: "delete_block",
        default_keys: &["cmd-shift-backspace", "ctrl-shift-backspace"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::IndentBlock,
        id: "indent_block",
        default_keys: &["tab"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::OutdentBlock,
        id: "outdent_block",
        default_keys: &["shift-tab"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::ExitCodeBlock,
        id: "exit_code_block",
        default_keys: &["cmd-enter", "ctrl-enter"],
        context: BLOCK_CONTEXT,
    },
    ShortcutDefinition {
        command: ShortcutCommand::DismissTransientUi,
        id: "dismiss_transient_ui",
        default_keys: &["escape"],
        context: Some(EDITOR_CONTEXT),
    },
    ShortcutDefinition {
        command: ShortcutCommand::ToggleViewMode,
        id: "toggle_view_mode",
        default_keys: &["cmd-/", "ctrl-/"],
        context: None,
    },
];

/// 编译目标对应的 gpui `OperatingSystem`。
fn current_os() -> gpui::OperatingSystem {
    match std::env::consts::OS {
        "macos" => gpui::OperatingSystem::Mac,
        "windows" => gpui::OperatingSystem::Windows,
        "linux" => gpui::OperatingSystem::Linux,
        "freebsd" => gpui::OperatingSystem::FreeBsd,
        "ios" => gpui::OperatingSystem::Ios,
        "android" => gpui::OperatingSystem::Android,
        _ => gpui::OperatingSystem::Unknown,
    }
}

/// 动态字符串构造 KeyBinding（gpui-fast v0.1.12 起 KeyBinding::new 只收 &'static str）。
fn load_dynamic_keybinding<A: gpui::Action>(
    keystrokes: &str,
    action: A,
    context: Option<&str>,
    cx: &App,
) -> KeyBinding {
    let context_predicate = context
        .map(|context| gpui::KeyBindingContextPredicate::parse(context).unwrap().into());
    KeyBinding::load(keystrokes, Box::new(action), context_predicate, false, None, cx).unwrap()
}

pub(crate) fn normalize_shortcut_keys(keys: &[String]) -> Option<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    let os = current_os();
    for key in keys {
        let parsed = Keystroke::parse_with_os(key.trim(), os).ok()?;
        if parsed.is_ime_in_progress() {
            return None;
        }
        let key = parsed.unparse(os);
        if seen.insert(key.clone()) {
            normalized.push(key);
        }
    }
    (!normalized.is_empty()).then_some(normalized)
}

fn default_keys(definition: ShortcutDefinition) -> Vec<String> {
    definition
        .default_keys
        .iter()
        .map(|key| key.to_string())
        .collect()
}

/// Legacy preference keys that should feed a modern shortcut definition.
///
/// Select-all used to be represented by separate source/rendered commands. The
/// editor now cycles those behaviors through one action, so old preferences map
/// forward to `select_all` instead of being silently discarded on load.
fn legacy_shortcut_ids(definition: ShortcutDefinition) -> &'static [&'static str] {
    match definition.command {
        ShortcutCommand::SelectAll => LEGACY_SELECT_ALL_IDS,
        _ => &[],
    }
}

/// Reads a user shortcut override, preferring the current id before aliases.
fn configured_shortcut_keys(
    definition: ShortcutDefinition,
    config: &BTreeMap<String, Vec<String>>,
) -> Option<Vec<String>> {
    config
        .get(definition.id)
        .and_then(|keys| normalize_shortcut_keys(keys))
        .or_else(|| {
            legacy_shortcut_ids(definition).iter().find_map(|id| {
                config
                    .get(*id)
                    .and_then(|keys| normalize_shortcut_keys(keys))
            })
        })
}

fn shortcuts_conflict(
    left: ShortcutDefinition,
    left_keys: &[String],
    right: ShortcutDefinition,
    right_keys: &[String],
) -> bool {
    left.context == right.context && left_keys.iter().any(|key| right_keys.contains(key))
}

pub(crate) fn normalize_shortcut_config(
    config: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, Vec<String>> {
    let mut effective: BTreeMap<&'static str, (bool, Vec<String>)> = BTreeMap::new();
    for definition in SHORTCUT_DEFINITIONS {
        let custom = configured_shortcut_keys(*definition, config);
        effective.insert(
            definition.id,
            match custom {
                Some(keys) if keys != default_keys(*definition) => (true, keys),
                _ => (false, default_keys(*definition)),
            },
        );
    }

    loop {
        let mut conflicted = BTreeSet::new();
        for (index, left) in SHORTCUT_DEFINITIONS.iter().enumerate() {
            let (left_custom, left_keys) = effective.get(left.id).expect("known shortcut");
            for right in SHORTCUT_DEFINITIONS.iter().skip(index + 1) {
                let (right_custom, right_keys) = effective.get(right.id).expect("known shortcut");
                if shortcuts_conflict(*left, left_keys, *right, right_keys) {
                    if *left_custom {
                        conflicted.insert(left.id);
                    }
                    if *right_custom {
                        conflicted.insert(right.id);
                    }
                }
            }
        }

        if conflicted.is_empty() {
            break;
        }

        for id in conflicted {
            if let Some(definition) = SHORTCUT_DEFINITIONS
                .iter()
                .find(|definition| definition.id == id)
            {
                effective.insert(definition.id, (false, default_keys(*definition)));
            }
        }
    }

    effective
        .into_iter()
        .filter_map(|(id, (custom, keys))| custom.then_some((id.to_string(), keys)))
        .collect()
}

#[cfg(test)]
pub(crate) fn resolved_shortcut_keys(
    config: &BTreeMap<String, Vec<String>>,
    command: ShortcutCommand,
) -> Vec<String> {
    let normalized = normalize_shortcut_config(config);
    let definition = SHORTCUT_DEFINITIONS
        .iter()
        .find(|definition| definition.command == command)
        .expect("known shortcut command");
    normalized
        .get(definition.id)
        .cloned()
        .unwrap_or_else(|| default_keys(*definition))
}

#[cfg(test)]
pub(crate) fn shortcut_conflict_for(
    command: ShortcutCommand,
    proposed_keys: &[String],
    config: &BTreeMap<String, Vec<String>>,
) -> Option<ShortcutDefinition> {
    let definition = SHORTCUT_DEFINITIONS
        .iter()
        .find(|definition| definition.command == command)?;
    let proposed_keys = normalize_shortcut_keys(proposed_keys)?;
    for other in SHORTCUT_DEFINITIONS
        .iter()
        .filter(|other| other.command != command)
    {
        let other_keys = resolved_shortcut_keys(config, other.command);
        if shortcuts_conflict(*definition, &proposed_keys, *other, &other_keys) {
            return Some(*other);
        }
    }
    None
}

fn key_binding_for(
    command: ShortcutCommand,
    key: &str,
    context: Option<&'static str>,
    cx: &App,
) -> KeyBinding {
    match command {
        ShortcutCommand::Newline => load_dynamic_keybinding(key, Newline, context, cx),
        ShortcutCommand::DeleteBack => load_dynamic_keybinding(key, DeleteBack, context, cx),
        ShortcutCommand::Delete => load_dynamic_keybinding(key, Delete, context, cx),
        ShortcutCommand::WordDeleteBack => load_dynamic_keybinding(key, WordDeleteBack, context, cx),
        ShortcutCommand::WordDeleteForward => load_dynamic_keybinding(key, WordDeleteForward, context, cx),
        ShortcutCommand::FocusPrev => load_dynamic_keybinding(key, FocusPrev, context, cx),
        ShortcutCommand::FocusNext => load_dynamic_keybinding(key, FocusNext, context, cx),
        ShortcutCommand::MoveLeft => load_dynamic_keybinding(key, MoveLeft, context, cx),
        ShortcutCommand::MoveRight => load_dynamic_keybinding(key, MoveRight, context, cx),
        ShortcutCommand::WordMoveLeft => load_dynamic_keybinding(key, WordMoveLeft, context, cx),
        ShortcutCommand::WordMoveRight => load_dynamic_keybinding(key, WordMoveRight, context, cx),
        ShortcutCommand::Home => load_dynamic_keybinding(key, Home, context, cx),
        ShortcutCommand::End => load_dynamic_keybinding(key, End, context, cx),
        ShortcutCommand::BlockUp => load_dynamic_keybinding(key, BlockUp, context, cx),
        ShortcutCommand::BlockDown => load_dynamic_keybinding(key, BlockDown, context, cx),
        ShortcutCommand::PageUp => load_dynamic_keybinding(key, PageUp, context, cx),
        ShortcutCommand::PageDown => load_dynamic_keybinding(key, PageDown, context, cx),
        ShortcutCommand::JumpToTop => load_dynamic_keybinding(key, JumpToTop, context, cx),
        ShortcutCommand::JumpToBottom => load_dynamic_keybinding(key, JumpToBottom, context, cx),
        ShortcutCommand::SelectLeft => load_dynamic_keybinding(key, SelectLeft, context, cx),
        ShortcutCommand::SelectRight => load_dynamic_keybinding(key, SelectRight, context, cx),
        ShortcutCommand::WordSelectLeft => load_dynamic_keybinding(key, WordSelectLeft, context, cx),
        ShortcutCommand::WordSelectRight => load_dynamic_keybinding(key, WordSelectRight, context, cx),
        ShortcutCommand::SelectHome => load_dynamic_keybinding(key, SelectHome, context, cx),
        ShortcutCommand::SelectEnd => load_dynamic_keybinding(key, SelectEnd, context, cx),
        ShortcutCommand::SelectAll => load_dynamic_keybinding(key, SelectAll, context, cx),
        ShortcutCommand::Copy => load_dynamic_keybinding(key, Copy, context, cx),
        ShortcutCommand::Cut => load_dynamic_keybinding(key, Cut, context, cx),
        ShortcutCommand::Paste => load_dynamic_keybinding(key, Paste, context, cx),
        ShortcutCommand::Undo => load_dynamic_keybinding(key, Undo, context, cx),
        ShortcutCommand::Redo => load_dynamic_keybinding(key, Redo, context, cx),
        ShortcutCommand::BoldSelection => load_dynamic_keybinding(key, BoldSelection, context, cx),
        ShortcutCommand::ItalicSelection => load_dynamic_keybinding(key, ItalicSelection, context, cx),
        ShortcutCommand::UnderlineSelection => load_dynamic_keybinding(key, UnderlineSelection, context, cx),
        ShortcutCommand::StrikethroughSelection => {
            load_dynamic_keybinding(key, StrikethroughSelection, context, cx)
        }
        ShortcutCommand::CodeSelection => load_dynamic_keybinding(key, CodeSelection, context, cx),
        ShortcutCommand::SetParagraph => load_dynamic_keybinding(key, SetParagraph, context, cx),
        ShortcutCommand::SetHeading1 => load_dynamic_keybinding(key, SetHeading1, context, cx),
        ShortcutCommand::SetHeading2 => load_dynamic_keybinding(key, SetHeading2, context, cx),
        ShortcutCommand::SetHeading3 => load_dynamic_keybinding(key, SetHeading3, context, cx),
        ShortcutCommand::SetHeading4 => load_dynamic_keybinding(key, SetHeading4, context, cx),
        ShortcutCommand::SetHeading5 => load_dynamic_keybinding(key, SetHeading5, context, cx),
        ShortcutCommand::SetHeading6 => load_dynamic_keybinding(key, SetHeading6, context, cx),
        ShortcutCommand::ToggleBulletList => load_dynamic_keybinding(key, ToggleBulletList, context, cx),
        ShortcutCommand::ToggleOrderedList => load_dynamic_keybinding(key, ToggleOrderedList, context, cx),
        ShortcutCommand::ToggleTaskList => load_dynamic_keybinding(key, ToggleTaskList, context, cx),
        ShortcutCommand::ToggleQuote => load_dynamic_keybinding(key, ToggleQuote, context, cx),
        ShortcutCommand::ToggleCodeBlock => load_dynamic_keybinding(key, ToggleCodeBlock, context, cx),
        ShortcutCommand::MoveBlockUp => load_dynamic_keybinding(key, MoveBlockUp, context, cx),
        ShortcutCommand::MoveBlockDown => load_dynamic_keybinding(key, MoveBlockDown, context, cx),
        ShortcutCommand::DuplicateBlock => load_dynamic_keybinding(key, DuplicateBlock, context, cx),
        ShortcutCommand::DeleteBlock => load_dynamic_keybinding(key, DeleteBlock, context, cx),
        ShortcutCommand::IndentBlock => load_dynamic_keybinding(key, IndentBlock, context, cx),
        ShortcutCommand::OutdentBlock => load_dynamic_keybinding(key, OutdentBlock, context, cx),
        ShortcutCommand::ExitCodeBlock => load_dynamic_keybinding(key, ExitCodeBlock, context, cx),
        ShortcutCommand::DismissTransientUi => load_dynamic_keybinding(key, DismissTransientUi, context, cx),
        ShortcutCommand::ToggleViewMode => load_dynamic_keybinding(key, ToggleViewMode, context, cx),
    }
}

pub(crate) fn resolved_keybindings(
    config: &BTreeMap<String, Vec<String>>,
    cx: &App,
) -> Vec<KeyBinding> {
    let normalized = normalize_shortcut_config(config);
    let mut bindings = Vec::new();
    for definition in SHORTCUT_DEFINITIONS {
        let keys = normalized
            .get(definition.id)
            .cloned()
            .unwrap_or_else(|| default_keys(*definition));
        bindings.extend(
            keys.iter()
                .map(|key| key_binding_for(definition.command, key, definition.context, cx)),
        );
    }
    bindings
}

pub(crate) fn install_keybindings(cx: &mut App, config: &BTreeMap<String, Vec<String>>) {
    cx.bind_keys(resolved_keybindings(config, cx));
}

/// Register key bindings for the block editor.
pub fn init(cx: &mut App) {
    install_keybindings(cx, &BTreeMap::new());
}

#[cfg(test)]
mod tests {
    use super::{
        ShortcutCommand, normalize_shortcut_config, resolved_shortcut_keys, shortcut_conflict_for,
    };
    use std::collections::BTreeMap;

    #[test]
    fn custom_shortcut_replaces_command_defaults() {
        let mut config = BTreeMap::new();
        config.insert("copy".to_string(), vec!["ctrl-alt-y".to_string()]);

        assert_eq!(
            resolved_shortcut_keys(&config, ShortcutCommand::Copy),
            vec!["ctrl-alt-y".to_string()]
        );
    }

    #[test]
    fn toggle_view_mode_has_default_shortcuts() {
        assert_eq!(
            resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::ToggleViewMode),
            vec!["cmd-/".to_string(), "ctrl-/".to_string()]
        );
    }

    #[test]
    fn typora_style_block_commands_have_default_shortcuts() {
        let config = BTreeMap::new();
        assert_eq!(
            resolved_shortcut_keys(&config, ShortcutCommand::SetHeading1),
            vec!["cmd-1".to_string(), "ctrl-1".to_string()]
        );
        assert_eq!(
            resolved_shortcut_keys(&config, ShortcutCommand::ToggleBulletList),
            vec!["cmd-shift-8".to_string(), "ctrl-shift-8".to_string()]
        );
        assert_eq!(
            resolved_shortcut_keys(&config, ShortcutCommand::DuplicateBlock),
            vec!["cmd-d".to_string(), "ctrl-d".to_string()]
        );
        assert_eq!(
            resolved_shortcut_keys(&config, ShortcutCommand::DeleteBlock),
            vec![
                "cmd-shift-backspace".to_string(),
                "ctrl-shift-backspace".to_string()
            ]
        );
    }

    #[test]
    fn select_all_has_default_shortcuts() {
        assert_eq!(
            resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::SelectAll),
            vec!["cmd-a".to_string(), "ctrl-a".to_string()]
        );
        assert!(
            shortcut_conflict_for(
                ShortcutCommand::SelectAll,
                &["cmd-a".to_string(), "ctrl-a".to_string()],
                &BTreeMap::new()
            )
            .is_none()
        );
    }

    #[test]
    fn select_all_shortcut_can_be_customized() {
        let mut config = BTreeMap::new();
        config.insert("select_all".to_string(), vec!["ctrl-shift-a".to_string()]);

        assert_eq!(
            resolved_shortcut_keys(&config, ShortcutCommand::SelectAll),
            vec!["ctrl-shift-a".to_string()]
        );
    }

    #[test]
    fn legacy_split_select_all_shortcut_config_maps_to_unified_command() {
        let mut config = BTreeMap::new();
        config.insert(
            "select_all_source_text".to_string(),
            vec!["ctrl-shift-a".to_string()],
        );

        assert_eq!(
            resolved_shortcut_keys(&config, ShortcutCommand::SelectAll),
            vec!["ctrl-shift-a".to_string()]
        );

        let normalized = normalize_shortcut_config(&config);
        assert_eq!(
            normalized.get("select_all"),
            Some(&vec!["ctrl-shift-a".to_string()])
        );
        assert!(!normalized.contains_key("select_all_source_text"));
        assert!(!normalized.contains_key("select_focused_block_text_rendered"));

        config.clear();
        config.insert(
            "select_focused_block_text_rendered".to_string(),
            vec!["ctrl-alt-shift-a".to_string()],
        );

        assert_eq!(
            resolved_shortcut_keys(&config, ShortcutCommand::SelectAll),
            vec!["ctrl-alt-shift-a".to_string()]
        );

        let normalized = normalize_shortcut_config(&config);
        assert_eq!(
            normalized.get("select_all"),
            Some(&vec!["ctrl-alt-shift-a".to_string()])
        );
        assert!(!normalized.contains_key("select_all_source_text"));
        assert!(!normalized.contains_key("select_focused_block_text_rendered"));
    }

    #[test]
    fn word_and_block_shortcuts_have_ctrl_and_alt_defaults() {
        assert_eq!(
            resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::WordMoveLeft),
            vec!["ctrl-left".to_string(), "alt-left".to_string()]
        );
        assert_eq!(
            resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::WordDeleteBack),
            vec!["ctrl-backspace".to_string(), "alt-backspace".to_string()]
        );
        assert_eq!(
            resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::BlockUp),
            vec!["ctrl-up".to_string(), "alt-up".to_string()]
        );
        assert_eq!(
            resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::WordSelectRight),
            vec![
                "ctrl-shift-right".to_string(),
                "alt-shift-right".to_string()
            ]
        );
    }

    #[test]
    fn page_navigation_shortcuts_have_defaults() {
        assert_eq!(
            resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::PageUp),
            vec!["pageup".to_string()]
        );
        assert_eq!(
            resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::PageDown),
            vec!["pagedown".to_string()]
        );
        assert_eq!(
            resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::JumpToTop),
            vec!["ctrl-home".to_string(), "cmd-up".to_string()]
        );
        assert_eq!(
            resolved_shortcut_keys(&BTreeMap::new(), ShortcutCommand::JumpToBottom),
            vec!["ctrl-end".to_string(), "cmd-down".to_string()]
        );
    }

    #[test]
    fn invalid_or_empty_shortcuts_fall_back_to_defaults() {
        let mut config = BTreeMap::new();
        config.insert("copy".to_string(), vec!["".to_string()]);
        config.insert("paste".to_string(), Vec::new());

        let normalized = normalize_shortcut_config(&config);
        assert!(!normalized.contains_key("copy"));
        assert!(!normalized.contains_key("paste"));
    }

    #[test]
    fn conflicting_custom_shortcut_falls_back_to_default() {
        let mut config = BTreeMap::new();
        config.insert("copy".to_string(), vec!["ctrl-x".to_string()]);

        let normalized = normalize_shortcut_config(&config);
        assert!(!normalized.contains_key("copy"));
        assert_eq!(
            resolved_shortcut_keys(&config, ShortcutCommand::Copy),
            vec!["cmd-c".to_string(), "ctrl-c".to_string()]
        );
    }

    #[test]
    fn detects_shortcut_conflicts_for_preferences_drafts() {
        let conflict = shortcut_conflict_for(
            ShortcutCommand::Copy,
            &["ctrl-x".to_string()],
            &BTreeMap::new(),
        )
        .expect("copy should conflict with cut");

        assert_eq!(conflict.id, "cut");
    }
}
