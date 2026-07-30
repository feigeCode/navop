//! Navop's embedded block-based Markdown editor.
//!
//! The editor core is derived from Velotype 0.7.0 at upstream commit
//! `050943e`, then adapted for Navop's host rendering and WASM language services.

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
    BoldSelection, CodeSelection, ItalicSelection, Redo, SelectAll, UnderlineSelection, Undo,
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
    cx.set_global(i18n::I18nManager::default());
    cx.set_global(theme::ThemeManager::default());
    components::init(cx);
}
