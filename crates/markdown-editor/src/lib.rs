//! Navop's embedded block-based Markdown editor.
//!
//! The editor core is derived from Velotype 0.7.0 at upstream commit
//! `050943e`, then adapted for Navop's host rendering and WASM language services.

rust_i18n::i18n!("locales", fallback = "en");

mod block_render;
pub mod components;
mod config;
pub mod editor;
mod host_services;
mod i18n;
mod icons;
mod navop_host;
mod navop_theme;
mod net;
mod theme;
mod wasm_highlight;

use gpui::App;

pub use block_render::{
    MarkdownBlockRenderArtifact, MarkdownBlockRenderKind, MarkdownBlockRenderProvider,
    MarkdownBlockRenderRequest,
};
pub use components::{
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
pub use editor::{EditorEvent as MarkdownEditorEvent, ViewMode};
pub use host_services::{
    BlockRenderArtifact, BlockRenderKind, BlockRenderProvider, BlockRenderRequest,
    CodeHighlightProvider, CodeHighlightRequest, CodeHighlightResult, CodeHighlightService,
    CodeHighlightSpan, CodeHighlightStyle, EditorHostServices, EditorHostTheme,
};
pub use navop_host::markdown_editor_host_services;
pub use navop_theme::MarkdownEditorTheme;

/// The embedded editor type exposed to Notes.
pub type MarkdownEditor = editor::Editor;

/// Installs globals and key bindings required by the embedded editor.
pub fn init(cx: &mut App) {
    let locale = gpui_component::locale();
    rust_i18n::set_locale(i18n::normalize_locale(&locale));
    cx.set_global(theme::ThemeManager::default());
    components::init(cx);
}

/// Synchronises the embedded editor language with Navop's active locale.
pub fn set_locale(locale: &str, _cx: &mut App) -> bool {
    let locale = i18n::normalize_locale(locale);
    let changed = &*rust_i18n::locale() != locale;
    rust_i18n::set_locale(locale);
    changed
}
