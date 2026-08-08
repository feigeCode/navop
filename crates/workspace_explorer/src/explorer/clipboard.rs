use super::WorkspaceExplorer;
use gpui::{Context, KeyBinding, Window, actions};
use gpui_component::{WindowExt as _, notification::Notification};
use rust_i18n::t;
use std::path::{Path, PathBuf};

actions!(
    workspace_explorer,
    [CutSelectedEntry, CopySelectedEntry, PasteEntry]
);

pub(super) const WORKSPACE_EXPLORER_KEY_CONTEXT: &str = "WorkspaceExplorer";

#[cfg(target_os = "macos")]
const CUT_SHORTCUT: &str = "cmd-x";
#[cfg(not(target_os = "macos"))]
const CUT_SHORTCUT: &str = "ctrl-x";
#[cfg(target_os = "macos")]
const COPY_SHORTCUT: &str = "cmd-c";
#[cfg(not(target_os = "macos"))]
const COPY_SHORTCUT: &str = "ctrl-c";
#[cfg(target_os = "macos")]
const PASTE_SHORTCUT: &str = "cmd-v";
#[cfg(not(target_os = "macos"))]
const PASTE_SHORTCUT: &str = "ctrl-v";

pub(crate) fn keybindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new(
            CUT_SHORTCUT,
            CutSelectedEntry,
            Some(WORKSPACE_EXPLORER_KEY_CONTEXT),
        ),
        KeyBinding::new(
            COPY_SHORTCUT,
            CopySelectedEntry,
            Some(WORKSPACE_EXPLORER_KEY_CONTEXT),
        ),
        KeyBinding::new(
            PASTE_SHORTCUT,
            PasteEntry,
            Some(WORKSPACE_EXPLORER_KEY_CONTEXT),
        ),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileClipboardKind {
    Copy,
    Cut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FileClipboard {
    pub(super) source: PathBuf,
    pub(super) kind: FileClipboardKind,
}

impl WorkspaceExplorer {
    pub(super) fn cut_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.set_file_clipboard(path, FileClipboardKind::Cut, window, cx);
    }

    pub(super) fn copy_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.set_file_clipboard(path, FileClipboardKind::Copy, window, cx);
    }

    pub(super) fn paste_into(
        &mut self,
        destination: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(clipboard) = self.file_clipboard.clone() else {
            return;
        };
        self.execute_paste(clipboard, destination, window, cx);
    }

    pub(super) fn has_file_clipboard(&self) -> bool {
        self.file_clipboard
            .as_ref()
            .is_some_and(|clipboard| std::fs::symlink_metadata(&clipboard.source).is_ok())
    }

    pub(super) fn cut_selected_entry(
        &mut self,
        _: &CutSelectedEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_handle.is_focused(window) {
            return;
        }
        if let Some(path) = self.selected_path.clone() {
            self.cut_path(path, window, cx);
        }
    }

    pub(super) fn copy_selected_entry(
        &mut self,
        _: &CopySelectedEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_handle.is_focused(window) {
            return;
        }
        if let Some(path) = self.selected_path.clone() {
            self.copy_path(path, window, cx);
        }
    }

    pub(super) fn paste_entry(
        &mut self,
        _: &PasteEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_handle.is_focused(window) {
            return;
        }
        let destination = self.selected_paste_destination();
        self.paste_into(destination, window, cx);
    }

    fn set_file_clipboard(
        &mut self,
        source: PathBuf,
        kind: FileClipboardKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_operation_running {
            return;
        }
        self.file_clipboard = Some(FileClipboard { source, kind });
        let message = match kind {
            FileClipboardKind::Copy => t!("WorkspaceExplorer.file_action.copied"),
            FileClipboardKind::Cut => t!("WorkspaceExplorer.file_action.cut_ready"),
        };
        window.push_notification(
            Notification::success(message.to_string()).autohide(true),
            cx,
        );
        cx.notify();
    }

    fn selected_paste_destination(&self) -> PathBuf {
        let Some(selected) = self.selected_path.as_ref() else {
            return self.root.clone();
        };
        if self.is_known_directory(selected) {
            return selected.clone();
        }
        selected
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone())
    }

    fn is_known_directory(&self, path: &Path) -> bool {
        self.listings
            .values()
            .flatten()
            .any(|entry| entry.path == path && entry.is_dir)
    }
}
