use crate::{
    DirectorySizeState, FileItem, FileListPanel, SftpView, exec_remote_command,
    generate_unique_name, join_remote_path,
};
use anyhow::{Context as _, Result};
use gpui::{AppContext, Context, Entity, ParentElement, Styled, Window, div, px};
use gpui_component::{WindowExt, h_flex, notification::Notification, v_flex};
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;
use sftp::{
    DirectoryConflictPolicy, RemoteFileOperation, ServerCopyItem, SftpClient,
    build_remote_file_command, calculate_directory_size, remote_path_is_same_or_descendant,
};
use ssh::SshSessionManager;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClipboardEndpoint {
    Local,
    RemoteLeft,
    RemoteRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileClipboardKind {
    Copy,
    Cut,
}

#[derive(Clone, Debug)]
pub(crate) struct ClipboardEntry {
    name: String,
    full_path: String,
    is_dir: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct FileClipboard {
    kind: FileClipboardKind,
    endpoint: ClipboardEndpoint,
    entries: Vec<ClipboardEntry>,
}

fn local_path_is_same_or_descendant(parent: &Path, candidate: &Path) -> bool {
    let parent = normalize_local_path(parent);
    let candidate = normalize_local_path(candidate);
    candidate == parent || candidate.starts_with(&parent)
}

fn normalize_local_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn copy_local_entry(source: &Path, target: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)
        .with_context(|| format!("failed to read {}", source.display()))?;
    if metadata.is_dir() {
        std::fs::create_dir_all(target)
            .with_context(|| format!("failed to create {}", target.display()))?;
        for entry in std::fs::read_dir(source)
            .with_context(|| format!("failed to read {}", source.display()))?
        {
            let entry = entry?;
            copy_local_entry(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else {
        std::fs::copy(source, target).with_context(|| {
            format!(
                "failed to copy {} to {}",
                source.display(),
                target.display()
            )
        })?;
    }
    Ok(())
}

fn calculate_local_directory_size(path: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in
        std::fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))?
    {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total = total.saturating_add(calculate_local_directory_size(&entry.path())?);
        } else {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

fn property_row(label: String, value: String) -> impl gpui::IntoElement {
    h_flex()
        .gap_3()
        .child(div().w(px(96.)).text_sm().child(label))
        .child(div().flex_1().text_sm().child(value))
}

fn can_paste_file_clipboard(
    clipboard: Option<&FileClipboard>,
    endpoint: ClipboardEndpoint,
) -> bool {
    clipboard
        .is_some_and(|clipboard| clipboard.endpoint == endpoint && !clipboard.entries.is_empty())
}

impl SftpView {
    pub(crate) fn can_paste_file_clipboard(&self, endpoint: ClipboardEndpoint) -> bool {
        can_paste_file_clipboard(self.file_clipboard.as_ref(), endpoint)
    }

    pub(crate) fn store_file_clipboard(
        &mut self,
        endpoint: ClipboardEndpoint,
        kind: FileClipboardKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (panel, base_path, remote) = match endpoint {
            ClipboardEndpoint::Local => (
                self.local_panel.clone(),
                self.local_current_path.to_string_lossy().to_string(),
                false,
            ),
            ClipboardEndpoint::RemoteLeft => {
                let Some(left) = self.left_remote.as_ref() else {
                    return;
                };
                (self.local_panel.clone(), left.current_path.clone(), true)
            }
            ClipboardEndpoint::RemoteRight => (
                self.remote_panel.clone(),
                self.remote_current_path.clone(),
                true,
            ),
        };

        let entries = panel
            .read(cx)
            .selected_items(cx)
            .into_iter()
            .map(|item| ClipboardEntry {
                full_path: if remote {
                    join_remote_path(&base_path, &item.name)
                } else {
                    Path::new(&base_path)
                        .join(&item.name)
                        .to_string_lossy()
                        .to_string()
                },
                name: item.name,
                is_dir: item.is_dir,
            })
            .collect::<Vec<_>>();

        if entries.is_empty() {
            return;
        }

        self.file_clipboard = Some(FileClipboard {
            kind,
            endpoint,
            entries,
        });
        let key = match kind {
            FileClipboardKind::Copy => "Notification.clipboard_copied",
            FileClipboardKind::Cut => "Notification.clipboard_cut",
        };
        window.push_notification(Notification::success(t!(key)), cx);
    }

    pub(crate) fn paste_file_clipboard(
        &mut self,
        endpoint: ClipboardEndpoint,
        target_dir: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(clipboard) = self.file_clipboard.clone() else {
            window.push_notification(Notification::info(t!("Error.clipboard_empty")), cx);
            return;
        };
        if clipboard.endpoint != endpoint {
            window.push_notification(
                Notification::error(t!("Error.incompatible_paste_endpoint")),
                cx,
            );
            return;
        }

        match endpoint {
            ClipboardEndpoint::Local => {
                self.paste_local_clipboard(clipboard, PathBuf::from(target_dir), window, cx)
            }
            ClipboardEndpoint::RemoteLeft | ClipboardEndpoint::RemoteRight => {
                self.paste_remote_clipboard(clipboard, target_dir, window, cx)
            }
        }
    }

    fn paste_local_clipboard(
        &mut self,
        clipboard: FileClipboard,
        target_dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if clipboard.entries.iter().any(|entry| {
            entry.is_dir
                && local_path_is_same_or_descendant(Path::new(&entry.full_path), &target_dir)
        }) {
            window.push_notification(Notification::error(t!("Error.invalid_paste_target")), cx);
            return;
        }

        let kind = clipboard.kind;
        let task = Tokio::spawn(cx, async move {
            tokio::task::spawn_blocking(move || -> Result<()> {
                let mut used_names = std::fs::read_dir(&target_dir)?
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.file_name().to_string_lossy().to_string())
                    .collect::<HashSet<_>>();
                for entry in &clipboard.entries {
                    let target_name = if used_names.contains(&entry.name) {
                        generate_unique_name(&entry.name, &used_names)
                    } else {
                        entry.name.clone()
                    };
                    used_names.insert(target_name.clone());
                    let target = target_dir.join(target_name);
                    match clipboard.kind {
                        FileClipboardKind::Copy => {
                            copy_local_entry(Path::new(&entry.full_path), &target)?
                        }
                        FileClipboardKind::Cut => std::fs::rename(&entry.full_path, &target)
                            .with_context(|| {
                                format!(
                                    "failed to move {} to {}",
                                    entry.full_path,
                                    target.display()
                                )
                            })?,
                    }
                }
                Ok(())
            })
            .await
            .map_err(anyhow::Error::from)?
        });
        let view = cx.entity().downgrade();
        window
            .spawn(cx, async move |cx| {
                let result = task.await;
                let _ = view.update_in(cx, |this, window, cx| match result {
                    Ok(Ok(())) => {
                        if kind == FileClipboardKind::Cut {
                            this.file_clipboard = None;
                        }
                        this.refresh_local_dir(cx);
                        window.push_notification(
                            Notification::success(t!("Notification.paste_success")),
                            cx,
                        );
                    }
                    Ok(Err(error)) => window.push_notification(
                        Notification::error(t!("Error.paste_failed", error = error)),
                        cx,
                    ),
                    Err(error) => window.push_notification(
                        Notification::error(t!("Error.paste_failed", error = error)),
                        cx,
                    ),
                });
            })
            .detach();
    }

    fn paste_remote_clipboard(
        &mut self,
        clipboard: FileClipboard,
        target_dir: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if clipboard.entries.iter().any(|entry| {
            entry.is_dir && remote_path_is_same_or_descendant(&entry.full_path, &target_dir)
        }) {
            window.push_notification(Notification::error(t!("Error.invalid_paste_target")), cx);
            return;
        }

        let (client, config) = match clipboard.endpoint {
            ClipboardEndpoint::RemoteLeft => {
                let Some(left) = self.left_remote.as_ref() else {
                    return;
                };
                let Some(client) = left.client.clone() else {
                    return;
                };
                (client, left.config.clone())
            }
            ClipboardEndpoint::RemoteRight => {
                let Some(client) = self.sftp_client.clone() else {
                    return;
                };
                (client, self.sftp_config.clone())
            }
            ClipboardEndpoint::Local => return,
        };

        let session_manager = Arc::new(SshSessionManager::new(config));
        let endpoint = clipboard.endpoint;
        let kind = clipboard.kind;
        let task = Tokio::spawn(cx, async move {
            let mut client_guard = client.lock().await;
            let mut used_names = client_guard
                .list_dir(&target_dir)
                .await?
                .into_iter()
                .map(|entry| entry.name)
                .collect::<HashSet<_>>();
            let items = clipboard
                .entries
                .iter()
                .map(|entry| {
                    let target_name = if used_names.contains(&entry.name) {
                        generate_unique_name(&entry.name, &used_names)
                    } else {
                        entry.name.clone()
                    };
                    used_names.insert(target_name.clone());
                    ServerCopyItem {
                        source_path: entry.full_path.clone(),
                        target_path: join_remote_path(&target_dir, &target_name),
                        is_dir: entry.is_dir,
                        size: 0,
                        directory_conflict_policy: DirectoryConflictPolicy::Merge,
                    }
                })
                .collect::<Vec<_>>();

            drop(client_guard);
            let operation = match clipboard.kind {
                FileClipboardKind::Copy => RemoteFileOperation::Copy,
                FileClipboardKind::Cut => RemoteFileOperation::Move,
            };
            let command = build_remote_file_command(operation, &items)?;
            exec_remote_command(session_manager, &command).await?;
            Ok::<_, anyhow::Error>(())
        });

        let view = cx.entity().downgrade();
        window
            .spawn(cx, async move |cx| {
                let result = task.await;
                let _ = view.update_in(cx, |this, window, cx| match result {
                    Ok(Ok(())) => {
                        if kind == FileClipboardKind::Cut {
                            this.file_clipboard = None;
                        }
                        match endpoint {
                            ClipboardEndpoint::RemoteLeft => this.refresh_left_remote_dir(cx),
                            ClipboardEndpoint::RemoteRight => this.refresh_remote_dir(cx),
                            ClipboardEndpoint::Local => {}
                        }
                        window.push_notification(
                            Notification::success(t!("Notification.paste_success")),
                            cx,
                        );
                    }
                    Ok(Err(error)) => window.push_notification(
                        Notification::error(t!("Error.paste_failed", error = error)),
                        cx,
                    ),
                    Err(error) => window.push_notification(
                        Notification::error(t!("Error.paste_failed", error = error)),
                        cx,
                    ),
                });
            })
            .detach();
    }

    pub(crate) fn show_file_properties(
        &self,
        item: FileItem,
        full_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let size = if item.is_dir {
            match item.directory_size {
                DirectorySizeState::Unknown => t!("File.calculate").to_string(),
                DirectorySizeState::Calculating => t!("File.calculating").to_string(),
                DirectorySizeState::Ready(size) => crate::file_list_panel::format_file_size(size),
            }
        } else {
            crate::file_list_panel::format_file_size(item.size)
        };
        let modified: chrono::DateTime<chrono::Local> = item.modified.into();
        let permissions = if item.permissions.is_empty() {
            "-".to_string()
        } else {
            item.permissions.clone()
        };
        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog
                .title(t!("File.properties").to_string())
                .w(px(480.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(property_row(
                            t!("File.property_name").to_string(),
                            item.name.clone(),
                        ))
                        .child(property_row(
                            t!("File.property_path").to_string(),
                            full_path.clone(),
                        ))
                        .child(property_row(
                            t!("File.property_type").to_string(),
                            if item.is_dir {
                                t!("File.property_folder").to_string()
                            } else {
                                t!("File.property_file").to_string()
                            },
                        ))
                        .child(property_row(
                            t!("File.property_size").to_string(),
                            size.clone(),
                        ))
                        .child(property_row(
                            t!("File.property_modified").to_string(),
                            modified.format("%Y-%m-%d %H:%M:%S").to_string(),
                        ))
                        .child(property_row(
                            t!("File.property_permissions").to_string(),
                            permissions.clone(),
                        )),
                )
                .close_button(true)
        });
    }

    pub(crate) fn calculate_size_for_endpoint(
        &mut self,
        endpoint: ClipboardEndpoint,
        full_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel: Entity<FileListPanel> = match endpoint {
            ClipboardEndpoint::Local | ClipboardEndpoint::RemoteLeft => self.local_panel.clone(),
            ClipboardEndpoint::RemoteRight => self.remote_panel.clone(),
        };
        if !panel.update(cx, |panel, cx| {
            panel.set_directory_size_state(&full_path, DirectorySizeState::Calculating, cx)
        }) {
            return;
        }

        match endpoint {
            ClipboardEndpoint::Local => {
                let path = PathBuf::from(&full_path);
                let task = Tokio::spawn(cx, async move {
                    tokio::task::spawn_blocking(move || calculate_local_directory_size(&path))
                        .await
                        .map_err(anyhow::Error::from)?
                });
                Self::finish_directory_size_task(panel, full_path, task, window, cx);
            }
            ClipboardEndpoint::RemoteLeft | ClipboardEndpoint::RemoteRight => {
                let client: Option<Arc<Mutex<sftp::RusshSftpClient>>> = match endpoint {
                    ClipboardEndpoint::RemoteLeft => self
                        .left_remote
                        .as_ref()
                        .and_then(|left| left.client.clone()),
                    ClipboardEndpoint::RemoteRight => self.sftp_client.clone(),
                    ClipboardEndpoint::Local => None,
                };
                let Some(client) = client else {
                    panel.update(cx, |panel, cx| {
                        panel.set_directory_size_state(&full_path, DirectorySizeState::Unknown, cx);
                    });
                    return;
                };
                let path = full_path.clone();
                let task = Tokio::spawn(cx, async move {
                    let mut client = client.lock().await;
                    calculate_directory_size(&mut *client, &path, Arc::new(AtomicBool::new(false)))
                        .await
                });
                Self::finish_directory_size_task(panel, full_path, task, window, cx);
            }
        }
    }

    fn finish_directory_size_task(
        panel: Entity<FileListPanel>,
        full_path: String,
        task: gpui::Task<Result<Result<u64>, tokio::task::JoinError>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let window_handle = window.window_handle();
        window
            .spawn(cx, async move |cx| match task.await {
                Ok(Ok(size)) => {
                    let _ = panel.update(cx, |panel, cx| {
                        panel.set_directory_size_state(
                            &full_path,
                            DirectorySizeState::Ready(size),
                            cx,
                        );
                    });
                }
                Ok(Err(error)) => {
                    let _ = panel.update(cx, |panel, cx| {
                        panel.set_directory_size_state(&full_path, DirectorySizeState::Unknown, cx);
                    });
                    cx.update_window(window_handle, |_, window, cx| {
                        window.push_notification(
                            Notification::error(t!("Error.calculate_size_failed", error = error)),
                            cx,
                        );
                    })
                    .ok();
                }
                Err(error) => {
                    let _ = panel.update(cx, |panel, cx| {
                        panel.set_directory_size_state(&full_path, DirectorySizeState::Unknown, cx);
                    });
                    cx.update_window(window_handle, |_, window, cx| {
                        window.push_notification(
                            Notification::error(t!("Error.calculate_size_failed", error = error)),
                            cx,
                        );
                    })
                    .ok();
                }
            })
            .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ClipboardEndpoint, ClipboardEntry, FileClipboard, FileClipboardKind,
        calculate_local_directory_size, can_paste_file_clipboard, local_path_is_same_or_descendant,
    };
    use std::fs;
    use std::time::SystemTime;

    fn clipboard(endpoint: ClipboardEndpoint) -> FileClipboard {
        FileClipboard {
            kind: FileClipboardKind::Copy,
            endpoint,
            entries: vec![ClipboardEntry {
                name: "notes.txt".to_string(),
                full_path: "/tmp/notes.txt".to_string(),
                is_dir: false,
            }],
        }
    }

    #[test]
    fn paste_availability_requires_a_matching_non_empty_endpoint() {
        let matching = Some(clipboard(ClipboardEndpoint::RemoteRight));
        assert!(can_paste_file_clipboard(
            matching.as_ref(),
            ClipboardEndpoint::RemoteRight
        ));
        assert!(!can_paste_file_clipboard(
            matching.as_ref(),
            ClipboardEndpoint::RemoteLeft
        ));

        let empty = FileClipboard {
            kind: FileClipboardKind::Cut,
            endpoint: ClipboardEndpoint::Local,
            entries: Vec::new(),
        };
        assert!(!can_paste_file_clipboard(
            Some(&empty),
            ClipboardEndpoint::Local
        ));
        assert!(!can_paste_file_clipboard(None, ClipboardEndpoint::Local));
    }

    #[test]
    fn local_descendant_check_respects_component_boundaries() {
        assert!(local_path_is_same_or_descendant(
            std::path::Path::new("/tmp/a"),
            std::path::Path::new("/tmp/a/b")
        ));
        assert!(!local_path_is_same_or_descendant(
            std::path::Path::new("/tmp/a"),
            std::path::Path::new("/tmp/ab")
        ));
    }

    #[test]
    fn local_directory_size_counts_files_recursively() {
        let root = std::env::temp_dir().join(format!(
            "navop-sftp-size-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("a"), b"123").unwrap();
        fs::write(root.join("nested/b"), b"4567").unwrap();

        assert_eq!(7, calculate_local_directory_size(&root).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
}
