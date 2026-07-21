//! Local workspace file browsing, editing, and Git change inspection for GPUI hosts.
//!
//! The crate intentionally keeps filesystem and Git behavior out of host views. A host creates a
//! [`WorkspaceEditor`] and passes it to [`WorkspaceExplorer`]. Selecting a file or Git change in
//! the explorer opens the corresponding document in the editor.

rust_i18n::i18n!("locales", fallback = "en");

mod editor;
mod explorer;
mod file_system;
mod git;
mod model;
mod theme;

pub use editor::{WorkspaceEditor, WorkspaceEditorEvent};
pub use explorer::{
    ExplorerFramePlacement, WorkspaceExplorer, WorkspaceExplorerConfig, WorkspaceExplorerEvent,
};
pub use git::{GitChange, GitChangeKind, GitRepository};
pub use theme::WorkspaceTheme;
