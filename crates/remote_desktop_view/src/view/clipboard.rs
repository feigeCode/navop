use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{ClipboardEntry, ClipboardItem, Context, ExternalPaths, Window};
use remote_desktop::{RemoteDesktopInput, RemoteDesktopProtocol};

use super::RemoteDesktopView;

const CLIPBOARD_SYNC_INTERVAL: Duration = Duration::from_millis(500);
const REMOTE_CLIPBOARD_TRANSFER_BIT: u64 = 1 << 63;
pub(super) const FIRST_LOCAL_CLIPBOARD_TRANSFER_ID: u64 = 1;
const REMOTE_CLIPBOARD_STAGING_ROOT: &str = "navop-rdp-clipboard";

pub(super) fn clipboard_text_supported(protocol: RemoteDesktopProtocol, text: &str) -> bool {
    protocol == RemoteDesktopProtocol::Rdp || text.is_ascii()
}

pub(super) fn clipboard_files_supported(protocol: RemoteDesktopProtocol) -> bool {
    protocol == RemoteDesktopProtocol::Rdp
}

pub(super) fn allocate_local_clipboard_transfer_id(next_id: &mut u64) -> u64 {
    if *next_id == 0 || *next_id >= REMOTE_CLIPBOARD_TRANSFER_BIT {
        *next_id = FIRST_LOCAL_CLIPBOARD_TRANSFER_ID;
    }
    let transfer_id = *next_id;
    *next_id = transfer_id
        .checked_add(1)
        .filter(|next| *next < REMOTE_CLIPBOARD_TRANSFER_BIT)
        .unwrap_or(FIRST_LOCAL_CLIPBOARD_TRANSFER_ID);
    transfer_id
}

fn is_remote_clipboard_transfer_id(transfer_id: u64) -> bool {
    transfer_id & REMOTE_CLIPBOARD_TRANSFER_BIT != 0
}

fn remote_clipboard_staging_root() -> PathBuf {
    std::env::temp_dir().join(REMOTE_CLIPBOARD_STAGING_ROOT)
}

fn validate_remote_clipboard_paths(paths: &[String]) -> anyhow::Result<Vec<PathBuf>> {
    validate_remote_clipboard_paths_in_root(&remote_clipboard_staging_root(), paths)
}

fn validate_remote_clipboard_paths_in_root(
    staging_root: &Path,
    paths: &[String],
) -> anyhow::Result<Vec<PathBuf>> {
    anyhow::ensure!(!paths.is_empty(), "remote clipboard file list is empty");
    let canonical_root = std::fs::canonicalize(staging_root).map_err(|error| {
        anyhow::anyhow!("remote clipboard staging root is unavailable: {error}")
    })?;
    let mut validated = Vec::with_capacity(paths.len());
    for raw_path in paths {
        let path = PathBuf::from(raw_path);
        anyhow::ensure!(path.is_absolute(), "remote clipboard path is not absolute");
        let canonical_path = std::fs::canonicalize(&path)
            .map_err(|error| anyhow::anyhow!("remote clipboard path is unavailable: {error}"))?;
        anyhow::ensure!(
            canonical_path != canonical_root && canonical_path.starts_with(&canonical_root),
            "remote clipboard path escaped its staging root"
        );
        validated.push(canonical_path);
    }
    Ok(validated)
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum LocalClipboardContent {
    Files(Vec<String>),
    Text(String),
    Other,
}

pub(super) fn classify_local_clipboard(item: &ClipboardItem) -> LocalClipboardContent {
    if let Some(paths) = item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::ExternalPaths(paths) => {
            let paths: Vec<String> = paths
                .paths()
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            (!paths.is_empty()).then_some(paths)
        }
        ClipboardEntry::Image(_) | ClipboardEntry::String(_) => None,
    }) {
        return LocalClipboardContent::Files(paths);
    }

    item.text()
        .map(LocalClipboardContent::Text)
        .unwrap_or(LocalClipboardContent::Other)
}

impl RemoteDesktopView {
    pub(super) fn apply_remote_clipboard(&mut self, text: String, cx: &mut Context<Self>) {
        if self.last_clipboard_text.as_deref() == Some(text.as_str()) {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        self.last_clipboard_text = Some(text);
        self.last_clipboard_files = None;
        self.last_clipboard_sync_at = Some(Instant::now());
    }

    pub(super) fn apply_remote_clipboard_files(
        &mut self,
        transfer_id: u64,
        paths: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !is_remote_clipboard_transfer_id(transfer_id) {
            tracing::warn!(
                transfer_id,
                "rejecting remote clipboard files with a local transfer identifier"
            );
            self.notify_clipboard_files_invalid(window, cx);
            return;
        }
        let paths = match validate_remote_clipboard_paths(&paths) {
            Ok(paths) => paths,
            Err(error) => {
                tracing::warn!(
                    transfer_id,
                    error = %error,
                    "rejecting unsafe remote clipboard file paths"
                );
                self.notify_clipboard_files_invalid(window, cx);
                return;
            }
        };
        self.install_remote_clipboard_files(paths, window, cx);
    }

    fn install_remote_clipboard_files(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path_strings = paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let count = paths.len();
        cx.write_to_clipboard(ClipboardItem {
            entries: vec![ClipboardEntry::ExternalPaths(ExternalPaths(
                paths.into_iter().collect(),
            ))],
        });
        self.last_clipboard_files = Some(path_strings);
        self.last_clipboard_text = None;
        self.last_clipboard_sync_at = Some(Instant::now());
        self.notify_clipboard_files_received(count, window, cx);
    }

    pub(super) fn sync_local_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) || !self.clipboard_sync_due() {
            return;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        match classify_local_clipboard(&item) {
            LocalClipboardContent::Files(paths) => self.sync_local_clipboard_files(paths),
            LocalClipboardContent::Text(text) => self.sync_local_clipboard_text(text, window, cx),
            LocalClipboardContent::Other => {}
        }
    }

    fn clipboard_sync_due(&mut self) -> bool {
        if self
            .last_clipboard_sync_at
            .is_some_and(|synced_at| synced_at.elapsed() < CLIPBOARD_SYNC_INTERVAL)
        {
            return false;
        }
        self.last_clipboard_sync_at = Some(Instant::now());
        true
    }

    fn sync_local_clipboard_files(&mut self, paths: Vec<String>) {
        if self.last_clipboard_files.as_ref() == Some(&paths) {
            return;
        }
        self.last_clipboard_files = Some(paths.clone());
        self.last_clipboard_text = None;
        if clipboard_files_supported(self.options.protocol) {
            let transfer_id =
                allocate_local_clipboard_transfer_id(&mut self.next_clipboard_transfer_id);
            self.send_input(RemoteDesktopInput::ClipboardFiles { transfer_id, paths });
        }
    }

    fn sync_local_clipboard_text(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.last_clipboard_text.as_deref() == Some(text.as_str()) {
            return;
        }
        self.last_clipboard_text = Some(text.clone());
        self.last_clipboard_files = None;
        if !clipboard_text_supported(self.options.protocol, &text) {
            self.notify_vnc_clipboard_ascii_warning(window, cx);
            return;
        }
        self.send_input(RemoteDesktopInput::ClipboardText { text });
    }
}

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;
