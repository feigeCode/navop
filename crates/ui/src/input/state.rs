//! A text input field that allows the user to enter text.
//!
//! Based on the `Input` example from the `gpui` crate.
//! https://github.com/zed-industries/zed/blob/main/crates/gpui/examples/input.rs
use anyhow::Result;
use gpui::{
    Action, App, AppContext, Bounds, ClickEvent, ClipboardItem, Context, Edges, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, HighlightStyle, Hsla,
    InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Point, Render, ScrollHandle,
    ScrollWheelEvent, ShapedLine, SharedString, Styled as _, Subscription, Task, UTF16Selection,
    Window, actions, div, point, prelude::FluentBuilder as _, px,
};
use gpui::{Half, TextAlign};
use ropey::{Rope, RopeSlice};
use serde::Deserialize;
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use sum_tree::Bias;
use unicode_segmentation::*;

use super::{
    DisplayMap, MASK_CHAR,
    blink_cursor::BlinkCursor,
    change::Change,
    element::{EditorScrollbarSnapshot, TextElement},
    mask_pattern::MaskPattern,
    mode::InputMode,
    number_input,
};
use crate::actions::{SelectDown, SelectLeft, SelectRight, SelectUp};
#[cfg(not(target_family = "wasm"))]
use crate::highlighter::LanguageRegistry;
use crate::highlighter::{DiagnosticSet, HighlightTheme};
use crate::input::blink_cursor::CURSOR_WIDTH;
use crate::input::movement::MoveDirection;
use crate::input::{
    HoverDefinition, InlineCompletion, Lsp, Position, RopeExt as _, Selection,
    display_map::LineLayout,
    element::RIGHT_MARGIN,
    popovers::{ContextMenu, DiagnosticPopover, HoverPopover, InputContextMenu, SignatureHelpPopover},
    search::{self, SearchPanel},
};
use crate::menu::PopupMenu;
use crate::{Icon, IconName, Size};
use crate::{Root, history::History};

#[derive(Action, Clone, PartialEq, Eq, Deserialize)]
#[action(namespace = input, no_json)]
pub struct Enter {
    /// Is confirm with secondary.
    pub secondary: bool,
}

actions!(
    input,
    [
        Backspace,
        Delete,
        DeleteToBeginningOfLine,
        DeleteToEndOfLine,
        DeleteToPreviousWordStart,
        DeleteToNextWordEnd,
        Indent,
        Outdent,
        IndentInline,
        OutdentInline,
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        MoveHome,
        MoveEnd,
        MovePageUp,
        MovePageDown,
        SelectAll,
        SelectToStartOfLine,
        SelectToEndOfLine,
        SelectToStart,
        SelectToEnd,
        SelectToPreviousWordStart,
        SelectToNextWordEnd,
        ShowCharacterPalette,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        MoveToStartOfLine,
        MoveToEndOfLine,
        MoveToStart,
        MoveToEnd,
        MoveToPreviousWord,
        MoveToNextWord,
        Escape,
        ToggleCodeActions,
        Search,
        GoToDefinition,
    ]
);

#[derive(Clone)]
pub enum InputEvent {
    Change,
    PressEnter {
        secondary: bool,
    },
    Focus,
    Blur,
    GutterMarkerMouseDown {
        marker_id: SharedString,
        logical_row: usize,
    },
}

/// A clickable marker anchored to one logical buffer row in the code-editor gutter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputGutterMarker {
    /// A stable ID supplied by the owning feature. It should encode enough
    /// information to validate the marker against a current document snapshot.
    pub id: SharedString,
    pub logical_row: usize,
    pub icon: IconName,
    pub enabled: bool,
    pub tooltip: Option<SharedString>,
    /// Execution/lifecycle state of the marker, used by the owner to reflect
    /// running/success/error/cancel without replacing the marker list.
    pub state: InputGutterMarkerState,
}

/// Lifecycle state for a gutter marker.
///
/// This mirrors the SQL execution states and lets a feature update a marker's
/// presentation without re-emitting the whole marker list on every state change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputGutterMarkerState {
    #[default]
    Idle,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl InputGutterMarker {
    pub fn new(id: impl Into<SharedString>, logical_row: usize, icon: IconName) -> Self {
        Self {
            id: id.into(),
            logical_row,
            icon,
            enabled: true,
            tooltip: None,
            state: InputGutterMarkerState::Idle,
        }
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn state(mut self, state: InputGutterMarkerState) -> Self {
        self.state = state;
        self
    }
}

/// The kind of a range decoration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputRangeDecorationKind {
    /// Highlight the executable statement the cursor currently sits in.
    #[default]
    CurrentStatement,
}

/// The presentation style for a range decoration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputRangeDecorationStyle {
    /// Draw a thin frame around the range.
    #[default]
    Frame,
    /// Draw a faint fill behind the range (preview highlight).
    Highlight,
    /// No visual presentation; kept as a no-op alternative.
    None,
}

/// A caller-owned decoration over a byte range of the document.
///
/// Ranges are UTF-8 byte offsets into the current text. Callers are
/// responsible for keeping ranges consistent with the document revision they
/// were computed from; editing invalidates all installed decorations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputRangeDecoration {
    pub id: SharedString,
    pub range: Range<usize>,
    pub kind: InputRangeDecorationKind,
    pub style: InputRangeDecorationStyle,
}

impl InputRangeDecoration {
    pub fn new(id: impl Into<SharedString>, range: Range<usize>) -> Self {
        Self {
            id: id.into(),
            range,
            kind: InputRangeDecorationKind::CurrentStatement,
            style: InputRangeDecorationStyle::Frame,
        }
    }

    pub fn style(mut self, style: InputRangeDecorationStyle) -> Self {
        self.style = style;
        self
    }
}

/// A non-editable inline hint anchored to a byte offset in the document.
///
/// The hint text is rendered immediately before the anchored offset in a muted
/// style (spec §14.4). It is not part of the document text: it cannot be
/// edited, selected, or included in copied text. Offsets are UTF-8 byte
/// offsets into the current text; editing invalidates all installed widgets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputInlineWidget {
    pub id: SharedString,
    /// Byte offset the widget is anchored before.
    pub offset: usize,
    /// Hint text rendered before the offset.
    pub text: SharedString,
}

impl InputInlineWidget {
    pub fn new(id: impl Into<SharedString>, offset: usize, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            offset,
            text: text.into(),
        }
    }
}

/// Per-line presentation overrides for code-editor inputs.
///
/// When a decoration list is installed, `line_number` replaces the default
/// one-based buffer row number. Use `None` for alignment placeholder rows that
/// should not display a number.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InputLineDecoration {
    pub line_number: Option<usize>,
    pub background: Option<Hsla>,
}

pub type InputTextHighlight = (Range<usize>, HighlightStyle);

pub type InputContextMenuActionFactory = Rc<dyn Fn() -> Box<dyn Action>>;
pub type InputContextMenuClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(Clone)]
pub enum InputContextMenuItem {
    Separator,
    Item {
        label: SharedString,
        icon: Option<Icon>,
        disabled: bool,
        action: Option<InputContextMenuActionFactory>,
        on_click: Option<InputContextMenuClickHandler>,
    },
    Submenu {
        label: SharedString,
        icon: Option<Icon>,
        disabled: bool,
        items: Vec<InputContextMenuItem>,
    },
}

impl InputContextMenuItem {
    pub fn separator() -> Self {
        Self::Separator
    }

    pub fn action<A>(label: impl Into<SharedString>, action: A) -> Self
    where
        A: Action + Clone + 'static,
    {
        Self::action_factory(label, move || Box::new(action.clone()))
    }

    pub fn action_factory(
        label: impl Into<SharedString>,
        factory: impl Fn() -> Box<dyn Action> + 'static,
    ) -> Self {
        Self::Item {
            label: label.into(),
            icon: None,
            disabled: false,
            action: Some(Rc::new(factory)),
            on_click: None,
        }
    }

    pub fn on_click(
        label: impl Into<SharedString>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self::Item {
            label: label.into(),
            icon: None,
            disabled: false,
            action: None,
            on_click: Some(Rc::new(handler)),
        }
    }

    pub fn submenu(
        label: impl Into<SharedString>,
        items: impl IntoIterator<Item = InputContextMenuItem>,
    ) -> Self {
        Self::Submenu {
            label: label.into(),
            icon: None,
            disabled: false,
            items: items.into_iter().collect(),
        }
    }

    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        match &mut self {
            InputContextMenuItem::Item {
                icon: item_icon, ..
            }
            | InputContextMenuItem::Submenu {
                icon: item_icon, ..
            } => *item_icon = Some(icon.into()),
            InputContextMenuItem::Separator => {}
        }
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        match &mut self {
            InputContextMenuItem::Item {
                disabled: is_disabled,
                ..
            }
            | InputContextMenuItem::Submenu {
                disabled: is_disabled,
                ..
            } => *is_disabled = disabled,
            InputContextMenuItem::Separator => {}
        }
        self
    }
}

pub(super) const CONTEXT: &str = "Input";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some(CONTEXT)),
        KeyBinding::new("shift-backspace", Backspace, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-backspace", Backspace, Some(CONTEXT)),
        KeyBinding::new("delete", Delete, Some(CONTEXT)),
        KeyBinding::new("shift-delete", Delete, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-backspace", DeleteToBeginningOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-delete", DeleteToEndOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-backspace", DeleteToPreviousWordStart, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-backspace", DeleteToPreviousWordStart, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-delete", DeleteToNextWordEnd, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-delete", DeleteToNextWordEnd, Some(CONTEXT)),
        KeyBinding::new("enter", Enter { secondary: false }, Some(CONTEXT)),
        KeyBinding::new("shift-enter", Enter { secondary: true }, Some(CONTEXT)),
        KeyBinding::new("secondary-enter", Enter { secondary: true }, Some(CONTEXT)),
        KeyBinding::new("escape", Escape, Some(CONTEXT)),
        KeyBinding::new("up", MoveUp, Some(CONTEXT)),
        KeyBinding::new("down", MoveDown, Some(CONTEXT)),
        KeyBinding::new("left", MoveLeft, Some(CONTEXT)),
        KeyBinding::new("right", MoveRight, Some(CONTEXT)),
        KeyBinding::new("pageup", MovePageUp, Some(CONTEXT)),
        KeyBinding::new("pagedown", MovePageDown, Some(CONTEXT)),
        KeyBinding::new("tab", IndentInline, Some(CONTEXT)),
        KeyBinding::new("shift-tab", OutdentInline, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-]", Indent, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-]", Indent, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-[", Outdent, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-[", Outdent, Some(CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(CONTEXT)),
        KeyBinding::new("shift-up", SelectUp, Some(CONTEXT)),
        KeyBinding::new("shift-down", SelectDown, Some(CONTEXT)),
        KeyBinding::new("home", MoveHome, Some(CONTEXT)),
        KeyBinding::new("end", MoveEnd, Some(CONTEXT)),
        KeyBinding::new("shift-home", SelectToStartOfLine, Some(CONTEXT)),
        KeyBinding::new("shift-end", SelectToEndOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-shift-a", SelectToStartOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-shift-e", SelectToEndOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("shift-cmd-left", SelectToStartOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("shift-cmd-right", SelectToEndOfLine, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-shift-left", SelectToPreviousWordStart, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-left", SelectToPreviousWordStart, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-shift-right", SelectToNextWordEnd, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-shift-right", SelectToNextWordEnd, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-a", SelectAll, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-a", SelectAll, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", Copy, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-c", Copy, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-x", Cut, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-x", Cut, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-v", Paste, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-v", Paste, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-a", MoveHome, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-left", MoveHome, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("ctrl-e", MoveEnd, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-right", MoveEnd, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-z", Undo, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-z", Redo, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-up", MoveToStart, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-down", MoveToEnd, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-left", MoveToPreviousWord, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("alt-right", MoveToNextWord, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-left", MoveToPreviousWord, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-right", MoveToNextWord, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-up", SelectToStart, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-shift-down", SelectToEnd, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-z", Undo, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-y", Redo, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-.", ToggleCodeActions, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-.", ToggleCodeActions, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-f", Search, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-f", Search, Some(CONTEXT)),
    ]);

    search::init(cx);
    number_input::init(cx);
}

/// Whitespace indicators for rendering spaces and tabs.
#[derive(Clone, Default)]
pub(crate) struct WhitespaceIndicators {
    /// Shaped line for space character indicator (•)
    pub(crate) space: ShapedLine,
    /// Shaped line for tab character indicator (→)
    pub(crate) tab: ShapedLine,
}

#[derive(Clone)]
pub(super) struct LastLayout {
    /// The visible range (no wrap) of lines in the viewport, the value is row (0-based) index.
    /// This is the buffer line range that encompasses all visible lines.
    pub(super) visible_range: Range<usize>,
    /// The list of visible buffer line indices (excludes hidden/folded lines).
    /// Parallel to `lines`: `visible_buffer_lines[i]` is the buffer line index of `lines[i]`.
    pub(super) visible_buffer_lines: Vec<usize>,
    /// Byte offset of each visible buffer line in the Rope (parallel to visible_buffer_lines/lines).
    pub(super) visible_line_byte_offsets: Vec<usize>,
    /// The first visible line top position in scroll viewport.
    pub(super) visible_top: Pixels,
    /// The range of byte offset of the visible lines.
    pub(super) visible_range_offset: Range<usize>,
    /// The last layout lines (Only have visible lines, no empty entries for hidden lines).
    pub(super) lines: Rc<Vec<LineLayout>>,
    /// The line_height of text layout, this will change will InputElement painted.
    pub(super) line_height: Pixels,
    /// The wrap width of text layout, this will change will InputElement painted.
    pub(super) wrap_width: Option<Pixels>,
    /// The line number area width of text layout, if not line number, this will be 0px.
    pub(super) line_number_width: Pixels,
    /// The cursor position (top, left) in pixels.
    pub(super) cursor_bounds: Option<Bounds<Pixels>>,
    /// The text align of the text layout.
    pub(super) text_align: TextAlign,
    /// The content width of the text layout.
    pub(super) content_width: Pixels,
}

impl LastLayout {
    /// Get the line layout for the given buffer row (0-based).
    ///
    /// Uses binary search on `visible_buffer_lines` to find the line.
    /// Returns None if the row is not visible (out of range or folded).
    pub(crate) fn line(&self, row: usize) -> Option<&LineLayout> {
        let pos = self.visible_buffer_lines.binary_search(&row).ok()?;
        self.lines.get(pos)
    }

    /// Get the alignment offset for the given line width.
    pub(super) fn alignment_offset(&self, line_width: Pixels) -> Pixels {
        match self.text_align {
            TextAlign::Left => px(0.),
            TextAlign::Center => (self.content_width - line_width).half().max(px(0.)),
            TextAlign::Right => (self.content_width - line_width).max(px(0.)),
        }
    }
}

/// InputState to keep editing state of the [`super::Input`].
pub struct InputState {
    pub(super) focus_handle: FocusHandle,
    pub(super) mode: InputMode,
    pub(super) text: Rope,
    pub(super) document_revision: u64,
    /// Monotonic identity for all completion requests.
    ///
    /// Incrementing this value invalidates popup and inline responses without
    /// requiring a document mutation.
    pub(super) completion_epoch: u64,
    pub(super) gutter_markers: Rc<[InputGutterMarker]>,
    /// Once gutter markers have been installed, keep their lane width reserved
    /// even while markers are transiently cleared (edits invalidate markers
    /// before the owner re-sets them). Prevents the whole text from shifting
    /// horizontally on every keystroke.
    pub(super) gutter_marker_lane_reserved: bool,
    pub(super) range_decorations: Rc<[InputRangeDecoration]>,
    pub(super) inline_widgets: Rc<[InputInlineWidget]>,
    pub(super) display_map: DisplayMap,
    pub(super) history: History<Change>,
    pub(super) blink_cursor: Entity<BlinkCursor>,
    pub(super) loading: bool,
    /// Range in UTF-8 length for the selected text.
    ///
    /// - "Hello 世界💝" = 16
    /// - "💝" = 4
    pub(super) selected_range: Selection,
    pub(super) search_panel: Option<Entity<SearchPanel>>,
    pub(super) searchable: bool,
    /// Range for save the selected word, use to keep word range when drag move.
    pub(super) selected_word_range: Option<Selection>,
    pub(super) selection_reversed: bool,
    /// The marked range is the temporary insert text on IME typing.
    pub(super) ime_marked_range: Option<Selection>,
    pub(super) last_layout: Option<LastLayout>,
    pub(super) last_cursor: Option<usize>,
    /// The input container bounds
    pub(super) input_bounds: Bounds<Pixels>,
    /// The text bounds
    pub(super) last_bounds: Option<Bounds<Pixels>>,
    pub(super) last_selected_range: Option<Selection>,
    pub(super) selecting: bool,
    pub(super) size: Size,
    pub(super) disabled: bool,
    pub(super) read_only: bool,
    pub(super) caret_color: Option<Hsla>,
    pub(super) placeholder_color: Option<Hsla>,
    pub(super) background_color: Option<Hsla>,
    pub(super) highlight_theme: Option<Arc<HighlightTheme>>,
    pub(super) indent_guide_color: Option<Hsla>,
    pub(super) line_decorations: Option<Rc<[InputLineDecoration]>>,
    pub(super) text_highlights: Rc<[InputTextHighlight]>,
    pub(super) masked: bool,
    pub(super) clean_on_escape: bool,
    pub(super) soft_wrap: bool,
    pub(super) show_whitespaces: bool,
    /// This flag tells the renderer to prefer the end of the current visual line.
    pub(crate) cursor_line_end_affinity: bool,
    pub(super) pattern: Option<regex::Regex>,
    pub(super) validate: Option<Box<dyn Fn(&str, &mut Context<Self>) -> bool + 'static>>,
    pub(crate) scroll_handle: ScrollHandle,
    /// The deferred scroll offset to apply on next layout.
    pub(crate) deferred_scroll_offset: Option<Point<Pixels>>,
    /// The size of the scrollable content.
    pub(crate) scroll_size: gpui::Size<Pixels>,
    pub(super) editor_scrollbar_paddings: Cell<Edges<Pixels>>,
    pub(super) editor_scrollbar_snapshot: Cell<Option<EditorScrollbarSnapshot>>,
    pub(super) text_align: TextAlign,
    pub(super) text_layout_margin: bool,

    /// The mask pattern for formatting the input text
    pub(crate) mask_pattern: MaskPattern,
    pub(super) placeholder: SharedString,

    /// Popover
    diagnostic_popover: Option<Entity<DiagnosticPopover>>,
    /// Completion/CodeAction context menu
    pub(super) context_menu_content: Option<ContextMenu>,
    pub(super) context_menu: Entity<InputContextMenu>,

    /// An optional context menu builder to allow a custom context menu on the input.
    ///
    /// If set, this will override the built-in context menu and ignore the value set in [`Self::enable_context_menu`].
    pub(super) context_menu_builder:
        Option<Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>>,
    pub(super) mouse_context_menu_items: Vec<InputContextMenuItem>,

    /// Whether the context menu that shows on right-click is enabled.
    ///
    /// This value will be ignored if a context menu builder is defined in [`Self::context_menu_builder`].
    pub(super) enable_context_menu: bool,

    /// A flag to indicate if we are currently inserting a completion item.
    pub(super) completion_inserting: bool,
    pub(super) hover_popover: Option<Entity<HoverPopover>>,
    /// The active signature help popover (function call argument help).
    pub(super) signature_help_popover: Option<Entity<SignatureHelpPopover>>,
    /// (cursor offset, document revision) of the last signature help request,
    /// used to coalesce requests that would resolve to identical output.
    pub(super) last_signature_help_request: Option<(usize, u64)>,
    /// The LSP definitions locations for "Go to Definition" feature.
    pub(super) hover_definition: HoverDefinition,

    pub lsp: Lsp,

    /// A flag to indicate if we have a pending update to the text.
    ///
    /// If true, will call some update (for example LSP, Syntax Highlight) before render.
    _pending_update: bool,
    /// A flag to indicate if we should ignore the next completion event.
    pub(super) silent_replace_text: bool,
    /// A flag to indicate if we should emit InputEvents.
    pub(super) emit_events: bool,

    /// To remember the horizontal column (x-coordinate) of the cursor position for keep column for move up/down.
    ///
    /// The first element is the x-coordinate (Pixels), preferred to use this.
    /// The second element is the column (usize), fallback to use this.
    pub(super) preferred_column: Option<(Pixels, usize)>,
    _subscriptions: Vec<Subscription>,

    pub(super) _context_menu_task: Task<Result<()>>,
    pub(super) inline_completion: InlineCompletion,
    pub(super) auto_pair: bool,
}

impl EventEmitter<InputEvent> for InputState {}

fn offset_after_replacement(offset: usize, range: &Range<usize>, replacement_len: usize) -> usize {
    if range.is_empty() {
        return if offset < range.start {
            offset
        } else {
            offset.saturating_add(replacement_len)
        };
    }
    if offset <= range.start {
        return offset;
    }
    if offset < range.end {
        return range.start.saturating_add(replacement_len);
    }
    offset
        .saturating_sub(range.len())
        .saturating_add(replacement_len)
}

impl InputState {
    /// Create a Input state with default [`InputMode::SingleLine`] mode.
    ///
    /// See also: [`Self::multi_line`], [`Self::auto_grow`] to set other mode.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle().tab_stop(true);
        let blink_cursor = cx.new(|_| BlinkCursor::new());
        let history = History::new().group_interval(std::time::Duration::from_secs(1));

        let _subscriptions = vec![
            // Observe the blink cursor to repaint the view when it changes.
            cx.observe(&blink_cursor, |_, _, cx| cx.notify()),
            // Blink the cursor when the window is active, pause when it's not.
            cx.observe_window_activation(window, |input, window, cx| {
                if window.is_window_active() {
                    let focus_handle = input.focus_handle.clone();
                    if focus_handle.is_focused(window) {
                        input.blink_cursor.update(cx, |blink_cursor, cx| {
                            blink_cursor.start(cx);
                        });
                    }
                } else {
                    input.invalidate_completions(cx);
                }
            }),
            cx.on_focus(&focus_handle, window, Self::on_focus),
            cx.on_blur(&focus_handle, window, Self::on_blur),
        ];

        let text_style = window.text_style();
        let mouse_context_menu = InputContextMenu::new(cx.entity(), window, cx);

        Self {
            focus_handle: focus_handle.clone(),
            text: "".into(),
            document_revision: 0,
            completion_epoch: 0,
            gutter_markers: Rc::from([]),
            gutter_marker_lane_reserved: false,
            range_decorations: Rc::from([]),
            inline_widgets: Rc::from([]),
            display_map: DisplayMap::new(text_style.font(), window.rem_size(), None),
            blink_cursor,
            history,
            selected_range: Selection::default(),
            search_panel: None,
            searchable: false,
            selected_word_range: None,
            selection_reversed: false,
            ime_marked_range: None,
            input_bounds: Bounds::default(),
            selecting: false,
            disabled: false,
            read_only: false,
            caret_color: None,
            placeholder_color: None,
            background_color: None,
            highlight_theme: None,
            indent_guide_color: None,
            line_decorations: None,
            text_highlights: Rc::from([]),
            masked: false,
            clean_on_escape: false,
            soft_wrap: true,
            show_whitespaces: false,
            loading: false,
            pattern: None,
            validate: None,
            mode: InputMode::default(),
            last_layout: None,
            last_bounds: None,
            last_selected_range: None,
            last_cursor: None,
            scroll_handle: ScrollHandle::new(),
            scroll_size: gpui::size(px(0.), px(0.)),
            editor_scrollbar_paddings: Cell::new(Edges {
                top: px(0.),
                right: px(0.),
                bottom: px(0.),
                left: px(0.),
            }),
            editor_scrollbar_snapshot: Cell::new(None),
            deferred_scroll_offset: None,
            preferred_column: None,
            placeholder: SharedString::default(),
            mask_pattern: MaskPattern::default(),
            text_align: TextAlign::Left,
            text_layout_margin: true,
            lsp: Lsp::default(),
            diagnostic_popover: None,
            context_menu_content: None,
            context_menu: mouse_context_menu,
            context_menu_builder: None,
            mouse_context_menu_items: vec![],
            enable_context_menu: true,
            completion_inserting: false,
            hover_popover: None,
            signature_help_popover: None,
            last_signature_help_request: None,
            hover_definition: HoverDefinition::default(),
            silent_replace_text: false,
            emit_events: true,
            size: Size::default(),
            _subscriptions,
            _context_menu_task: Task::ready(Ok(())),
            _pending_update: false,
            inline_completion: InlineCompletion::default(),
            cursor_line_end_affinity: false,
            auto_pair: false,
        }
    }

    /// Set Input to use multi line mode.
    ///
    /// Default rows is 2.
    pub fn multi_line(mut self, multi_line: bool) -> Self {
        self.mode = self.mode.multi_line(multi_line);
        self
    }

    /// Set Input to use [`InputMode::AutoGrow`] mode with min, max rows limit.
    pub fn auto_grow(mut self, min_rows: usize, max_rows: usize) -> Self {
        self.mode = InputMode::auto_grow(min_rows, max_rows);
        self
    }

    /// Set Input to use [`InputMode::CodeEditor`] mode.
    ///
    /// Default options:
    ///
    /// - line_number: true
    /// - tab_size: 2
    /// - hard_tabs: false
    /// - height: 100%
    /// - multi_line: true
    /// - indent_guides: true
    ///
    /// If `highlighter` is None, will use the default highlighter.
    ///
    /// Code Editor aim for help used to simple code editing or display, not a full-featured code editor.
    ///
    /// ## Features
    ///
    /// - Syntax Highlighting
    /// - Auto Indent
    /// - Line Number
    /// - Large Text support, up to 50K lines.
    pub fn code_editor(mut self, language: impl Into<SharedString>) -> Self {
        let language: SharedString = language.into();
        self.mode = InputMode::code_editor(language);
        self.searchable = true;
        self.auto_pair = true;
        self
    }

    /// Switch an existing input to the lightweight code editor mode.
    pub fn set_code_editor_mode(
        &mut self,
        language: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut mode = InputMode::code_editor(language);
        if let InputMode::CodeEditor {
            line_number,
            folding,
            ..
        } = &mut mode
        {
            *line_number = false;
            *folding = false;
        }
        self.mode = mode;
        self.searchable = true;
        self.auto_pair = true;
        self.set_soft_wrap(false, window, cx);
    }

    /// Switch an existing input back to the multi-line rich text mode.
    pub fn set_rich_text_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = InputMode::plain_text().multi_line(true);
        self.searchable = false;
        self.auto_pair = false;
        self.set_soft_wrap(true, window, cx);
    }

    /// Make the current multi-line mode grow with its wrapped content.
    ///
    /// Code editor mode keeps its syntax highlighter and indentation behavior.
    pub fn set_auto_grow_mode(
        &mut self,
        min_rows: usize,
        max_rows: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode.enable_auto_grow(min_rows, max_rows);
        self.mode.update_auto_grow(&self.display_map);
        self.set_soft_wrap(true, window, cx);
    }

    /// Set this input to read-only while preserving focus, selection, search,
    /// copying, and scrolling.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Use a shared scroll handle for this input.
    ///
    /// Multiple editors that share a handle keep both axes synchronized.
    pub fn shared_scroll_handle(mut self, scroll_handle: ScrollHandle) -> Self {
        self.scroll_handle = scroll_handle;
        self
    }

    /// Override line numbers and backgrounds for individual buffer rows.
    pub fn line_decorations(mut self, decorations: impl Into<Rc<[InputLineDecoration]>>) -> Self {
        self.line_decorations = Some(decorations.into());
        self
    }

    /// Update line presentation overrides at runtime.
    pub fn set_line_decorations(
        &mut self,
        decorations: impl Into<Rc<[InputLineDecoration]>>,
        cx: &mut Context<Self>,
    ) {
        self.line_decorations = Some(decorations.into());
        cx.notify();
    }

    pub fn set_text_highlights(
        &mut self,
        highlights: impl Into<Rc<[InputTextHighlight]>>,
        cx: &mut Context<Self>,
    ) {
        let highlights = highlights.into();
        if self.text_highlights.as_ref() == highlights.as_ref() {
            return;
        }
        self.text_highlights = highlights;
        cx.notify();
    }

    /// 设置是否启用括号自动配对。
    pub fn auto_pair(mut self, enabled: bool) -> Self {
        self.auto_pair = enabled;
        self
    }

    /// 设置是否启用括号自动配对（运行时修改）。
    pub fn set_auto_pair(&mut self, enabled: bool) {
        self.auto_pair = enabled;
    }

    /// Sets whether the context menu that shows on right-click is enabled.
    ///
    /// The context menu is enabled by default.
    /// This value will be ignored if a custom context menu is defined on the input.
    pub fn context_menu(mut self, enable: bool) -> Self {
        self.enable_context_menu = enable;
        self
    }

    /// 为右键菜单设置额外菜单项，支持一级和二级菜单。
    pub fn mouse_context_menu_items(mut self, items: Vec<InputContextMenuItem>) -> Self {
        self.mouse_context_menu_items = items;
        self
    }

    /// Set this input is searchable, default is false (Default true for Code Editor).
    pub fn searchable(mut self, searchable: bool) -> Self {
        debug_assert!(self.mode.is_multi_line());
        self.searchable = searchable;
        self
    }

    /// Set placeholder
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Set enable/disable code folding, only for [`InputMode::CodeEditor`] mode.
    ///
    /// Default: true
    pub fn folding(mut self, folding: bool) -> Self {
        debug_assert!(self.mode.is_code_editor());
        if let InputMode::CodeEditor { folding: f, .. } = &mut self.mode {
            *f = folding;
        }
        self
    }

    /// Set code folding at runtime, only for [`InputMode::CodeEditor`] mode.
    ///
    /// When disabling, all existing folds are cleared.
    pub fn set_folding(&mut self, folding: bool, _: &mut Window, cx: &mut Context<Self>) {
        debug_assert!(self.mode.is_code_editor());
        if let InputMode::CodeEditor { folding: f, .. } = &mut self.mode {
            *f = folding;
        }
        if !folding {
            self.display_map.clear_folds();
        }
        cx.notify();
    }

    /// Set enable/disable line number, only for [`InputMode::CodeEditor`] mode.
    pub fn line_number(mut self, line_number: bool) -> Self {
        debug_assert!(self.mode.is_code_editor() && self.mode.is_multi_line());
        if let InputMode::CodeEditor { line_number: l, .. } = &mut self.mode {
            *l = line_number;
        }
        self
    }

    /// Set line number, only for [`InputMode::CodeEditor`] mode.
    pub fn set_line_number(&mut self, line_number: bool, _: &mut Window, cx: &mut Context<Self>) {
        debug_assert!(self.mode.is_code_editor() && self.mode.is_multi_line());
        if let InputMode::CodeEditor { line_number: l, .. } = &mut self.mode {
            *l = line_number;
        }
        cx.notify();
    }

    /// Set the number of rows for the multi-line Textarea.
    ///
    /// This is only used when `multi_line` is set to true.
    ///
    /// default: 2
    pub fn rows(mut self, rows: usize) -> Self {
        match &mut self.mode {
            InputMode::PlainText { rows: r, .. } | InputMode::CodeEditor { rows: r, .. } => {
                *r = rows
            }
            InputMode::AutoGrow {
                max_rows: max_r,
                rows: r,
                ..
            } => {
                *r = rows;
                *max_r = rows;
            }
        }
        self
    }

    /// Set highlighter language for for [`InputMode::CodeEditor`] mode.
    pub fn set_highlighter(
        &mut self,
        new_language: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        match &mut self.mode {
            InputMode::CodeEditor {
                language,
                highlighter,
                parse_task,
                ..
            } => {
                *language = new_language.into();
                *highlighter.borrow_mut() = None;
                parse_task.borrow_mut().take();
            }
            _ => {}
        }
        cx.notify();
    }

    fn reset_highlighter(&mut self, cx: &mut Context<Self>) {
        match &mut self.mode {
            InputMode::CodeEditor {
                highlighter,
                parse_task,
                ..
            } => {
                *highlighter.borrow_mut() = None;
                parse_task.borrow_mut().take();
            }
            _ => {}
        }
        cx.notify();
    }

    #[inline]
    pub fn diagnostics(&self) -> Option<&DiagnosticSet> {
        self.mode.diagnostics()
    }

    #[inline]
    pub fn diagnostics_mut(&mut self) -> Option<&mut DiagnosticSet> {
        self.mode.diagnostics_mut()
    }

    /// Set placeholder
    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// 运行时更新右键菜单的额外菜单项。
    pub fn set_mouse_context_menu_items(
        &mut self,
        items: Vec<InputContextMenuItem>,
        cx: &mut Context<Self>,
    ) {
        self.mouse_context_menu_items = items;
        cx.notify();
    }

    /// 追加一个右键菜单项。
    pub fn add_mouse_context_menu_item(
        &mut self,
        item: InputContextMenuItem,
        cx: &mut Context<Self>,
    ) {
        self.mouse_context_menu_items.push(item);
        cx.notify();
    }

    /// 清空右键菜单的额外菜单项。
    pub fn clear_mouse_context_menu_items(&mut self, cx: &mut Context<Self>) {
        self.mouse_context_menu_items.clear();
        cx.notify();
    }

    /// Find which line and sub-line the given offset belongs to, along with the position within that sub-line.
    ///
    /// Returns:
    ///
    /// - The index of the line (zero-based) containing the offset.
    /// - The index of the sub-line (zero-based) within the line containing the offset.
    /// - The position of the offset.
    pub(super) fn line_and_position_for_offset(
        &self,
        offset: usize,
    ) -> (usize, usize, Option<Point<Pixels>>) {
        let Some(last_layout) = &self.last_layout else {
            return (0, 0, None);
        };
        let line_height = last_layout.line_height;

        let mut y_offset = last_layout.visible_top;
        for (vi, line) in last_layout.lines.iter().enumerate() {
            let prev_lines_offset = last_layout.visible_line_byte_offsets[vi];
            let local_offset = offset.saturating_sub(prev_lines_offset);
            if let Some(pos) = line.position_for_index(local_offset, last_layout, false) {
                let sub_line_index = (pos.y / line_height) as usize;
                let adjusted_pos = point(pos.x + last_layout.line_number_width, pos.y + y_offset);
                return (vi, sub_line_index, Some(adjusted_pos));
            }

            y_offset += line.size(line_height).height;
        }
        (0, 0, None)
    }

    /// Set the text of the input field.
    ///
    /// And the selection_range will be reset to 0..0.
    pub fn set_value(
        &mut self,
        value: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.history.ignore = true;
        self.emit_events = false;
        let previous_revision = self.document_revision;
        self.replace_text(value, window, cx);
        // A same-value set is model-to-view synchronization, not a content edit.
        if self.document_revision == previous_revision {
            self.range_decorations = Rc::from([]);
            self.inline_widgets = Rc::from([]);
        }
        self.history.ignore = false;
        self.emit_events = true;

        // Ensure cursor to start when set text
        if self.mode.is_single_line() {
            self.selected_range = (self.text.len()..self.text.len()).into();
        } else {
            self.selected_range.clear();
        }

        if self.mode.is_code_editor() {
            self._pending_update = true;
            self.lsp.reset();
        }

        // Move scroll to top
        self.scroll_handle.set_offset(point(px(0.), px(0.)));

        self.history.clear();
        cx.notify();
    }

    /// Replace one UTF-8 text range without resetting scroll, selection, or history.
    ///
    /// This is intended for model-to-view synchronization where the caller applies
    /// a minimal diff and then restores the desired selection explicitly.
    pub fn replace_text_range(
        &mut self,
        range: Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = range.start.min(self.text.len());
        let end = range.end.min(self.text.len()).max(start);
        let range = start..end;
        let selection = Range::<usize>::from(self.selected_range);
        let selection_reversed = self.selection_reversed;
        let scroll_offset = self.scroll_handle.offset();
        let range_utf16 = self.range_to_utf16(&range);
        self.history.ignore = true;
        self.emit_events = false;
        self.replace_text_in_range_silent(Some(range_utf16), text, window, cx);
        self.emit_events = true;
        self.history.ignore = false;
        let selected_range = offset_after_replacement(selection.start, &range, text.len())
            ..offset_after_replacement(selection.end, &range, text.len());
        self.selected_range = selected_range.clone().into();
        self.selection_reversed = selection_reversed && !selected_range.is_empty();
        self.scroll_handle.set_offset(scroll_offset);
        self.deferred_scroll_offset = Some(scroll_offset);
    }

    /// Insert text at the current cursor position.
    ///
    /// And the cursor will be moved to the end of inserted text.
    pub fn insert(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_disabled = self.disabled;
        let was_read_only = self.read_only;
        self.disabled = false;
        self.read_only = false;
        let text: SharedString = text.into();
        let range_utf16 = self.range_to_utf16(&(self.cursor()..self.cursor()));
        self.replace_text_in_range_silent(Some(range_utf16), &text, window, cx);
        self.selected_range = (self.selected_range.end..self.selected_range.end).into();
        self.disabled = was_disabled;
        self.read_only = was_read_only;
    }

    /// Replace text at the current cursor position.
    ///
    /// And the cursor will be moved to the end of replaced text.
    pub fn replace(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_disabled = self.disabled;
        let was_read_only = self.read_only;
        self.disabled = false;
        self.read_only = false;
        let text: SharedString = text.into();
        self.replace_text_in_range_silent(None, &text, window, cx);
        self.selected_range = (self.selected_range.end..self.selected_range.end).into();
        self.disabled = was_disabled;
        self.read_only = was_read_only;
    }

    fn replace_text(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_disabled = self.disabled;
        let was_read_only = self.read_only;
        self.disabled = false;
        self.read_only = false;
        let text: SharedString = text.into();
        let range = 0..self.text.chars().map(|c| c.len_utf16()).sum();
        self.replace_text_in_range_silent(Some(range), &text, window, cx);
        self.reset_highlighter(cx);
        self.disabled = was_disabled;
        self.read_only = was_read_only;
    }

    /// Set with disabled mode.
    ///
    /// See also: [`Self::set_disabled`], [`Self::is_disabled`].
    #[allow(unused)]
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set with password masked state.
    ///
    /// Only for [`InputMode::SingleLine`] mode.
    pub fn masked(mut self, masked: bool) -> Self {
        debug_assert!(self.mode.is_single_line());
        self.masked = masked;
        self
    }

    /// Set the password masked state of the input field.
    ///
    /// Only for [`InputMode::SingleLine`] mode.
    pub fn set_masked(&mut self, masked: bool, _: &mut Window, cx: &mut Context<Self>) {
        debug_assert!(self.mode.is_single_line());
        self.masked = masked;
        cx.notify();
    }

    /// Set true to clear the input by pressing Escape key.
    pub fn clean_on_escape(mut self) -> Self {
        self.clean_on_escape = true;
        self
    }

    /// Set the soft wrap mode for multi-line input, default is true.
    pub fn soft_wrap(mut self, wrap: bool) -> Self {
        debug_assert!(self.mode.is_multi_line());
        self.soft_wrap = wrap;
        self
    }

    /// Set whether to show whitespace characters.
    pub fn show_whitespaces(mut self, show: bool) -> Self {
        self.show_whitespaces = show;
        self
    }

    /// Update the soft wrap mode for multi-line input, default is true.
    pub fn set_soft_wrap(&mut self, wrap: bool, _: &mut Window, cx: &mut Context<Self>) {
        debug_assert!(self.mode.is_multi_line());
        self.soft_wrap = wrap;
        if wrap {
            let wrap_width = self
                .last_layout
                .as_ref()
                .and_then(|b| b.wrap_width)
                .unwrap_or(self.input_bounds.size.width);

            self.display_map.on_layout_changed(Some(wrap_width), cx);

            // Reset scroll to left 0
            let mut offset = self.scroll_handle.offset();
            offset.x = px(0.);
            self.scroll_handle.set_offset(offset);
        } else {
            self.display_map.on_layout_changed(None, cx);
        }
        cx.notify();
    }

    /// Update whether to show whitespace characters.
    pub fn set_show_whitespaces(&mut self, show: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.show_whitespaces = show;
        cx.notify();
    }

    /// Set the regular expression pattern of the input field.
    ///
    /// Only for [`InputMode::SingleLine`] mode.
    pub fn pattern(mut self, pattern: regex::Regex) -> Self {
        debug_assert!(self.mode.is_single_line());
        self.pattern = Some(pattern);
        self
    }

    /// Set the regular expression pattern of the input field with reference.
    ///
    /// Only for [`InputMode::SingleLine`] mode.
    pub fn set_pattern(
        &mut self,
        pattern: regex::Regex,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        debug_assert!(self.mode.is_single_line());
        self.pattern = Some(pattern);
    }

    /// Set the validation function of the input field.
    ///
    /// Only for [`InputMode::SingleLine`] mode.
    pub fn validate(mut self, f: impl Fn(&str, &mut Context<Self>) -> bool + 'static) -> Self {
        debug_assert!(self.mode.is_single_line());
        self.validate = Some(Box::new(f));
        self
    }

    /// Set true to show spinner at the input right.
    ///
    /// Only for [`InputMode::SingleLine`] mode.
    pub fn set_loading(&mut self, loading: bool, _: &mut Window, cx: &mut Context<Self>) {
        debug_assert!(self.mode.is_single_line());
        self.loading = loading;
        cx.notify();
    }

    /// Set the default value of the input field.
    pub fn default_value(mut self, value: impl Into<SharedString>) -> Self {
        let text: SharedString = value.into();
        self.text = Rope::from(text.as_str());
        if let Some(diagnostics) = self.mode.diagnostics_mut() {
            diagnostics.reset(&self.text)
        }
        // Note: We can't call display_map.set_text here because it needs cx.
        // The text will be set during prepare_if_need in element.rs
        self._pending_update = true;
        self
    }

    /// Return the value of the input field.
    pub fn value(&self) -> SharedString {
        SharedString::new(self.text.to_string())
    }

    /// Return the document content revision.
    ///
    /// Selection, focus, scrolling, and presentation-only changes do not alter
    /// this value. Every successful text mutation increments it.
    pub fn document_revision(&self) -> u64 {
        self.document_revision
    }

    /// Return the current popup/inline completion invalidation epoch.
    ///
    /// This is exposed for owners that need to bind asynchronous SQL metadata
    /// or completion work to the same generic completion lifetime.
    pub fn completion_epoch(&self) -> u64 {
        self.completion_epoch
    }

    /// Invalidate in-flight popup and inline completion requests.
    ///
    /// This is used for editor deactivation, IME composition, cursor context
    /// changes, and metadata/scope invalidation. It intentionally does not
    /// alter the document revision.
    pub fn invalidate_completions(&mut self, cx: &mut Context<Self>) {
        self.completion_epoch = self.completion_epoch.saturating_add(1);
        self.context_menu_content = None;
        self._context_menu_task = Task::ready(Ok(()));
        self.clear_inline_completion(cx);
    }

    /// Replace the generic gutter markers.
    ///
    /// Markers are owner-managed: they stay visible across document edits
    /// (avoids flicker) until the owner replaces them. Owners should encode
    /// the document revision into marker ids so clicks on stale markers can
    /// be rejected.
    pub fn set_gutter_markers(&mut self, markers: Vec<InputGutterMarker>, cx: &mut Context<Self>) {
        self.gutter_markers = Rc::from(markers);
        self.gutter_marker_lane_reserved = true;
        cx.notify();
    }

    pub fn gutter_markers(&self) -> &[InputGutterMarker] {
        &self.gutter_markers
    }

    /// Replace the caller-owned range decorations.
    ///
    /// Decorations are invalidated automatically when the document content
    /// changes, matching the gutter marker contract.
    pub fn set_range_decorations(
        &mut self,
        decorations: Vec<InputRangeDecoration>,
        cx: &mut Context<Self>,
    ) {
        self.range_decorations = Rc::from(decorations);
        cx.notify();
    }

    pub fn range_decorations(&self) -> &[InputRangeDecoration] {
        &self.range_decorations
    }

    /// Replace the caller-owned inline widgets.
    ///
    /// Widgets are invalidated automatically when the document content changes,
    /// matching the decoration/marker contract.
    pub fn set_inline_widgets(&mut self, widgets: Vec<InputInlineWidget>, cx: &mut Context<Self>) {
        self.inline_widgets = Rc::from(widgets);
        cx.notify();
    }

    pub fn inline_widgets(&self) -> &[InputInlineWidget] {
        &self.inline_widgets
    }

    /// Return the portion of the value within the input field that
    /// is selected by the user
    pub fn selected_value(&self) -> SharedString {
        SharedString::new(self.selected_text().to_string())
    }

    /// Get the currently selected text as a String.
    /// Returns an empty string if no text is selected.
    pub fn selected_text_string(&self) -> String {
        if self.selected_range.is_empty() {
            return String::new();
        }
        let text_len = self.text.len();
        if self.selected_range.end > text_len {
            return String::new();
        }
        self.text.slice(self.selected_range).to_string()
    }

    /// Return the value without mask.
    pub fn unmask_value(&self) -> SharedString {
        self.mask_pattern.unmask(&self.text.to_string()).into()
    }

    /// Return the text [`Rope`] of the input field.
    pub fn text(&self) -> &Rope {
        &self.text
    }

    /// Returns the bounds assigned to the text element during its latest paint.
    ///
    /// This is useful for embedding components that need to verify that the editor received the
    /// same horizontal constraint as its container. It is `None` until the input has been painted.
    pub fn laid_out_text_bounds(&self) -> Option<Bounds<Pixels>> {
        self.last_bounds
    }

    /// Returns the latest visible input bounds, excluding line-number and editor padding gutters.
    pub fn laid_out_input_bounds(&self) -> Bounds<Pixels> {
        self.input_bounds
    }

    /// Return the (0-based) [`Position`] of the cursor.
    pub fn cursor_position(&self) -> Position {
        let offset = self.cursor();
        self.text.offset_to_position(offset)
    }

    /// Set (0-based) [`Position`] of the cursor.
    ///
    /// This will move the cursor to the specified line and column, and update the selection range.
    pub fn set_cursor_position(
        &mut self,
        position: impl Into<Position>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position: Position = position.into();
        let offset = self.text.position_to_offset(&position);

        self.move_to(offset, None, cx);
        self.update_preferred_column();
        self.focus(window, cx);
    }

    pub fn set_selected_range(
        &mut self,
        range: Range<usize>,
        reversed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = self
            .text
            .clip_offset(range.start.min(self.text.len()), Bias::Left);
        let end = self
            .text
            .clip_offset(range.end.min(self.text.len()), Bias::Right);
        self.selected_range = (start.min(end)..start.max(end)).into();
        self.selection_reversed = reversed && start != end;
        self.update_preferred_column();
        self.focus(window, cx);
        cx.notify();
    }

    /// Focus the input field.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        self.blink_cursor.update(cx, |cursor, cx| {
            cursor.start(cx);
        });
    }

    /// Whether this input currently owns focus.
    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub(super) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor()), cx);
    }

    pub(super) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor()), cx);
    }

    pub(super) fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        if self.mode.is_single_line() {
            return;
        }
        let offset = self.start_of_line().saturating_sub(1);
        self.select_to(self.previous_boundary(offset), cx);
    }

    pub(super) fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        if self.mode.is_single_line() {
            return;
        }
        let offset = (self.end_of_line() + 1).min(self.text.len());
        self.select_to(self.next_boundary(offset), cx);
    }

    pub(super) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = (0..self.text.len()).into();
        cx.notify();
    }

    pub(super) fn select_to_start(
        &mut self,
        _: &SelectToStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(0, cx);
    }

    pub(super) fn select_to_end(
        &mut self,
        _: &SelectToEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let end = self.text.len();
        self.select_to(end, cx);
    }

    pub(super) fn select_to_start_of_line(
        &mut self,
        _: &SelectToStartOfLine,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = self.start_of_line();
        self.select_to(offset, cx);
    }

    pub(super) fn select_to_end_of_line(
        &mut self,
        _: &SelectToEndOfLine,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = self.end_of_line();
        self.select_to(offset, cx);
    }

    pub(super) fn select_to_previous_word(
        &mut self,
        _: &SelectToPreviousWordStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = self.previous_start_of_word();
        self.select_to(offset, cx);
    }

    pub(super) fn select_to_next_word(
        &mut self,
        _: &SelectToNextWordEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = self.next_end_of_word();
        self.select_to(offset, cx);
    }

    /// Return the start offset of the previous word.
    pub(super) fn previous_start_of_word(&mut self) -> usize {
        let offset = self.selected_range.start;
        let offset = self.offset_from_utf16(self.offset_to_utf16(offset));
        // FIXME: Avoid to_string
        let left_part = self.text.slice(0..offset).to_string();

        UnicodeSegmentation::split_word_bound_indices(left_part.as_str())
            .rfind(|(_, s)| !s.trim_start().is_empty())
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    /// Return the next end offset of the next word.
    pub(super) fn next_end_of_word(&mut self) -> usize {
        let offset = self.cursor();
        let offset = self.offset_from_utf16(self.offset_to_utf16(offset));
        let right_part = self.text.slice(offset..self.text.len()).to_string();

        UnicodeSegmentation::split_word_bound_indices(right_part.as_str())
            .find(|(_, s)| !s.trim_start().is_empty())
            .map(|(i, s)| offset + i + s.len())
            .unwrap_or(self.text.len())
    }

    /// Get start of line byte offset of cursor.
    ///
    /// When soft wrap is active, first press goes to visual line start,
    /// second press (already at visual start) goes to logical line start.
    pub(super) fn start_of_line(&self) -> usize {
        if self.mode.is_single_line() {
            return 0;
        }

        let row = self.text.offset_to_point(self.cursor()).row;
        let logical_start = self.text.line_start_offset(row);

        if self.soft_wrap && self.mode.is_code_editor() {
            let wrap_point = self.display_map.offset_to_wrap_display_point(self.cursor());
            if let Some(line) = self.display_map.lines().get(row)
                && let Some(range) = line.wrapped_lines.get(wrap_point.local_row)
            {
                let visual_start = logical_start + range.start;
                if self.cursor() != visual_start {
                    return visual_start;
                }
            }
        }

        logical_start
    }

    /// Get end of line byte offset of cursor.
    ///
    /// When soft wrap is active, first press goes to visual line end,
    /// second press (already at visual end) goes to logical line end.
    pub(super) fn end_of_line(&self) -> usize {
        if self.mode.is_single_line() {
            return self.text.len();
        }

        let row = self.text.offset_to_point(self.cursor()).row;
        let logical_start = self.text.line_start_offset(row);
        let logical_end = self.text.line_end_offset(row);

        if self.soft_wrap && self.mode.is_code_editor() {
            let wrap_point = self.display_map.offset_to_wrap_display_point(self.cursor());
            if let Some(line) = self.display_map.lines().get(row)
                && let Some(range) = line.wrapped_lines.get(wrap_point.local_row)
            {
                let visual_end = logical_start + range.end;
                if self.cursor() != visual_end {
                    return visual_end;
                }
            }
        }

        logical_end
    }

    /// Get start line of selection start or end (The min value).
    ///
    /// This is means is always get the first line of selection.
    pub(super) fn start_of_line_of_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        if self.mode.is_single_line() {
            return 0;
        }

        let mut offset =
            self.previous_boundary(self.selected_range.start.min(self.selected_range.end));
        if self.text.char_at(offset) == Some('\r') {
            offset += 1;
        }

        let line = self
            .text_for_range(self.range_to_utf16(&(0..offset + 1)), &mut None, window, cx)
            .unwrap_or_default()
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        line
    }

    /// Get indent string of next line.
    ///
    /// To get current and next line indent, to return more depth one.
    pub(super) fn indent_of_next_line(&mut self) -> String {
        if self.mode.is_single_line() {
            return "".into();
        }

        let mut current_indent = String::new();
        let mut next_indent = String::new();
        let current_line_start_pos = self.start_of_line();
        let next_line_start_pos = self.end_of_line();
        for c in self.text.slice(current_line_start_pos..).chars() {
            if !c.is_whitespace() {
                break;
            }
            if c == '\n' || c == '\r' {
                break;
            }
            current_indent.push(c);
        }

        for c in self.text.slice(next_line_start_pos..).chars() {
            if !c.is_whitespace() {
                break;
            }
            if c == '\n' || c == '\r' {
                break;
            }
            next_indent.push(c);
        }

        if next_indent.len() > current_indent.len() {
            return next_indent;
        } else {
            return current_indent;
        }
    }

    pub(super) fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor()), cx)
        }
        self.replace_text_in_range(None, "", window, cx);
        self.pause_blink_cursor(cx);
    }

    pub(super) fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor()), cx)
        }
        self.replace_text_in_range(None, "", window, cx);
        self.pause_blink_cursor(cx);
    }

    pub(super) fn delete_to_beginning_of_line(
        &mut self,
        _: &DeleteToBeginningOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selected_range.is_empty() {
            self.replace_text_in_range(None, "", window, cx);
            self.pause_blink_cursor(cx);
            return;
        }

        let mut offset = self.start_of_line();
        if offset == self.cursor() {
            offset = offset.saturating_sub(1);
        }
        self.replace_text_in_range_silent(
            Some(self.range_to_utf16(&(offset..self.cursor()))),
            "",
            window,
            cx,
        );
        self.pause_blink_cursor(cx);
    }

    pub(super) fn delete_to_end_of_line(
        &mut self,
        _: &DeleteToEndOfLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selected_range.is_empty() {
            self.replace_text_in_range(None, "", window, cx);
            self.pause_blink_cursor(cx);
            return;
        }

        let mut offset = self.end_of_line();
        if offset == self.cursor() {
            offset = (offset + 1).clamp(0, self.text.len());
        }
        self.replace_text_in_range_silent(
            Some(self.range_to_utf16(&(self.cursor()..offset))),
            "",
            window,
            cx,
        );
        self.pause_blink_cursor(cx);
    }

    pub(super) fn delete_previous_word(
        &mut self,
        _: &DeleteToPreviousWordStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selected_range.is_empty() {
            self.replace_text_in_range(None, "", window, cx);
            self.pause_blink_cursor(cx);
            return;
        }

        let offset = self.previous_start_of_word();
        self.replace_text_in_range_silent(
            Some(self.range_to_utf16(&(offset..self.cursor()))),
            "",
            window,
            cx,
        );
        self.pause_blink_cursor(cx);
    }

    pub(super) fn delete_next_word(
        &mut self,
        _: &DeleteToNextWordEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selected_range.is_empty() {
            self.replace_text_in_range(None, "", window, cx);
            self.pause_blink_cursor(cx);
            return;
        }

        let offset = self.next_end_of_word();
        self.replace_text_in_range_silent(
            Some(self.range_to_utf16(&(self.cursor()..offset))),
            "",
            window,
            cx,
        );
        self.pause_blink_cursor(cx);
    }

    pub(super) fn enter(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.handle_action_for_context_menu(Box::new(action.clone()), window, cx) {
            return;
        }

        // Clear inline completion on enter (user chose not to accept it)
        if self.has_inline_completion() {
            self.clear_inline_completion(cx);
        }

        if self.mode.is_multi_line() {
            // Get current line indent
            let indent = if self.mode.is_code_editor() {
                self.indent_of_next_line()
            } else {
                "".to_string()
            };

            // Add newline and indent
            let new_line_text = format!("\n{}", indent);
            self.replace_text_in_range_silent(None, &new_line_text, window, cx);
            self.pause_blink_cursor(cx);
        } else {
            // Single line input, just emit the event (e.g.: In a dialog to confirm).
            cx.propagate();
        }

        cx.emit(InputEvent::PressEnter {
            secondary: action.secondary,
        });
    }

    pub(super) fn clean(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text("", window, cx);
        self.selected_range = (0..0).into();
        self.scroll_to(0, None, cx);
    }

    pub(super) fn escape(&mut self, action: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.handle_action_for_context_menu(Box::new(action.clone()), window, cx) {
            return;
        }

        // Clear inline completion on escape
        if self.has_inline_completion() {
            self.clear_inline_completion(cx);
            return; // Consume the escape, don't propagate
        }

        if self.ime_marked_range.is_some() {
            self.unmark_text(window, cx);
        }

        if self.clean_on_escape {
            return self.clean(window, cx);
        }

        cx.propagate();
    }

    pub(super) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Clear inline completion on any mouse interaction
        self.clear_inline_completion(cx);

        // If there have IME marked range and is empty (Means pressed Esc to abort IME typing)
        // Clear the marked range.
        if let Some(ime_marked_range) = &self.ime_marked_range {
            if ime_marked_range.len() == 0 {
                self.ime_marked_range = None;
            }
        }

        self.selecting = true;
        let offset = self.index_for_mouse_position(event.position);

        if self.handle_click_hover_definition(event, offset, window, cx) {
            return;
        }

        // Triple click to select line
        if event.button == MouseButton::Left && event.click_count >= 3 {
            self.select_line(offset, window, cx);
            return;
        }

        // Double click to select word
        if event.button == MouseButton::Left && event.click_count == 2 {
            self.select_word(offset, window, cx);
            return;
        }

        // Show Mouse context menu
        if event.button == MouseButton::Right {
            if self.enable_context_menu || self.context_menu_builder.is_some() {
                self.handle_right_click_menu(event, offset, window, cx);
            }
            return;
        }

        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, None, cx)
        }
    }

    pub(super) fn on_mouse_up(
        &mut self,
        _: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.selection_reversed = false;
        }
        self.selecting = false;
        self.selected_word_range = None;
    }

    pub(super) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Check if mouse is within bounds
        let within_bounds = self
            .last_bounds
            .as_ref()
            .map(|bounds| bounds.contains(&event.position))
            .unwrap_or(false);

        if !within_bounds {
            // Clear hover when mouse leaves the input
            self.clear_hover_state(cx);
            return;
        }

        // Show diagnostic popover on mouse move
        let offset = self.index_for_mouse_position(event.position);
        self.handle_mouse_move(offset, event, window, cx);

        if self.mode.is_code_editor() {
            if let Some(diagnostic) = self
                .mode
                .diagnostics()
                .and_then(|set| set.for_offset(offset))
            {
                if let Some(diagnostic_popover) = self.diagnostic_popover.as_ref() {
                    if diagnostic_popover.read(cx).diagnostic.range == diagnostic.range {
                        diagnostic_popover.update(cx, |this, cx| {
                            this.show(cx);
                        });

                        return;
                    }
                }

                self.diagnostic_popover = Some(DiagnosticPopover::new(diagnostic, cx.entity(), cx));
                cx.notify();
            } else {
                if let Some(diagnostic_popover) = self.diagnostic_popover.as_mut() {
                    diagnostic_popover.update(cx, |this, cx| {
                        this.check_to_hide(event.position, cx);
                    })
                }
            }
        }
    }

    pub(super) fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line_height = self
            .last_layout
            .as_ref()
            .map(|layout| layout.line_height)
            .unwrap_or(window.line_height());
        let delta = event.delta.pixel_delta(line_height);

        let old_offset = self.scroll_handle.offset();
        self.update_scroll_offset(Some(old_offset + delta), cx);

        // Only stop propagation if the offset actually changed
        if self.scroll_handle.offset() != old_offset {
            cx.stop_propagation();
        }

        self.diagnostic_popover = None;
    }

    pub(super) fn update_scroll_offset(
        &mut self,
        offset: Option<Point<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        let mut offset = offset.unwrap_or(self.scroll_handle.offset());
        // In addition to left alignment, a cursor position will be reserved on the right side
        let safe_x_offset = if self.text_align == TextAlign::Left {
            px(0.)
        } else {
            -CURSOR_WIDTH
        };

        let safe_y_range =
            (-self.scroll_size.height + self.input_bounds.size.height).min(px(0.0))..px(0.);
        let safe_x_range = (-self.scroll_size.width + self.input_bounds.size.width + safe_x_offset)
            .min(safe_x_offset)..px(0.);

        offset.y = if self.mode.is_single_line() {
            px(0.)
        } else {
            offset.y.clamp(safe_y_range.start, safe_y_range.end)
        };
        offset.x = offset.x.clamp(safe_x_range.start, safe_x_range.end);
        self.scroll_handle.set_offset(offset);
        cx.notify();
    }

    /// Scroll to make the given offset visible.
    ///
    /// If `direction` is Some, will keep edges at the same side.
    pub(crate) fn scroll_to(
        &mut self,
        offset: usize,
        direction: Option<MoveDirection>,
        cx: &mut Context<Self>,
    ) {
        let Some(last_layout) = self.last_layout.as_ref() else {
            return;
        };
        let Some(bounds) = self.last_bounds.as_ref() else {
            return;
        };

        let mut scroll_offset = self.scroll_handle.offset();
        let was_offset = scroll_offset;
        let line_height = last_layout.line_height;

        let point = self.text.offset_to_point(offset);

        let row = point.row;

        let mut row_offset_y = px(0.);
        for (ix, _wrap_line) in self.display_map.lines().iter().enumerate() {
            if ix == row {
                break;
            }

            // Only accumulate height for visible (non-folded) wrap rows
            let visible_wrap_rows = self.display_map.visible_wrap_row_count_for_buffer_line(ix);
            row_offset_y += line_height * visible_wrap_rows;
        }

        // For Right alignment use 0 margin: the cursor indicator is clamped inside bounds
        // in layout_cursor, so shifting the text here would cause a first-click visual jump.
        let safety_margin = match last_layout.text_align {
            TextAlign::Left if self.text_layout_margin => RIGHT_MARGIN,
            TextAlign::Left => px(0.),
            TextAlign::Right => px(0.),
            TextAlign::Center => CURSOR_WIDTH,
        };
        if let Some(line) = last_layout
            .lines
            .get(row.saturating_sub(last_layout.visible_range.start))
        {
            // Check to scroll horizontally and soft wrap lines
            if let Some(pos) = line.position_for_index(point.column, last_layout, false) {
                let bounds_width = bounds.size.width - last_layout.line_number_width;
                let col_offset_x = pos.x;
                row_offset_y += pos.y;
                if col_offset_x - safety_margin < -scroll_offset.x {
                    // If the position is out of the visible area, scroll to make it visible
                    scroll_offset.x = -col_offset_x + safety_margin;
                } else if col_offset_x + safety_margin > -scroll_offset.x + bounds_width {
                    scroll_offset.x = -(col_offset_x - bounds_width + safety_margin);
                }
            }
        }

        // Check if row_offset_y is out of the viewport
        // If row offset is not in the viewport, scroll to make it visible
        let edge_height = if direction.is_some() && self.mode.is_code_editor() {
            3 * line_height
        } else {
            line_height
        };
        if row_offset_y - edge_height + line_height < -scroll_offset.y {
            // Scroll up
            scroll_offset.y = -row_offset_y + edge_height - line_height;
        } else if row_offset_y + edge_height > -scroll_offset.y + bounds.size.height {
            // Scroll down
            scroll_offset.y = -(row_offset_y - bounds.size.height + edge_height);
        }

        // Avoid necessary scroll, when it was already in the correct position.
        if direction == Some(MoveDirection::Up) {
            scroll_offset.y = scroll_offset.y.max(was_offset.y);
        } else if direction == Some(MoveDirection::Down) {
            scroll_offset.y = scroll_offset.y.min(was_offset.y);
        }

        scroll_offset.x = scroll_offset.x.min(px(0.));
        scroll_offset.y = scroll_offset.y.min(px(0.));
        self.deferred_scroll_offset = Some(scroll_offset);
        cx.notify();
    }

    pub(super) fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    pub(super) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            return;
        }

        let selected_text = self.text.slice(self.selected_range).to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(selected_text));
    }

    pub(super) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            return;
        }

        let selected_text = self.text.slice(self.selected_range).to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(selected_text));

        self.replace_text_in_range_silent(None, "", window, cx);
    }

    pub(super) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(clipboard) = cx.read_from_clipboard() {
            let mut new_text = clipboard.text().unwrap_or_default();
            if !self.mode.is_multi_line() {
                new_text = new_text.replace('\n', "");
            }

            self.replace_text_in_range_silent(None, &new_text, window, cx);
            self.scroll_to(self.cursor(), None, cx);
        }
    }

    fn push_history(&mut self, text: &Rope, range: &Range<usize>, new_text: &str) {
        if self.history.ignore {
            return;
        }

        let range =
            text.clip_offset(range.start, Bias::Left)..text.clip_offset(range.end, Bias::Right);
        let old_text = text.slice(range.clone()).to_string();
        let new_range = range.start..range.start + new_text.len();

        self.history
            .push(Change::new(range, &old_text, new_range, new_text));
    }

    pub(super) fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        self.history.ignore = true;
        if let Some(changes) = self.history.undo() {
            for change in changes {
                let range_utf16 = self.range_to_utf16(&change.new_range.into());
                self.replace_text_in_range_silent(Some(range_utf16), &change.old_text, window, cx);
            }
        }
        self.history.ignore = false;
    }

    pub(super) fn redo(&mut self, _: &Redo, window: &mut Window, cx: &mut Context<Self>) {
        self.history.ignore = true;
        if let Some(changes) = self.history.redo() {
            for change in changes {
                let range_utf16 = self.range_to_utf16(&change.old_range.into());
                self.replace_text_in_range_silent(Some(range_utf16), &change.new_text, window, cx);
            }
        }
        self.history.ignore = false;
    }

    /// Get byte offset of the cursor.
    ///
    /// The offset is the UTF-8 offset.
    pub fn cursor(&self) -> usize {
        if let Some(ime_marked_range) = &self.ime_marked_range {
            return ime_marked_range.end;
        }

        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    /// Whether an IME composition currently owns marked text.
    ///
    /// History navigation and similar editor-level shortcuts should not
    /// intercept navigation keys while the platform input method is composing.
    pub fn has_ime_marked_text(&self) -> bool {
        self.ime_marked_range.is_some()
    }

    /// Visible row range in the last laid-out viewport, `None` before first layout.
    pub fn visible_row_range(&self) -> Option<std::ops::Range<usize>> {
        self.last_layout.as_ref().map(|l| l.visible_range.clone())
    }

    /// Current scroll offset of the editor viewport.
    pub fn scroll_offset(&self) -> gpui::Point<gpui::Pixels> {
        self.scroll_handle.offset()
    }

    /// Scrolls a logical line near the top of the editor viewport.
    pub fn scroll_to_line(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(last_layout) = self.last_layout.as_ref() else {
            return;
        };
        let row = row.min(self.text.lines_len().saturating_sub(1));
        let line_height = last_layout.line_height;
        let visible_rows = (0..row)
            .map(|line| {
                self.display_map
                    .visible_wrap_row_count_for_buffer_line(line)
            })
            .sum::<usize>();
        let row_offset_y = line_height * visible_rows as f32;
        let current = self.scroll_handle.offset();
        let top_margin = line_height * 2.0;
        self.update_scroll_offset(
            Some(point(current.x, (-row_offset_y + top_margin).min(px(0.0)))),
            cx,
        );
    }

    /// Laid-out line height; `None` before first layout.
    pub fn line_height(&self) -> Option<gpui::Pixels> {
        self.last_layout.as_ref().map(|l| l.line_height)
    }

    /// Returns the current selection as a byte range into the text.
    ///
    /// The range is empty (`start == end`) when no text is selected; in
    /// that case the offset equals `cursor()`. Byte offsets are measured
    /// in the underlying rope's byte units.
    pub fn selected_range(&self) -> std::ops::Range<usize> {
        self.selected_range.into()
    }

    /// Return the closest UTF-8 byte offset for an absolute window position.
    ///
    /// Returns `None` before the input has completed its first layout.
    pub fn offset_for_position(&self, position: Point<Pixels>) -> Option<usize> {
        self.last_bounds.as_ref()?;
        self.last_layout.as_ref()?;
        Some(self.index_for_mouse_position(position))
    }

    pub(crate) fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        // If the text is empty, always return 0
        if self.text.len() == 0 {
            return 0;
        }

        let (Some(bounds), Some(last_layout)) =
            (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };

        let line_height = last_layout.line_height;
        let line_number_width = last_layout.line_number_width;

        // TIP: About the IBeam cursor
        //
        // If cursor style is IBeam, the mouse mouse position is in the middle of the cursor (This is special in OS)

        // The position is relative to the bounds of the text input
        //
        // bounds.origin:
        //
        // - included the input padding.
        // - included the scroll offset.
        let inner_position = position - bounds.origin - point(line_number_width, px(0.));

        let mut y_offset = last_layout.visible_top;

        // Traverse visible buffer lines (compact, no hidden entries)
        for (vi, (line_layout, _buffer_line)) in last_layout
            .lines
            .iter()
            .zip(last_layout.visible_buffer_lines.iter())
            .enumerate()
        {
            let line_start_offset = last_layout.visible_line_byte_offsets[vi];

            // Calculate line origin for this display row
            let line_origin = point(px(0.), y_offset);
            let pos = inner_position - line_origin;

            // Return offset by use closest_index_for_x if is single line mode.
            if self.mode.is_single_line() {
                let local_index = line_layout.closest_index_for_x(pos.x, last_layout);
                let index = line_start_offset + local_index;
                return if self.masked {
                    self.text.char_index_to_offset(index / MASK_CHAR.len_utf8())
                } else {
                    index.min(self.text.len())
                };
            }

            // Check if mouse is in this line's bounds
            if let Some(local_index) = line_layout.closest_index_for_position(pos, last_layout) {
                let index = line_start_offset + local_index;
                return if self.masked {
                    self.text.char_index_to_offset(index / MASK_CHAR.len_utf8())
                } else {
                    index.min(self.text.len())
                };
            } else if pos.y < px(0.) {
                // Mouse is above this line, return start of this line
                return if self.masked {
                    self.text
                        .char_index_to_offset(line_start_offset / MASK_CHAR.len_utf8())
                } else {
                    line_start_offset
                };
            }

            y_offset += line_layout.size(line_height).height;
        }

        // Mouse is below all visible lines, return end of text
        self.text.len()
    }

    /// Returns a y offsetted point for the line origin.
    /// Select the text from the current cursor position to the given offset.
    ///
    /// The offset is the UTF-8 offset.
    ///
    /// Ensure the offset use self.next_boundary or self.previous_boundary to get the correct offset.
    pub(crate) fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.invalidate_completions(cx);

        let previous_cursor = self.cursor();
        let offset = offset.clamp(0, self.text.len());
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };

        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = (self.selected_range.end..self.selected_range.start).into();
        }

        // Ensure keep word selected range
        if let Some(word_range) = self.selected_word_range.as_ref() {
            if self.selected_range.start > word_range.start {
                self.selected_range.start = word_range.start;
            }
            if self.selected_range.end < word_range.end {
                self.selected_range.end = word_range.end;
            }
        }
        if self.selected_range.is_empty() {
            self.update_preferred_column();
        }
        let cursor = self.cursor();
        let direction = if cursor < previous_cursor {
            Some(MoveDirection::Up)
        } else if cursor > previous_cursor {
            Some(MoveDirection::Down)
        } else {
            None
        };
        self.scroll_to(cursor, direction, cx);
        cx.notify()
    }

    /// Unselects the currently selected text.
    pub fn unselect(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.cursor();
        self.selected_range = (offset..offset).into();
        cx.notify()
    }

    #[inline]
    pub(super) fn offset_from_utf16(&self, offset: usize) -> usize {
        self.text.offset_utf16_to_offset(offset)
    }

    #[inline]
    pub(super) fn offset_to_utf16(&self, offset: usize) -> usize {
        self.text.offset_to_offset_utf16(offset)
    }

    #[inline]
    pub(super) fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    #[inline]
    pub(super) fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn utf16_offset_to_byte_offset(text: &str, offset_utf16: usize, bias: Bias) -> usize {
        if offset_utf16 == 0 {
            return 0;
        }

        let mut current_utf16 = 0;
        for (byte_offset, ch) in text.char_indices() {
            let next_utf16 = current_utf16 + ch.len_utf16();
            if offset_utf16 < next_utf16 {
                return if bias == Bias::Left {
                    byte_offset
                } else {
                    byte_offset + ch.len_utf8()
                };
            }
            if offset_utf16 == next_utf16 {
                return byte_offset + ch.len_utf8();
            }
            current_utf16 = next_utf16;
        }

        text.len()
    }

    fn relative_utf16_range_to_byte_range(text: &str, range_utf16: &Range<usize>) -> Range<usize> {
        let start_utf16 = range_utf16.start.min(range_utf16.end);
        let end_utf16 = range_utf16.start.max(range_utf16.end);
        let start = Self::utf16_offset_to_byte_offset(text, start_utf16, Bias::Left);
        let end = Self::utf16_offset_to_byte_offset(text, end_utf16, Bias::Right);
        start..end
    }

    fn absolute_range_from_relative_utf16(
        base_start: usize,
        text: &str,
        range_utf16: &Range<usize>,
    ) -> Range<usize> {
        let relative = Self::relative_utf16_range_to_byte_range(text, range_utf16);
        base_start.saturating_add(relative.start)..base_start.saturating_add(relative.end)
    }

    fn clip_byte_range(&self, range: Range<usize>) -> Range<usize> {
        let start = range.start.min(range.end).min(self.text.len());
        let end = range.start.max(range.end).min(self.text.len());
        self.text.clip_offset(start, Bias::Left)..self.text.clip_offset(end, Bias::Right)
    }

    fn unmarked_replacement_range_from_utf16(
        &self,
        range_utf16: Option<&Range<usize>>,
    ) -> Range<usize> {
        range_utf16
            .map(|range_utf16| self.clip_byte_range(self.range_from_utf16(range_utf16)))
            .unwrap_or_else(|| self.clip_byte_range(self.selected_range.into()))
    }

    /// Resolve a `setMarkedText` replacement range to an absolute UTF-8 byte range.
    ///
    /// AppKit reports document-relative UTF-16 ranges when no composition is active, but once
    /// marked text exists, subsequent composition ranges are relative to that marked text.
    fn composition_replacement_range_from_utf16(
        &self,
        range_utf16: Option<&Range<usize>>,
    ) -> Range<usize> {
        if let Some(marked_range) = self.ime_marked_range {
            let marked_range = self.clip_byte_range(marked_range.into());
            return range_utf16
                .map(|range_utf16| {
                    let marked_text = self.text.slice(marked_range.clone()).to_string();
                    Self::absolute_range_from_relative_utf16(
                        marked_range.start,
                        &marked_text,
                        range_utf16,
                    )
                })
                .unwrap_or(marked_range);
        }

        self.unmarked_replacement_range_from_utf16(range_utf16)
    }

    /// Resolve an `insertText` replacement range to an absolute UTF-8 byte range.
    ///
    /// Committing or deleting an active composition replaces the complete marked range. This also
    /// accepts platforms that pass the marked document range back as an absolute UTF-16 range.
    fn committed_replacement_range_from_utf16(
        &self,
        range_utf16: Option<&Range<usize>>,
    ) -> Range<usize> {
        self.ime_marked_range
            .map(|range| self.clip_byte_range(range.into()))
            .unwrap_or_else(|| self.unmarked_replacement_range_from_utf16(range_utf16))
    }

    /// If offset falls on a hidden (folded) line, clamp backward to the end of
    /// the fold header line (last visible position before the fold).
    fn clamp_offset_to_visible_backward(&self, offset: usize) -> usize {
        let line = self.text.offset_to_point(offset).row;
        if self.display_map.is_buffer_line_hidden(line) {
            for fold in self.display_map.folded_ranges() {
                if line > fold.start_line && line <= fold.end_line {
                    return self.text.line_end_offset(fold.start_line);
                }
            }
        }
        offset
    }

    /// If offset falls on a hidden (folded) line, clamp forward to the start of
    /// the fold end line (first visible position after the fold).
    fn clamp_offset_to_visible_forward(&self, offset: usize) -> usize {
        let line = self.text.offset_to_point(offset).row;
        if self.display_map.is_buffer_line_hidden(line) {
            for fold in self.display_map.folded_ranges() {
                if line > fold.start_line && line <= fold.end_line {
                    return self.text.line_start_offset(fold.end_line);
                }
            }
        }
        offset
    }

    pub(super) fn previous_boundary(&self, offset: usize) -> usize {
        let mut offset = self.text.clip_offset(offset.saturating_sub(1), Bias::Left);
        if let Some(ch) = self.text.char_at(offset) {
            if ch == '\r' {
                offset -= 1;
            }
        }

        self.clamp_offset_to_visible_backward(offset)
    }

    pub(super) fn next_boundary(&self, offset: usize) -> usize {
        let mut offset = self.text.clip_offset(offset + 1, Bias::Right);
        if let Some(ch) = self.text.char_at(offset) {
            if ch == '\r' {
                offset += 1;
            }
        }

        self.clamp_offset_to_visible_forward(offset)
    }

    /// Returns the true to let InputElement to render cursor, when Input is focused and current BlinkCursor is visible.
    pub(crate) fn show_cursor(&self, window: &Window, cx: &App) -> bool {
        (self.focus_handle.is_focused(window) || self.is_context_menu_open(cx))
            && !self.disabled
            && !self.read_only
            && self.blink_cursor.read(cx).visible()
            && window.is_window_active()
    }

    fn on_focus(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.blink_cursor.update(cx, |cursor, cx| {
            cursor.start(cx);
        });
        cx.emit(InputEvent::Focus);
    }

    fn on_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_context_menu_open(cx) {
            return;
        }

        // NOTE: Do not cancel select, when blur.
        // Because maybe user want to copy the selected text by AppMenuBar (will take focus handle).

        self.hover_definition.clear();
        // Keep `hover_popover`: blur here is usually the popover's own
        // selectable text taking focus for click-to-copy. The popover's
        // mouse-move-out / mouse-down-out handlers hide it afterwards.
        self.lsp.invalidate_hover();
        self.diagnostic_popover = None;
        self.invalidate_completions(cx);
        self.blink_cursor.update(cx, |cursor, cx| {
            cursor.stop(cx);
        });
        Root::update(window, cx, |root, _, _| {
            root.focused_input = None;
        });
        cx.emit(InputEvent::Blur);
        cx.notify();
    }

    pub(super) fn pause_blink_cursor(&mut self, cx: &mut Context<Self>) {
        self.blink_cursor.update(cx, |cursor, cx| {
            cursor.pause(cx);
        });
    }

    pub(super) fn on_key_down(&mut self, _: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.pause_blink_cursor(cx);
    }

    pub(super) fn on_drag_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.text.len() == 0 {
            return;
        }

        if self.last_layout.is_none() {
            return;
        }

        if !self.focus_handle.is_focused(window) {
            return;
        }

        if !self.selecting {
            return;
        }

        let offset = self.index_for_mouse_position(event.position);
        self.select_to(offset, cx);
    }

    fn is_valid_input(&self, new_text: &str, cx: &mut Context<Self>) -> bool {
        if new_text.is_empty() {
            return true;
        }

        if let Some(validate) = &self.validate {
            if !validate(new_text, cx) {
                return false;
            }
        }

        if !self.mask_pattern.is_valid(new_text) {
            return false;
        }

        let Some(pattern) = &self.pattern else {
            return true;
        };

        pattern.is_match(new_text)
    }

    /// Set the mask pattern for formatting the input text.
    ///
    /// The pattern can contain:
    /// - 9: Any digit or dot
    /// - A: Any letter
    /// - *: Any character
    /// - Other characters will be treated as literal mask characters
    ///
    /// Example: "(999)999-999" for phone numbers
    pub fn mask_pattern(mut self, pattern: impl Into<MaskPattern>) -> Self {
        self.mask_pattern = pattern.into();
        if let Some(placeholder) = self.mask_pattern.placeholder() {
            self.placeholder = placeholder.into();
        }
        self
    }

    pub fn set_mask_pattern(
        &mut self,
        pattern: impl Into<MaskPattern>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mask_pattern = pattern.into();
        if let Some(placeholder) = self.mask_pattern.placeholder() {
            self.placeholder = placeholder.into();
        }
        cx.notify();
    }

    pub(super) fn set_input_bounds(&mut self, new_bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        let wrap_width_changed = self.input_bounds.size.width != new_bounds.size.width;
        self.input_bounds = new_bounds;

        // Update display_map wrap_width if changed.
        if let Some(last_layout) = self.last_layout.as_ref() {
            if wrap_width_changed {
                let wrap_width = if !self.soft_wrap {
                    // None to disable wrapping (will use Pixels::MAX)
                    None
                } else {
                    last_layout.wrap_width
                };

                self.display_map.on_layout_changed(wrap_width, cx);
                self.mode.update_auto_grow(&self.display_map);
                cx.notify();
            }
        }
    }

    pub(super) fn selected_text(&self) -> RopeSlice<'_> {
        let range_utf16 = self.range_to_utf16(&self.selected_range.into());
        let range = self.range_from_utf16(&range_utf16);
        self.text.slice(range)
    }

    /// Return the rendered bounds for a UTF-8 byte range in the current input contents.
    ///
    /// Returns `None` when the requested range is not currently laid out or visible.
    pub fn range_to_bounds(&self, range: &Range<usize>) -> Option<Bounds<Pixels>> {
        let Some(last_layout) = self.last_layout.as_ref() else {
            return None;
        };

        let Some(last_bounds) = self.last_bounds else {
            return None;
        };

        let (_, _, start_pos) = self.line_and_position_for_offset(range.start);
        let (_, _, end_pos) = self.line_and_position_for_offset(range.end);

        let Some(start_pos) = start_pos else {
            return None;
        };
        let Some(end_pos) = end_pos else {
            return None;
        };

        Some(Bounds::from_corners(
            last_bounds.origin + start_pos,
            last_bounds.origin + end_pos + point(px(0.), last_layout.line_height),
        ))
    }

    /// Replace text in range in silent.
    ///
    /// This will not trigger any UI interaction, such as auto-completion.
    pub(crate) fn replace_text_in_range_silent(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.silent_replace_text = true;
        self.replace_text_in_range(range_utf16, new_text, window, cx);
        self.silent_replace_text = false;
    }

    /// Update fold candidates from tree-sitter syntax tree (full extraction).
    /// Used only on initial load or language changes.
    fn update_fold_candidates(&mut self) {
        if !self.mode.is_folding() {
            return;
        }

        let Some(highlighter_rc) = self.mode.highlighter() else {
            return;
        };

        let highlighter = highlighter_rc.borrow();
        let Some(highlighter) = highlighter.as_ref() else {
            return;
        };

        let Some(tree) = highlighter.tree() else {
            return;
        };

        let fold_ranges = crate::input::display_map::extract_fold_ranges(tree);
        self.display_map.set_fold_candidates(fold_ranges);
    }

    /// Incrementally update fold candidates after a text edit.
    /// Only traverses the edited region of the syntax tree instead of the full tree.
    fn update_fold_candidates_incremental(&mut self, edit_range: &Range<usize>, new_text: &str) {
        if !self.mode.is_folding() {
            return;
        }

        let Some(highlighter_rc) = self.mode.highlighter() else {
            return;
        };

        let highlighter = highlighter_rc.borrow();
        let Some(highlighter) = highlighter.as_ref() else {
            return;
        };

        let Some(tree) = highlighter.tree() else {
            return;
        };

        // The new byte range in the updated text after the edit
        let new_end = edit_range.start + new_text.len();
        self.display_map.update_fold_candidates_for_edit(
            tree,
            edit_range.start..new_end,
            &self.text,
        );
    }

    fn dispatch_highlighter_work(
        pending: super::mode::PendingHighlighterWork,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match pending {
            super::mode::PendingHighlighterWork::Initialize(pending) => {
                Self::dispatch_background_highlighter_initialization(pending, window, cx);
            }
            super::mode::PendingHighlighterWork::Parse(pending) => {
                Self::dispatch_background_parse(pending, window, cx);
            }
        }
    }

    /// Build and initially parse a syntax highlighter away from the UI thread.
    ///
    /// `SyntaxHighlighter::new` may lazily read and compile a WASM grammar and
    /// then construct several tree-sitter queries. Doing that from `render`
    /// blocks the whole application, especially when a markdown document
    /// mounts several code surfaces at once.
    #[cfg(not(target_family = "wasm"))]
    fn dispatch_background_highlighter_initialization(
        pending: super::mode::PendingHighlighterInitialization,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let highlighter_rc = pending.highlighter;
        let parse_task_rc = pending.parse_task;
        let language = pending.language;
        let text = pending.text;
        let is_folding = pending.is_folding;
        let initialized_highlighter = highlighter_rc.clone();

        let task = cx.spawn_in(window, async move |entity, cx| {
            let parsed_text = text.clone();
            let initialized_language = language.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let started = std::time::Instant::now();
                    let mut highlighter = crate::highlighter::SyntaxHighlighter::new(&language);
                    highlighter.update(None, &text, None);
                    let fold_ranges = if is_folding {
                        highlighter
                            .tree()
                            .map(crate::input::display_map::extract_fold_ranges)
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    tracing::debug!(
                        language = %language,
                        elapsed_ms = started.elapsed().as_millis(),
                        "initialized syntax highlighter in background"
                    );
                    (highlighter, fold_ranges)
                })
                .await;

            let (highlighter, fold_ranges) = result;
            _ = entity.update(cx, |state, cx| {
                let Some(current_highlighter) = state.mode.highlighter() else {
                    return;
                };
                if !Rc::ptr_eq(current_highlighter, &initialized_highlighter) {
                    return;
                }
                if state.text != parsed_text {
                    // The buffer changed while initialization was running.
                    // Leave this highlighter uninstalled so the next render
                    // schedules a fresh job for the current text.
                    state._pending_update = true;
                    cx.notify();
                    return;
                }

                *current_highlighter.borrow_mut() = Some(highlighter);
                if is_folding {
                    state.display_map.set_fold_candidates(fold_ranges);
                }
                tracing::debug!(
                    language = %initialized_language,
                    "installed background syntax highlighter"
                );
                cx.notify();
            });
        });

        // Replacing the task cancels an obsolete initialization when the
        // language or buffer changes before it finishes.
        parse_task_rc.borrow_mut().replace(task);
    }

    #[cfg(target_family = "wasm")]
    fn dispatch_background_highlighter_initialization(
        pending: super::mode::PendingHighlighterInitialization,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut highlighter = crate::highlighter::SyntaxHighlighter::new(&pending.language);
        highlighter.update(None, &pending.text, None);
        *pending.highlighter.borrow_mut() = Some(highlighter);
        cx.notify();
    }

    /// Spawn a background parse after the synchronous parse timed out.
    ///
    /// Dropping the returned `Task` (stored in `parse_task`) cancels the
    /// parse, which naturally debounces rapid edits.
    #[cfg(not(target_family = "wasm"))]
    fn dispatch_background_parse(
        pending: super::mode::PendingBackgroundParse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let highlighter_rc = pending.highlighter;
        let parse_task_rc = pending.parse_task;
        let language = pending.language;
        let text = pending.text;
        let is_folding = pending.is_folding;

        let old_tree = highlighter_rc
            .borrow()
            .as_ref()
            .and_then(|h| h.tree().cloned());

        // Extract injection parse data on the main thread before spawning, so that
        // compute_injection_layers can also run on the background thread.
        let injection_data = highlighter_rc
            .borrow()
            .as_ref()
            .and_then(|h| h.injection_parse_data());

        let text_for_apply = text.clone();
        let task = cx.spawn_in(window, async move |entity, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let Some(config) = LanguageRegistry::singleton().language(&language) else {
                        return None;
                    };

                    let mut parser = crate::highlighter::parser_for_config(&config).ok()?;

                    let new_tree = parser.parse_with_options(
                        &mut |offset, _| {
                            if offset >= text.len() {
                                ""
                            } else {
                                let (chunk, chunk_byte_ix) = text.chunk(offset);
                                &chunk[offset - chunk_byte_ix..]
                            }
                        },
                        old_tree.as_ref(),
                        None,
                    )?;

                    // Compute injection layers in the background to avoid blocking the
                    // main thread with combined-injection parsing (e.g. PHP, HTML+JS/CSS).
                    let injection_layers = if let Some(data) = injection_data {
                        crate::highlighter::SyntaxHighlighter::compute_injection_layers(
                            data, &new_tree, &text,
                        )
                    } else {
                        Default::default()
                    };

                    // Walk the syntax tree to extract fold ranges off the main thread.
                    let fold_ranges = if is_folding {
                        crate::input::display_map::extract_fold_ranges(&new_tree)
                    } else {
                        Vec::new()
                    };

                    Some((new_tree, injection_layers, fold_ranges))
                })
                .await;

            if let Some((new_tree, injection_layers, fold_ranges)) = result {
                if let Some(h) = highlighter_rc.borrow_mut().as_mut() {
                    h.apply_background_tree(new_tree, &text_for_apply, injection_layers);
                }

                // Trigger re-render so the new highlights are displayed and
                // apply the fold candidates extracted in the background.
                _ = entity.update(cx, |state, cx| {
                    if is_folding {
                        state.display_map.set_fold_candidates(fold_ranges);
                    }
                    cx.notify();
                });
            }
        });

        parse_task_rc.borrow_mut().replace(task);
    }

    #[cfg(target_family = "wasm")]
    fn dispatch_background_parse(
        _pending: super::mode::PendingBackgroundParse,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // No-op
    }

    /// 应用括号自动配对。
    /// 返回 (实际插入的文本, 光标需要往回移动的字符数, 是否跳过插入)。
    fn apply_auto_pair(new_text: &str, cursor: usize, text: &Rope) -> (String, usize, bool) {
        if new_text.len() != 1 {
            return (new_text.to_string(), 0, false);
        }

        let ch = new_text.chars().next().unwrap();
        let is_closing = matches!(ch, ')' | ']' | '}' | '"' | '\'');
        if is_closing
            && cursor < text.len()
            && let Some(next_char) = text.char_at(cursor)
            && next_char == ch
        {
            return (String::new(), 0, true);
        }

        let pair = match ch {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' => Some('"'),
            '\'' => Some('\''),
            _ => None,
        };

        match pair {
            Some(closing) => {
                let mut result = String::with_capacity(2);
                result.push(ch);
                result.push(closing);
                (result, 1, false)
            }
            None => (new_text.to_string(), 0, false),
        }
    }
}

impl EntityInputHandler for InputState {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        adjusted_range.replace(self.range_to_utf16(&range));
        Some(self.text.slice(range).to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range.into()),
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.ime_marked_range
            .map(|range| self.range_to_utf16(&range.into()))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.ime_marked_range = None;
    }

    /// Replace text in range.
    ///
    /// - If the new text is invalid, it will not be replaced.
    /// - If `range_utf16` is not provided, the current selected range will be used.
    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled || self.read_only {
            return;
        }

        if self.blink_cursor.read(cx).visible() {
            self.pause_blink_cursor(cx);
        }

        let range = self.committed_replacement_range_from_utf16(range_utf16.as_ref());

        let (actual_text, cursor_offset_back, skip_insert) = if self.auto_pair {
            InputState::apply_auto_pair(new_text, range.start, &self.text)
        } else {
            (new_text.to_string(), 0, false)
        };

        if skip_insert {
            self.selected_range = ((range.start + 1)..(range.start + 1)).into();
            self.update_preferred_column();
            cx.notify();
            return;
        }

        let old_text = self.text.clone();
        self.text.replace(range.clone(), &actual_text);
        let content_changed = self.text != old_text;
        if content_changed {
            self.document_revision = self.document_revision.saturating_add(1);
            // Gutter markers are owner-managed: keep the stale markers visible
            // (ids encode the document revision so stale clicks fail closed)
            // until the owner re-sets them, instead of blinking on every edit.
            self.range_decorations = Rc::from([]);
            self.inline_widgets = Rc::from([]);
            self.invalidate_completions(cx);
            self.hover_definition.clear();
            self.hover_popover = None;
            self.signature_help_popover = None;
            self.last_signature_help_request = None;
            self.lsp.invalidate_hover();
            self.lsp.invalidate_signature_help();
        }

        let mut new_offset = (range.start + actual_text.len())
            .saturating_sub(cursor_offset_back)
            .min(self.text.len());

        if self.mode.is_single_line() {
            let pending_text = self.text.to_string();
            // Check if the new text is valid
            if !self.is_valid_input(&pending_text, cx) {
                self.text = old_text;
                return;
            }

            if !self.mask_pattern.is_none() {
                let mask_text = self.mask_pattern.mask(&pending_text);
                self.text = Rope::from(mask_text.as_str());
                let new_text_len =
                    (actual_text.len() + mask_text.len()).saturating_sub(pending_text.len());
                new_offset = (range.start + new_text_len).min(mask_text.len());
            }
        }

        self.push_history(&old_text, &range, &actual_text);
        self.history.end_grouping();
        if let Some(diagnostics) = self.mode.diagnostics_mut() {
            diagnostics.reset(&self.text)
        }
        // Adjust folds before updating wrap map: remove overlapping folds and shift others
        self.display_map
            .adjust_folds_for_edit(&old_text, &range, &actual_text);
        self.display_map
            .on_text_changed(&self.text, &range, &Rope::from(actual_text.as_str()), cx);

        let bg =
            self.mode
                .update_highlighter(&range, &old_text, &self.text, &actual_text, true, cx);
        if let Some(bg) = bg {
            Self::dispatch_highlighter_work(bg, window, cx);
        }

        self.update_fold_candidates_incremental(&range, &actual_text);
        self.lsp.update(&self.text, window, cx);
        self.selected_range = (new_offset..new_offset).into();
        self.ime_marked_range.take();
        self.update_preferred_column();
        self.update_search(cx);
        self.mode.update_auto_grow(&self.display_map);
        if !self.silent_replace_text {
            self.handle_completion_trigger(&range, &actual_text, window, cx);
            self.handle_signature_help_edit(&actual_text, window, cx);
        }
        if self.emit_events {
            cx.emit(InputEvent::Change);
        }
        cx.notify();
    }

    /// Mark text is the IME temporary insert on typing.
    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled || self.read_only {
            return;
        }

        self.lsp.reset();
        self.hover_definition.clear();
        self.hover_popover = None;
        self.signature_help_popover = None;
        self.last_signature_help_request = None;
        self.invalidate_completions(cx);

        let range = self.composition_replacement_range_from_utf16(range_utf16.as_ref());

        let old_text = self.text.clone();
        self.text.replace(range.clone(), new_text);
        if self.text != old_text {
            self.document_revision = self.document_revision.saturating_add(1);
            self.range_decorations = Rc::from([]);
            self.inline_widgets = Rc::from([]);
        }

        if self.mode.is_single_line() {
            let pending_text = self.text.to_string();
            if !self.is_valid_input(&pending_text, cx) {
                self.text = old_text;
                return;
            }
        }

        if let Some(diagnostics) = self.mode.diagnostics_mut() {
            diagnostics.reset(&self.text)
        }
        // Adjust folds before updating wrap map: remove overlapping folds and shift others
        self.display_map
            .adjust_folds_for_edit(&old_text, &range, new_text);
        self.display_map
            .on_text_changed(&self.text, &range, &Rope::from(new_text), cx);

        let bg = self
            .mode
            .update_highlighter(&range, &old_text, &self.text, &new_text, true, cx);
        if let Some(bg) = bg {
            Self::dispatch_highlighter_work(bg, window, cx);
        }

        self.update_fold_candidates_incremental(&range, new_text);
        self.lsp.update(&self.text, window, cx);
        if new_text.is_empty() {
            // Cancel selection, when cancel IME input.
            self.selected_range = (range.start..range.start).into();
            self.ime_marked_range = None;
        } else {
            let marked_range = range.start..range.start + new_text.len();
            self.ime_marked_range = Some(marked_range.clone().into());
            let selected_range = new_selected_range_utf16
                .as_ref()
                .map(|range_utf16| {
                    Self::absolute_range_from_relative_utf16(range.start, new_text, range_utf16)
                })
                .unwrap_or_else(|| marked_range.end..marked_range.end);
            self.selected_range = self.clip_byte_range(selected_range).into();
        }
        self.mode.update_auto_grow(&self.display_map);
        self.history.start_grouping();
        self.push_history(&old_text, &range, new_text);
        cx.notify();
    }

    /// Used to position IME candidates.
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let line_height = last_layout.line_height;
        let line_number_width = last_layout.line_number_width;
        let range = self.range_from_utf16(&range_utf16);

        let mut start_origin = None;
        let mut end_origin = None;
        let line_number_origin = point(line_number_width, px(0.));
        let mut y_offset = last_layout.visible_top;

        for (vi, line) in last_layout.lines.iter().enumerate() {
            if start_origin.is_some() && end_origin.is_some() {
                break;
            }

            let index_offset = last_layout.visible_line_byte_offsets[vi];

            if start_origin.is_none() {
                if let Some(p) = line.position_for_index(
                    range.start.saturating_sub(index_offset),
                    last_layout,
                    false,
                ) {
                    start_origin = Some(p + point(px(0.), y_offset));
                }
            }

            if end_origin.is_none() {
                if let Some(p) = line.position_for_index(
                    range.end.saturating_sub(index_offset),
                    last_layout,
                    false,
                ) {
                    end_origin = Some(p + point(px(0.), y_offset));
                }
            }

            y_offset += line.size(line_height).height;
        }

        let start_origin = start_origin.unwrap_or_default();
        let mut end_origin = end_origin.unwrap_or_default();
        // Ensure at same line.
        end_origin.y = start_origin.y;

        Some(Bounds::from_corners(
            bounds.origin + line_number_origin + start_origin,
            // + line_height for show IME panel under the cursor line.
            bounds.origin + line_number_origin + point(end_origin.x, end_origin.y + line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let last_layout = self.last_layout.as_ref()?;
        let line_point = self.last_bounds?.localize(&point)?;

        for (vi, line) in last_layout.lines.iter().enumerate() {
            let offset = last_layout.visible_line_byte_offsets[vi];
            if let Some(utf8_index) = line.index_for_position(line_point, last_layout) {
                return Some(self.offset_to_utf16(offset + utf8_index));
            }
        }

        None
    }
}

impl Focusable for InputState {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for InputState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self._pending_update {
            let bg = self
                .mode
                .update_highlighter(&(0..0), &self.text, &self.text, "", false, cx);
            if let Some(bg) = bg {
                Self::dispatch_highlighter_work(bg, window, cx);
            }

            self.update_fold_candidates();
            self.lsp.update(&self.text, window, cx);
            self._pending_update = false;
        }

        div()
            .id("input-state")
            .flex_1()
            .when(self.mode.is_multi_line(), |this| this.h_full())
            .flex_grow_1()
            .overflow_x_hidden()
            .child(TextElement::new(cx.entity().clone()).placeholder(self.placeholder.clone()))
            .children(self.diagnostic_popover.clone())
            .children(self.context_menu_content.as_ref().map(|menu| menu.render()))
            .children(self.hover_popover.clone())
            .children(self.signature_help_popover.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use gpui::{TestAppContext, VisualTestContext, canvas};
    use std::cell::RefCell;

    /// Helper to create an InputState in a window for testing
    fn new_code_editor(cx: &mut TestAppContext) -> (Entity<InputState>, &mut VisualTestContext) {
        cx.update(|cx| {
            // Set up the theme first
            cx.set_global(Theme::default());
            // Initialize input keybindings
            super::super::init(cx);
        });
        cx.add_window_view(|window, cx| InputState::new(window, cx).code_editor("sql"))
    }

    fn new_detached_code_editor(
        cx: &mut TestAppContext,
    ) -> (Entity<InputState>, &mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(Theme::default());
            super::super::init(cx);
        });
        let cx = cx.add_empty_window();
        let input =
            cx.update(|window, cx| cx.new(|cx| InputState::new(window, cx).code_editor("sql")));
        (input, cx)
    }

    fn draw_input(cx: &mut VisualTestContext, input: &Entity<InputState>) {
        cx.draw(
            point(px(0.), px(0.)),
            gpui::size(px(800.), px(240.)),
            |_, _| input.clone().into_any_element(),
        );
    }

    fn assert_valid_utf8_range(text: &Rope, range: Range<usize>) {
        assert!(range.start <= range.end, "range is reversed: {range:?}");
        assert!(
            range.end <= text.len(),
            "range {range:?} exceeds text length {}",
            text.len()
        );
        assert!(
            text.is_char_boundary(range.start),
            "range start {} is not a UTF-8 boundary in {:?}",
            range.start,
            text.to_string()
        );
        assert!(
            text.is_char_boundary(range.end),
            "range end {} is not a UTF-8 boundary in {:?}",
            range.end,
            text.to_string()
        );
    }

    /// 编辑不清空 gutter markers：它们由 owner 管理，保持可见直到 owner 重设，
    /// 否则 SQL 编辑器的 ▶ 运行按钮会在每次输入时闪烁。
    #[gpui::test]
    fn gutter_markers_persist_across_document_edits_until_reset(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("select 1;\nselect 2;", window, cx);
                let initial_revision = state.document_revision();
                state.set_gutter_markers(
                    vec![
                        InputGutterMarker::new("sql-statement:0:0:9", 0, IconName::Play)
                            .tooltip("Run statement"),
                    ],
                    cx,
                );

                assert_eq!(initial_revision, state.document_revision());
                assert_eq!(0, state.gutter_markers()[0].logical_row);
                assert_eq!(state.gutter_markers()[0].id, "sql-statement:0:0:9");

                state.replace_text_range("select 1;".len().."select 1;".len(), "\n", window, cx);
                let next_revision = state.document_revision();
                assert!(next_revision > initial_revision);
                // Markers stay visible (stale) until the owner re-sets them.
                assert_eq!(1, state.gutter_markers().len());
                assert_eq!(state.gutter_markers()[0].id, "sql-statement:0:0:9");

                state.set_gutter_markers(
                    vec![InputGutterMarker::new("sql-statement:1:0:10", 1, IconName::Play)],
                    cx,
                );
                assert_eq!(state.gutter_markers()[0].id, "sql-statement:1:0:10");
            });
        });
    }

    /// 编辑不清空 gutter markers，marker 车道宽度也必须保持不变，
    /// 否则 SQL 编辑器每敲一个键 gutter 忽宽忽窄，文本左右抖动。
    #[gpui::test]
    fn gutter_marker_lane_width_stays_stable_across_edits(
        cx: &mut TestAppContext,
    ) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;
        let width_of = |input: &Entity<InputState>, cx: &mut VisualTestContext| {
            draw_input(cx, input);
            cx.update(|_, cx| {
                input
                    .read(cx)
                    .last_layout
                    .as_ref()
                    .expect("layout after draw")
                    .line_number_width
            })
        };

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("select 1;\nselect 2;", window, cx);
            });
        });
        let base_width = width_of(&input, cx);

        cx.update(|_window, cx| {
            input.update(cx, |state, cx| {
                state.set_gutter_markers(
                    vec![InputGutterMarker::new("sql-statement:0:0:9", 0, IconName::Play)],
                    cx,
                );
            });
        });
        let width_with_markers = width_of(&input, cx);
        assert!(width_with_markers > base_width);

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.replace_text_range("select".len().."select".len(), " ", window, cx);
                assert!(!state.gutter_markers().is_empty());
            });
        });
        let width_after_edit = width_of(&input, cx);

        assert_eq!(width_with_markers, width_after_edit);
    }

    #[gpui::test]
    fn same_value_set_synchronizes_view_and_keeps_gutter_markers(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("select 1;", window, cx);
                let revision = state.document_revision();
                state.set_selected_range(0.."select".len(), false, window, cx);
                state.set_gutter_markers(
                    vec![InputGutterMarker::new(
                        "sql-statement:0:0:9",
                        0,
                        IconName::Play,
                    )],
                    cx,
                );
                state.set_value("select 1;", window, cx);

                assert_eq!(revision, state.document_revision());
                assert_eq!(1, state.gutter_markers().len());
                assert_eq!(Selection::default(), state.selected_range);
            });
        });
    }

    /// 构造根节点为 `Root` 的代码编辑器窗口：`on_blur` 等路径需要访问 Root。
    fn new_rooted_code_editor(
        cx: &mut TestAppContext,
    ) -> (Entity<InputState>, &mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(Theme::default());
            super::super::init(cx);
        });
        let slot: Rc<RefCell<Option<Entity<InputState>>>> = Rc::new(RefCell::new(None));
        let slot_clone = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).code_editor("sql"));
            *slot_clone.borrow_mut() = Some(input.clone());
            Root::new(input, window, cx)
        });
        let input = slot.borrow().clone().expect("input entity captured");
        (input, cx)
    }

    /// 失焦不清 hover popover：popover 里的可选中文本被点击后会抢走焦点，
    /// 若失焦即清理，用户永远无法选中复制 hover 内容。
    #[gpui::test]
    fn blur_keeps_hover_popover_for_click_to_copy(cx: &mut TestAppContext) {
        let (input, cx) = new_rooted_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.focus_handle.focus(window, cx);
                let hover = lsp_types::Hover {
                    contents: lsp_types::HoverContents::Markup(lsp_types::MarkupContent {
                        kind: lsp_types::MarkupKind::Markdown,
                        value: "Table: users".to_string(),
                    }),
                    range: None,
                };
                state.hover_popover = Some(HoverPopover::new(
                    cx.entity(),
                    0.."select".len(),
                    &hover,
                    cx,
                ));

                // 模拟点击 popover 文本导致的失焦路径。
                state.on_blur(window, cx);

                assert!(state.hover_popover.is_some());
            });
        });
    }

    #[test]
    fn gutter_marker_state_builder_and_defaults() {
        let marker = InputGutterMarker::new("sql-statement:1:0:9", 0, IconName::Play);
        assert_eq!(marker.state, InputGutterMarkerState::Idle);

        let running = marker.state(InputGutterMarkerState::Running);
        assert_eq!(running.state, InputGutterMarkerState::Running);
        assert_eq!(running.id, "sql-statement:1:0:9");
        assert_eq!(running.logical_row, 0);
        assert_eq!(running.icon, IconName::Play);
    }

    #[test]
    fn range_decoration_builder_and_defaults() {
        let decoration = InputRangeDecoration::new("sql-frame:1:0:9", 0..9);
        assert_eq!(decoration.kind, InputRangeDecorationKind::CurrentStatement);
        assert_eq!(decoration.style, InputRangeDecorationStyle::Frame);
        assert_eq!(decoration.range, 0..9);

        let styled = decoration.style(InputRangeDecorationStyle::None);
        assert_eq!(styled.style, InputRangeDecorationStyle::None);
        assert_eq!(styled.id, "sql-frame:1:0:9");
    }

    #[gpui::test]
    fn change_subscriber_writing_back_to_input_state_does_not_double_acquire(
        cx: &mut TestAppContext,
    ) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            struct Subscriber(Entity<InputState>, Rc<std::cell::Cell<usize>>);
            let calls = Rc::new(std::cell::Cell::new(0usize));
            let calls_check = calls.clone();
            let subscriber = cx.new(|cx| Subscriber(input.clone(), calls));
            subscriber.update(cx, |sub, cx| {
                let this = sub.0.clone();
                let calls = sub.1.clone();
                let emitter = this.clone();
                cx.subscribe(&emitter, move |_, _input, _event: &InputEvent, cx| {
                    calls.set(calls.get() + 1);
                    this.update(cx, |state, cx| {
                        state.set_gutter_markers(
                            vec![InputGutterMarker::new("m", 0, IconName::Play)],
                            cx,
                        );
                    });
                });
            });

            input.update(cx, |state, cx| {
                state.replace_text_in_range(None, "select 1;", window, cx);
            });
            assert!(calls_check.get() >= 1);
        });
    }

    #[gpui::test]
    fn range_decorations_are_cleared_on_document_edit(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("select 1;\nselect 2;", window, cx);
                let revision = state.document_revision();
                state.set_range_decorations(
                    vec![InputRangeDecoration::new(
                        "sql-frame:0:0:9",
                        0.."select 1;".len(),
                    )],
                    cx,
                );

                assert_eq!(revision, state.document_revision());
                assert_eq!(1, state.range_decorations().len());

                state.replace_text_range("select 1;".len().."select 1;".len(), "\n", window, cx);
                assert!(state.document_revision() > revision);
                assert!(state.range_decorations().is_empty());
            });
        });
    }

    #[gpui::test]
    fn highlight_decoration_style_is_preserved(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("INSERT INTO t (a) VALUES (1);", window, cx);
                state.set_range_decorations(
                    vec![
                        InputRangeDecoration::new("insert-values:0:0:28", 21..27)
                            .style(InputRangeDecorationStyle::Highlight),
                    ],
                    cx,
                );
                assert_eq!(
                    InputRangeDecorationStyle::Highlight,
                    state.range_decorations()[0].style
                );
            });
        });
    }

    #[gpui::test]
    fn inline_widgets_are_cleared_on_document_edit(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("INSERT INTO t (a) VALUES ('x');", window, cx);
                let revision = state.document_revision();
                state.set_inline_widgets(
                    vec![
                        InputInlineWidget::new("insert-hint:0:0:26", 26, "a"),
                        InputInlineWidget::new("insert-hint:0:0:27", 27, "b"),
                    ],
                    cx,
                );
                assert_eq!(revision, state.document_revision());
                assert_eq!(2, state.inline_widgets().len());

                state.replace_text_range(25..26, "y", window, cx);
                assert!(state.document_revision() > revision);
                assert!(state.inline_widgets().is_empty());
            });
        });
    }

    #[gpui::test]
    fn inline_widget_builder_round_trips_values(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("INSERT INTO t (a) VALUES (1);", window, cx);
                let widget = InputInlineWidget::new("insert-hint:0", 21, "a");
                state.set_inline_widgets(vec![widget], cx);
                assert_eq!("insert-hint:0", state.inline_widgets()[0].id.to_string());
                assert_eq!(21, state.inline_widgets()[0].offset);
                assert_eq!("a", state.inline_widgets()[0].text.to_string());
            });
        });
    }

    #[gpui::test]
    fn same_value_set_clears_range_decorations(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("select 1;", window, cx);
                state.set_range_decorations(
                    vec![InputRangeDecoration::new("sql-frame:0:0:9", 0..9)],
                    cx,
                );
                state.set_value("select 1;", window, cx);

                assert!(state.range_decorations().is_empty());
            });
        });
    }

    #[gpui::test]
    fn completion_invalidation_survives_document_and_context_changes(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("select 1", window, cx);
                state.focus(window, cx);

                let initial_epoch = state.completion_epoch();
                state.invalidate_completions(cx);
                assert_eq!(initial_epoch + 1, state.completion_epoch());
                assert!(!state.is_context_menu_open(cx));
                assert!(!state.has_inline_completion());

                let initial_revision = state.document_revision();
                state.replace_text_range("select 1".len().."select 1".len(), "0", window, cx);
                assert!(state.document_revision() > initial_revision);
                assert!(state.completion_epoch() > initial_epoch + 1);

                state.set_inline_completion_text(Some("0".to_string()), cx);
                state.replace_and_mark_text_in_range(
                    Some("select 10".len().."select 10".len()),
                    "0",
                    None,
                    window,
                    cx,
                );
                assert!(state.has_ime_marked_text());
                assert!(!state.has_inline_completion());
                assert!(state.completion_epoch() > initial_epoch + 2);
            });
        });
    }

    /// Regression test for the SQL editor crash:
    ///
    /// `CompletionMenu::hide` used to call `editor.update(cx, ...)` on the
    /// bound `InputState`. When `hide` was invoked from inside an update of that
    /// same `InputState` (the popup hide path in `handle_completion_trigger` and
    /// the async empty-completions path both do this), the editor entity was
    /// already leased, so GPUI panicked with
    /// `cannot upate ... while it is already being upated`.
    ///
    /// `hide` only needs to close the popup and reset the trigger offset; the
    /// in-flight stale guards (`completion_epoch`, request id, document revision,
    /// cursor, `trigger_start_offset` via `is_active_completion_menu`) already
    /// discard any response that lands after the menu is hidden.
    #[gpui::test]
    fn hiding_completion_menu_within_input_update_does_not_panic(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("select 1;", window, cx);

                // Build a completion menu bound to this very InputState.
                let menu = crate::input::popovers::CompletionMenu::new(cx.entity(), window, cx);

                // Replays the popup-hide crash path: the InputState update is
                // already in flight while we hide the menu.
                menu.update(cx, |menu, cx| {
                    menu.hide(cx);
                });

                assert!(!menu.read(cx).is_open());
                assert_eq!(menu.read(cx).trigger_start_offset, None);

                // The input must still be usable afterwards.
                state.set_value("select 2;", window, cx);
                assert!(state.text.to_string().contains("select 2"));
            });
        });
    }

    #[test]
    fn ime_relative_utf16_range_clips_to_utf8_boundaries() {
        assert_eq!(
            InputState::relative_utf16_range_to_byte_range("数据解读", &(1..3)),
            "数".len().."数据解".len()
        );
        assert_eq!(
            InputState::relative_utf16_range_to_byte_range("🎉", &(1..1)),
            0.."🎉".len()
        );
        assert_eq!(
            InputState::relative_utf16_range_to_byte_range("你", &(0..usize::MAX)),
            0.."你".len()
        );
    }

    #[gpui::test]
    fn ime_selection_is_relative_to_inserted_text_after_cjk_prefix(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("前后", window, cx);
                state.set_selected_range("前".len().."前".len(), false, window, cx);

                state.replace_and_mark_text_in_range(None, "n", Some(0..1), window, cx);

                assert_eq!(state.text.to_string(), "前n后");
                assert_eq!(
                    state.ime_marked_range,
                    Some(("前".len().."前n".len()).into())
                );
                assert!(state.has_ime_marked_text());
                assert_eq!(state.selected_range(), "前".len().."前n".len());
                assert_eq!(state.selected_text_string(), "n");
                assert_valid_utf8_range(&state.text, state.selected_range());
            });
        });
    }

    #[gpui::test]
    fn ime_caret_stays_on_utf8_boundary_after_cjk_prefix(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("前后", window, cx);
                state.set_selected_range("前".len().."前".len(), false, window, cx);

                state.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);

                assert_eq!(state.text.to_string(), "前n后");
                assert_eq!(state.selected_range(), "前n".len().."前n".len());
                assert_valid_utf8_range(&state.text, state.selected_range());
            });
        });
    }

    #[gpui::test]
    fn ime_updates_and_commits_marked_text_relative_to_existing_mark(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("前后", window, cx);
                state.set_selected_range("前".len().."前".len(), false, window, cx);

                state.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
                state.replace_and_mark_text_in_range(Some(0..1), "ni", Some(2..2), window, cx);
                assert_eq!(state.text.to_string(), "前ni后");
                assert_eq!(
                    state.ime_marked_range,
                    Some(("前".len().."前ni".len()).into())
                );
                assert!(state.has_ime_marked_text());
                assert_eq!(state.selected_range(), "前ni".len().."前ni".len());
                assert_valid_utf8_range(&state.text, state.selected_range());

                state.replace_and_mark_text_in_range(Some(0..2), "你", Some(1..1), window, cx);
                assert_eq!(state.text.to_string(), "前你后");
                assert_eq!(
                    state.ime_marked_range,
                    Some(("前".len().."前你".len()).into())
                );
                assert!(state.has_ime_marked_text());
                assert_eq!(state.selected_range(), "前你".len().."前你".len());
                assert_valid_utf8_range(&state.text, state.selected_range());

                state.replace_text_in_range(Some(1..2), "好", window, cx);
                assert_eq!(state.text.to_string(), "前好后");
                assert_eq!(state.ime_marked_range, None);
                assert!(!state.has_ime_marked_text());
                assert_eq!(state.selected_range(), "前好".len().."前好".len());
                assert_valid_utf8_range(&state.text, state.selected_range());
                assert_eq!(state.selected_text_string(), "");
            });
        });
    }

    #[gpui::test]
    fn sibling_prepaint_sees_current_input_layout_on_first_frame(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;
        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("first line", window, cx);
            });
        });
        assert!(cx.update(|_, cx| {
            input.read_with(cx, |state, _| {
                state.last_layout.is_none() && state.last_bounds.is_none()
            })
        }));

        let observed = Rc::new(RefCell::new(None));
        let observed_in_overlay = observed.clone();
        let input_in_surface = input.clone();
        let input_in_overlay = input.clone();

        cx.draw(
            point(px(0.), px(0.)),
            gpui::size(px(800.), px(240.)),
            move |_, _| {
                div()
                    .relative()
                    .flex()
                    .size_full()
                    .child(input_in_surface)
                    .child(
                        canvas(
                            move |_, _, cx| {
                                *observed_in_overlay.borrow_mut() =
                                    input_in_overlay.read(cx).range_to_bounds(&(0..0));
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
            },
        );

        assert!(
            observed.borrow().is_some(),
            "a sibling overlay must see input geometry produced earlier in the same prepaint pass"
        );
    }

    #[gpui::test]
    fn sibling_prepaint_sees_current_input_layout_after_scroll(cx: &mut TestAppContext) {
        let (input, cx) = new_detached_code_editor(cx);
        let cx: &mut VisualTestContext = cx;
        let text = (0..40)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value(text, window, cx);
            });
        });
        draw_input(cx, &input);

        let marker_offset =
            cx.update(|_, cx| input.read_with(cx, |state, _| state.text.line_start_offset(5)));
        let before_scroll = cx.update(|_, cx| {
            input.read_with(cx, |state, _| {
                state
                    .range_to_bounds(&(marker_offset..marker_offset))
                    .expect("marker line should be visible before scrolling")
            })
        });

        cx.update(|_, cx| {
            input.update(cx, |state, _| {
                state.scroll_handle.set_offset(point(px(0.), px(-60.)));
            });
        });

        let observed = Rc::new(RefCell::new(None));
        let observed_in_overlay = observed.clone();
        let input_in_surface = input.clone();
        let input_in_overlay = input.clone();

        cx.draw(
            point(px(0.), px(0.)),
            gpui::size(px(800.), px(240.)),
            move |_, _| {
                div()
                    .relative()
                    .flex()
                    .size_full()
                    .child(input_in_surface)
                    .child(
                        canvas(
                            move |_, _, cx| {
                                *observed_in_overlay.borrow_mut() = input_in_overlay
                                    .read(cx)
                                    .range_to_bounds(&(marker_offset..marker_offset));
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
            },
        );

        let after_scroll = cx.update(|_, cx| {
            input.read_with(cx, |state, _| {
                state
                    .range_to_bounds(&(marker_offset..marker_offset))
                    .expect("marker line should remain visible after scrolling")
            })
        });
        assert_ne!(
            before_scroll, after_scroll,
            "the test must move the marker geometry"
        );
        assert_eq!(
            *observed.borrow(),
            Some(after_scroll),
            "a sibling overlay must not lag one frame behind the scrolled input"
        );
    }

    #[gpui::test]
    fn read_only_rejects_user_edits_but_allows_programmatic_updates(cx: &mut TestAppContext) {
        let (input, cx) = new_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("before", window, cx);
                state.read_only = true;
                state.replace_text_in_range(None, "blocked", window, cx);
                assert_eq!("before", state.text.to_string());

                state.set_value("after", window, cx);
                assert_eq!("after", state.text.to_string());
                assert!(state.read_only);
            });
        });
    }

    #[gpui::test]
    fn replace_text_range_preserves_selection_and_scroll(cx: &mut TestAppContext) {
        let (input, cx) = new_code_editor(cx);
        let cx: &mut VisualTestContext = cx;
        let text = (0..120)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value(text, window, cx);
                state.selected_range = (6..11).into();
                state.selection_reversed = false;
                state.scroll_handle.set_offset(point(px(0.), px(-120.)));
            });
        });
        let before_scroll =
            cx.update(|_, cx| input.read_with(cx, |state, _| state.scroll_offset()));
        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.replace_text_range(0..5, "longer", window, cx);
            });
        });
        draw_input(cx, &input);
        let (selection, scroll) = cx.update(|_, cx| {
            input.read_with(cx, |state, _| {
                (state.selected_range(), state.scroll_offset())
            })
        });
        assert_eq!(7..12, selection);
        assert_eq!(before_scroll, scroll);
    }

    #[gpui::test]
    fn set_auto_grow_mode_keeps_code_editor_capabilities(cx: &mut TestAppContext) {
        let (input, cx) = new_code_editor(cx);
        let cx: &mut VisualTestContext = cx;
        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_auto_grow_mode(1, 12, window, cx);
                assert!(state.mode.is_code_editor());
                assert!(state.mode.is_auto_grow());
                assert_eq!(12, state.mode.max_rows());
            });
        });
    }

    #[gpui::test]
    fn initial_syntax_highlighter_is_built_after_render_returns(cx: &mut TestAppContext) {
        let (input, cx) = new_code_editor(cx);
        let cx: &mut VisualTestContext = cx;
        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value("SELECT 1;", window, cx);
            });
        });

        draw_input(cx, &input);
        let initialized_during_render = cx.update(|_, cx| {
            input.read_with(cx, |state, _| {
                state
                    .mode
                    .highlighter()
                    .is_some_and(|highlighter| highlighter.borrow().is_some())
            })
        });
        assert!(
            !initialized_during_render,
            "render must not synchronously construct a syntax highlighter"
        );

        cx.run_until_parked();
        let highlighted_text = cx.update(|_, cx| {
            input.read_with(cx, |state, _| {
                state.mode.highlighter().and_then(|highlighter| {
                    highlighter
                        .borrow()
                        .as_ref()
                        .map(|highlighter| highlighter.text().to_string())
                })
            })
        });
        assert_eq!(Some("SELECT 1;".to_owned()), highlighted_text);
    }

    #[gpui::test]
    fn line_decorations_preserve_blank_line_numbers(cx: &mut TestAppContext) {
        let (input, cx) = new_code_editor(cx);
        let cx: &mut VisualTestContext = cx;
        let decorations: Rc<[InputLineDecoration]> = vec![
            InputLineDecoration {
                line_number: Some(10),
                background: None,
            },
            InputLineDecoration {
                line_number: None,
                background: Some(gpui::red()),
            },
        ]
        .into();

        cx.update(|_, cx| {
            input.update(cx, |state, cx| {
                state.set_line_decorations(decorations.clone(), cx);
                assert_eq!(
                    Some(10),
                    state.line_decorations.as_ref().unwrap()[0].line_number
                );
                assert_eq!(
                    None,
                    state.line_decorations.as_ref().unwrap()[1].line_number
                );
            });
        });
    }

    #[gpui::test]
    fn test_highlighting_preserved_after_fold(cx: &mut TestAppContext) {
        use crate::highlighter::HighlightTheme;
        use crate::input::display_map::FoldRange;

        let (input, cx) = new_code_editor(cx);
        let cx: &mut VisualTestContext = cx;

        // SQL text: fold the SELECT..WHERE block, verify comments keep highlighting.
        // Lines 0-9: SELECT block (fold range 0..9 hides lines 1-8)
        // Line 10+: comments that must keep highlighting
        let text = "\
SELECT *
FROM users
WHERE id = 1
AND name = 'test'
AND active = true
AND role = 'admin'
AND age > 18
AND status = 'ok'
AND country = 'US'
ORDER BY id

-- Comment 1
-- Comment 2
-- Comment 3";

        cx.update(|window, cx| {
            input.update(cx, |state, cx| {
                state.set_value(text, window, cx);
            });
        });
        draw_input(cx, &input);

        // Grab styles for "-- Comment 1" (line 11) before folding
        let theme = HighlightTheme::default_dark();
        let comment_line = 11;
        let comment_start = cx.update(|_, cx| {
            input.read_with(cx, |state, _| state.text.line_start_offset(comment_line))
        });
        let styles_before: Vec<(Range<usize>, gpui::HighlightStyle)> = cx.update(|_, cx| {
            input.read_with(cx, |state, _| {
                let mode = &state.mode;
                if let crate::input::mode::InputMode::CodeEditor { highlighter, .. } = mode {
                    let h = highlighter.borrow();
                    if let Some(h) = h.as_ref() {
                        let line_end = state.text.line_end_offset(comment_line);
                        return h.styles(&(comment_start..line_end), &theme);
                    }
                }
                vec![]
            })
        });

        // Fold at line 0 with range 0..9 (hides lines 1-8)
        cx.update(|_, cx| {
            input.update(cx, |state, _cx| {
                state
                    .display_map
                    .set_fold_candidates(vec![FoldRange::new(0, 9)]);
                state.display_map.set_folded(0, true);
            });
        });
        draw_input(cx, &input);

        // Verify fold is active and lines 1-8 are hidden
        cx.update(|_, cx| {
            input.read_with(cx, |state, _| {
                assert!(state.display_map.is_folded_at(0));
                for line in 1..=8 {
                    assert!(
                        state.display_map.is_buffer_line_hidden(line),
                        "Line {} should be hidden",
                        line
                    );
                }
                assert!(
                    !state.display_map.is_buffer_line_hidden(9),
                    "Line 9 (ORDER BY) should be visible"
                );
            });
        });

        // Get styles for the same comment line after folding
        let styles_after: Vec<(Range<usize>, gpui::HighlightStyle)> = cx.update(|_, cx| {
            input.read_with(cx, |state, _| {
                let mode = &state.mode;
                if let crate::input::mode::InputMode::CodeEditor { highlighter, .. } = mode {
                    let h = highlighter.borrow();
                    if let Some(h) = h.as_ref() {
                        let line_end = state.text.line_end_offset(comment_line);
                        return h.styles(&(comment_start..line_end), &theme);
                    }
                }
                vec![]
            })
        });

        let colored_before: Vec<_> = styles_before
            .iter()
            .filter(|(_, s)| s.color.is_some())
            .cloned()
            .collect();
        let colored_after: Vec<_> = styles_after
            .iter()
            .filter(|(_, s)| s.color.is_some())
            .cloned()
            .collect();

        assert_eq!(
            colored_before, colored_after,
            "Comment highlighting must be identical before and after folding.\n\
             Before: {:?}\nAfter: {:?}",
            colored_before, colored_after
        );
    }

    #[gpui::test]
    fn select_to_scrolls_reversed_selection_anchor_into_view(cx: &mut TestAppContext) {
        let (input, cx) = new_code_editor(cx);
        let cx: &mut VisualTestContext = cx;
        let text = (0..120)
            .map(|line| format!("select {line};"))
            .collect::<Vec<_>>()
            .join("\n");

        cx.update(|window, cx| {
            input.update(cx, |state, cx| state.set_value(text, window, cx));
        });
        draw_input(cx, &input);

        let bottom_offset =
            cx.update(|_, cx| input.read_with(cx, |state, _| state.text.line_start_offset(100)));
        cx.update(|_, cx| {
            input.update(cx, |state, cx| {
                state.move_to(bottom_offset, Some(MoveDirection::Down), cx);
            });
        });
        draw_input(cx, &input);

        let before_scroll =
            cx.update(|_, cx| input.read_with(cx, |state, _| state.scroll_offset()));
        let target_offset =
            cx.update(|_, cx| input.read_with(cx, |state, _| state.text.line_start_offset(40)));
        cx.update(|_, cx| {
            input.update(cx, |state, cx| {
                state.select_to(target_offset, cx);
            });
        });
        draw_input(cx, &input);

        let after_select = cx.update(|_, cx| {
            input.read_with(cx, |state, _| {
                (
                    state.cursor(),
                    state.selection_reversed,
                    state.selected_range,
                    state.scroll_offset(),
                )
            })
        });
        assert!(
            after_select.3.y > before_scroll.y,
            "expected upward selection to move viewport up, bottom={bottom_offset}, target={target_offset}, before={before_scroll:?}, state={after_select:?}"
        );
    }

    #[gpui::test]
    fn scroll_to_line_positions_the_requested_row_in_view(cx: &mut TestAppContext) {
        let (input, cx) = new_code_editor(cx);
        let cx: &mut VisualTestContext = cx;
        let text = (0..120)
            .map(|line| format!("select {line};"))
            .collect::<Vec<_>>()
            .join("\n");

        cx.update(|window, cx| {
            input.update(cx, |state, cx| state.set_value(text, window, cx));
        });
        draw_input(cx, &input);

        cx.update(|_, cx| {
            input.update(cx, |state, cx| state.scroll_to_line(80, cx));
        });
        draw_input(cx, &input);

        let (offset, visible_range) = cx.update(|_, cx| {
            input.read_with(cx, |state, _| {
                (state.scroll_offset(), state.visible_row_range())
            })
        });
        assert!(offset.y < px(0.0));
        assert!(
            visible_range
                .as_ref()
                .is_some_and(|range| range.contains(&80)),
            "expected row 80 to be visible, got {visible_range:?}"
        );
    }
}
