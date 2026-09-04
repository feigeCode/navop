//! 右键菜单功能处理模块
//!
//! 本模块实现 FileListPanel 右键菜单的所有功能

use crate::file_clipboard::{ClipboardEndpoint, FileClipboardKind};
use crate::{
    ActiveExtract, ExtractConflictAction, FileListPanelEvent, PanelSide, SftpView, SftpViewEvent,
    build_remote_extract_command, build_remote_extract_conflict_check_command, exec_remote_command,
    join_remote_path, remote_extract_has_conflict,
};
use gpui::{
    App, AppContext, ClipboardItem, Context, ParentElement, PathPromptOptions, Styled, Window, div,
    px,
};
use gpui_component::{
    WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogButtonProps,
    input::{Input, InputState},
    notification::Notification,
    v_flex,
};
use one_core::gpui_tokio::Tokio;
use rust_i18n::t;
use sftp::SftpClient;
use ssh::SshSessionManager;
use std::path::PathBuf;
use std::sync::Arc;

impl SftpView {
    fn handle_left_remote_context_menu_event(
        &mut self,
        event: &FileListPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            FileListPanelEvent::CopyEntries => {
                self.store_file_clipboard(
                    ClipboardEndpoint::RemoteLeft,
                    FileClipboardKind::Copy,
                    window,
                    cx,
                );
            }
            FileListPanelEvent::CutEntries => {
                self.store_file_clipboard(
                    ClipboardEndpoint::RemoteLeft,
                    FileClipboardKind::Cut,
                    window,
                    cx,
                );
            }
            FileListPanelEvent::PasteInto { target_dir } => {
                self.paste_file_clipboard(
                    ClipboardEndpoint::RemoteLeft,
                    target_dir.clone(),
                    window,
                    cx,
                );
            }
            FileListPanelEvent::Properties { item, full_path } => {
                self.show_file_properties(item.clone(), full_path.clone(), window, cx);
            }
            FileListPanelEvent::CalculateSize { full_path } => {
                self.calculate_size_for_endpoint(
                    ClipboardEndpoint::RemoteLeft,
                    full_path.clone(),
                    window,
                    cx,
                );
            }
            FileListPanelEvent::Download { .. } => {
                self.transfer_left_selection_to_right(window, cx);
            }
            FileListPanelEvent::Refresh => self.refresh_left_remote_dir(cx),
            FileListPanelEvent::ToggleHiddenFiles => {
                self.local_panel
                    .update(cx, |panel, cx| panel.toggle_show_hidden(cx));
            }
            FileListPanelEvent::CopyFileName { name } => {
                self.copy_file_name(name, window, cx);
            }
            FileListPanelEvent::CopyAbsolutePath { full_path } => {
                self.copy_absolute_path(full_path, window, cx);
            }
            FileListPanelEvent::OpenInTerminal => {
                self.open_left_remote_terminal(None, cx);
            }
            FileListPanelEvent::OpenInTerminalAt { full_path } => {
                self.open_left_remote_terminal(Some(full_path.clone()), cx);
            }
            _ => {
                window.push_notification(
                    Notification::info(t!("Endpoint.remote_edit_pending").to_string())
                        .autohide(true),
                    cx,
                );
            }
        }
    }

    fn select_and_upload_files_to(
        &mut self,
        remote_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.sftp_client.clone() else {
            return;
        };

        let view = cx.entity().clone();
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            multiple: true,
            directories: false,
            prompt: Some(t!("FilePicker.select_upload_files").to_string().into()),
        });

        window
            .spawn(cx, async move |cx| {
                if let Ok(Ok(Some(paths))) = future.await {
                    if paths.is_empty() {
                        return;
                    }

                    let _ = view.update_in(cx, |this, window, cx| {
                        this.upload_paths_to_remote(paths, remote_path, client, window, cx);
                    });
                }
            })
            .detach();
    }

    fn select_and_upload_folder_to(
        &mut self,
        remote_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(client) = self.sftp_client.clone() else {
            return;
        };

        let view = cx.entity().clone();
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            multiple: true,
            directories: true,
            prompt: Some(t!("FilePicker.select_upload_folder").to_string().into()),
        });

        window
            .spawn(cx, async move |cx| {
                if let Ok(Ok(Some(paths))) = future.await {
                    if paths.is_empty() {
                        return;
                    }

                    let _ = view.update_in(cx, |this, window, cx| {
                        this.upload_paths_to_remote(paths, remote_path, client, window, cx);
                    });
                }
            })
            .detach();
    }
}

fn is_valid_entry_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

/// 右键菜单处理 trait
/// 为 SftpView 实现右键菜单的各种功能
pub trait ContextMenuHandler {
    /// 处理本地面板的右键菜单事件
    fn handle_local_context_menu_event(
        &mut self,
        event: &FileListPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        Self: Sized;

    /// 处理远程面板的右键菜单事件
    fn handle_remote_context_menu_event(
        &mut self,
        event: &FileListPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        Self: Sized;

    /// 新建文件
    fn create_new_file(&mut self, side: PanelSide, window: &mut Window, cx: &mut Context<Self>)
    where
        Self: Sized;

    /// 重命名文件/文件夹
    fn rename_item(
        &mut self,
        name: &str,
        full_path: &str,
        side: PanelSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        Self: Sized;

    /// 复制文件名到剪贴板
    fn copy_file_name(&self, name: &str, window: &mut Window, cx: &mut Context<Self>)
    where
        Self: Sized;

    /// 复制绝对路径到剪贴板
    fn copy_absolute_path(&self, path: &str, window: &mut Window, cx: &mut Context<Self>)
    where
        Self: Sized;

    /// 修改权限
    fn change_permissions(
        &mut self,
        name: &str,
        full_path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        Self: Sized;

    /// 在终端中打开（当前目录）
    fn open_in_terminal(&self, side: PanelSide, window: &mut Window, cx: &mut Context<Self>)
    where
        Self: Sized;

    /// 在终端中打开（指定路径）
    fn open_in_terminal_at(
        &self,
        path: &str,
        side: PanelSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        Self: Sized;

    /// 切换隐藏文件显示
    fn toggle_hidden_files(&mut self, side: PanelSide, cx: &mut Context<Self>)
    where
        Self: Sized;

    /// 选择本地文件并上传到远程
    fn select_and_upload_files(&mut self, window: &mut Window, cx: &mut Context<Self>)
    where
        Self: Sized;

    /// 选择本地文件夹并上传到远程
    fn select_and_upload_folder(&mut self, window: &mut Window, cx: &mut Context<Self>)
    where
        Self: Sized;

    fn extract_archive(
        &mut self,
        name: String,
        _full_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        Self: Sized;

    fn show_extract_conflict_dialog(
        &mut self,
        name: String,
        _full_path: String,
        overwrite_command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        Self: Sized;

    fn start_extract_archive(
        &mut self,
        name: String,
        _full_path: String,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        Self: Sized;
}

impl ContextMenuHandler for SftpView {
    fn handle_local_context_menu_event(
        &mut self,
        event: &FileListPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.left_remote.is_some() {
            self.handle_left_remote_context_menu_event(event, window, cx);
            return;
        }
        match event {
            FileListPanelEvent::CopyEntries => {
                self.store_file_clipboard(
                    ClipboardEndpoint::Local,
                    FileClipboardKind::Copy,
                    window,
                    cx,
                );
            }
            FileListPanelEvent::CutEntries => {
                self.store_file_clipboard(
                    ClipboardEndpoint::Local,
                    FileClipboardKind::Cut,
                    window,
                    cx,
                );
            }
            FileListPanelEvent::PasteInto { target_dir } => {
                self.paste_file_clipboard(ClipboardEndpoint::Local, target_dir.clone(), window, cx);
            }
            FileListPanelEvent::Properties { item, full_path } => {
                self.show_file_properties(item.clone(), full_path.clone(), window, cx);
            }
            FileListPanelEvent::CalculateSize { full_path } => {
                self.calculate_size_for_endpoint(
                    ClipboardEndpoint::Local,
                    full_path.clone(),
                    window,
                    cx,
                );
            }
            FileListPanelEvent::NewFile => {
                self.create_new_file(PanelSide::Local, window, cx);
            }
            FileListPanelEvent::NewFolder => {
                self.show_new_folder_dialog(PanelSide::Local, window, cx);
            }
            FileListPanelEvent::Rename { name, full_path } => {
                self.rename_item(name, full_path, PanelSide::Local, window, cx);
            }
            FileListPanelEvent::Download {
                name: _,
                full_path: _,
            } => {
                if self.left_remote.is_some() {
                    self.transfer_left_selection_to_right(window, cx);
                }
            }
            FileListPanelEvent::ChangePermissions { name, full_path } => {
                self.change_permissions(name, full_path, window, cx);
            }
            FileListPanelEvent::OpenInTerminal => {
                self.open_in_terminal(PanelSide::Local, window, cx);
            }
            FileListPanelEvent::OpenInTerminalAt { full_path } => {
                self.open_in_terminal_at(full_path, PanelSide::Local, window, cx);
            }
            FileListPanelEvent::CopyFileName { name } => {
                self.copy_file_name(name, window, cx);
            }
            FileListPanelEvent::CopyAbsolutePath { full_path } => {
                self.copy_absolute_path(full_path, window, cx);
            }
            FileListPanelEvent::Delete {
                name: _,
                full_path: _,
            } => {
                self.delete_local_selected(window, cx);
            }
            FileListPanelEvent::FavoritePath { full_path } => {
                self.add_local_favorite_path(full_path, window, cx);
            }
            FileListPanelEvent::UploadFile => {
                self.upload_selected(window, cx);
            }
            FileListPanelEvent::UploadFolder => {
                self.upload_selected(window, cx);
            }
            FileListPanelEvent::Refresh => {
                self.refresh_local_dir_with_window(window, cx);
            }
            FileListPanelEvent::ToggleHiddenFiles => {
                self.toggle_hidden_files(PanelSide::Local, cx);
            }
            _ => {}
        }
    }

    fn handle_remote_context_menu_event(
        &mut self,
        event: &FileListPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            FileListPanelEvent::CopyEntries => {
                self.store_file_clipboard(
                    ClipboardEndpoint::RemoteRight,
                    FileClipboardKind::Copy,
                    window,
                    cx,
                );
            }
            FileListPanelEvent::CutEntries => {
                self.store_file_clipboard(
                    ClipboardEndpoint::RemoteRight,
                    FileClipboardKind::Cut,
                    window,
                    cx,
                );
            }
            FileListPanelEvent::PasteInto { target_dir } => {
                self.paste_file_clipboard(
                    ClipboardEndpoint::RemoteRight,
                    target_dir.clone(),
                    window,
                    cx,
                );
            }
            FileListPanelEvent::Properties { item, full_path } => {
                self.show_file_properties(item.clone(), full_path.clone(), window, cx);
            }
            FileListPanelEvent::CalculateSize { full_path } => {
                self.calculate_size_for_endpoint(
                    ClipboardEndpoint::RemoteRight,
                    full_path.clone(),
                    window,
                    cx,
                );
            }
            FileListPanelEvent::NewFile => {
                self.create_new_file(PanelSide::Remote, window, cx);
            }
            FileListPanelEvent::NewFolder => {
                self.show_new_folder_dialog(PanelSide::Remote, window, cx);
            }
            FileListPanelEvent::Rename { name, full_path } => {
                self.rename_item(name, full_path, PanelSide::Remote, window, cx);
            }
            FileListPanelEvent::Download {
                name: _,
                full_path: _,
            } => {
                if self.left_remote.is_some() {
                    self.transfer_right_selection_to_left(window, cx);
                } else {
                    self.download_selected(window, cx);
                }
            }
            FileListPanelEvent::Edit { full_path } => {
                self.open_remote_file(full_path.clone(), window, cx);
            }
            FileListPanelEvent::EditExternal {
                full_path,
                editor_key,
            } => {
                self.open_remote_external_editor(
                    (full_path.clone(), editor_key.clone()),
                    window,
                    cx,
                );
            }
            FileListPanelEvent::Extract { name, full_path } => {
                self.extract_archive(name.clone(), full_path.clone(), window, cx);
            }
            FileListPanelEvent::ChangePermissions { name, full_path } => {
                self.change_permissions(name, full_path, window, cx);
            }
            FileListPanelEvent::OpenInTerminal => {
                self.open_in_terminal(PanelSide::Remote, window, cx);
            }
            FileListPanelEvent::OpenInTerminalAt { full_path } => {
                self.open_in_terminal_at(full_path, PanelSide::Remote, window, cx);
            }
            FileListPanelEvent::CopyFileName { name } => {
                self.copy_file_name(name, window, cx);
            }
            FileListPanelEvent::CopyAbsolutePath { full_path } => {
                self.copy_absolute_path(full_path, window, cx);
            }
            FileListPanelEvent::Delete {
                name: _,
                full_path: _,
            } => {
                self.delete_remote_selected(window, cx);
            }
            FileListPanelEvent::FavoritePath { full_path } => {
                self.add_remote_favorite_path(full_path, window, cx);
            }
            FileListPanelEvent::UploadFile => {
                self.select_and_upload_files(window, cx);
            }
            FileListPanelEvent::UploadFolder => {
                self.select_and_upload_folder(window, cx);
            }
            FileListPanelEvent::UploadFileTo { full_path } => {
                self.select_and_upload_files_to(full_path.clone(), window, cx);
            }
            FileListPanelEvent::UploadFolderTo { full_path } => {
                self.select_and_upload_folder_to(full_path.clone(), window, cx);
            }
            FileListPanelEvent::Refresh => {
                self.refresh_remote_dir_with_window(window, cx);
            }
            FileListPanelEvent::ToggleHiddenFiles => {
                self.toggle_hidden_files(PanelSide::Remote, cx);
            }
            _ => {}
        }
    }

    fn create_new_file(&mut self, side: PanelSide, window: &mut Window, cx: &mut Context<Self>) {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Placeholder.filename")));
        let view = cx.entity().downgrade();

        // 在打开对话框前设置焦点，避免闪烁
        input.update(cx, |state, cx| {
            state.focus(window, cx);
        });

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let side = side;
            let view_clone = view.clone();
            let input_for_callback = input.clone();

            dialog
                .title(t!("File.new_file").to_string())
                .w(gpui::px(360.))
                .child(Input::new(&input))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("Common.create").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let file_name = input_for_callback.read(cx).text().to_string();
                    if file_name.is_empty() {
                        return false;
                    }
                    if !is_valid_entry_name(&file_name) {
                        window.push_notification(Notification::error(t!("Error.invalid_name")), cx);
                        return false;
                    }

                    let _ = view_clone.update(cx, |this, cx| {
                        match side {
                            PanelSide::Local => {
                                let path = this.local_current_path.join(&file_name);
                                if let Err(e) = std::fs::File::create(&path) {
                                    tracing::error!(
                                        "Failed to create file {}: {}",
                                        path.display(),
                                        e
                                    );
                                    window.push_notification(
                                        Notification::error(t!(
                                            "Error.create_file_failed",
                                            error = e
                                        )),
                                        cx,
                                    );
                                } else {
                                    window.close_dialog(cx);
                                }
                                this.refresh_local_dir(cx);
                            }
                            PanelSide::Remote => {
                                let Some(client) = this.sftp_client.clone() else {
                                    return;
                                };

                                let remote_path =
                                    join_remote_path(&this.remote_current_path, &file_name);

                                let task = Tokio::spawn(cx, async move {
                                    let mut client = client.lock().await;
                                    // 创建空文件
                                    client.write_file(&remote_path, &[]).await
                                });

                                let view = cx.entity().clone();
                                window
                                    .spawn(cx, async move |cx| match task.await {
                                        Ok(Ok(_)) => {
                                            let _ = view.update_in(cx, |this, window, cx| {
                                                window.close_dialog(cx);
                                                this.refresh_remote_dir(cx);
                                            });
                                        }
                                        Ok(Err(e)) => {
                                            tracing::error!("Failed to create remote file: {}", e);
                                            let _ = view.update_in(cx, |_this, window, cx| {
                                                window.push_notification(
                                                    Notification::error(t!(
                                                        "Error.create_file_failed",
                                                        error = e
                                                    )),
                                                    cx,
                                                );
                                            });
                                        }
                                        Err(e) => {
                                            tracing::error!("Task error: {}", e);
                                            let _ = view.update_in(cx, |_this, window, cx| {
                                                window.push_notification(
                                                    Notification::error(t!(
                                                        "Error.create_file_failed",
                                                        error = e
                                                    )),
                                                    cx,
                                                );
                                            });
                                        }
                                    })
                                    .detach();
                            }
                        }
                    });
                    false
                })
        });
    }

    fn rename_item(
        &mut self,
        name: &str,
        full_path: &str,
        side: PanelSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Placeholder.new_name")));
        let view = cx.entity().downgrade();
        let old_name = name.to_string();
        let old_path = full_path.to_string();

        // 设置初始值为当前文件名
        input.update(cx, |state, cx| {
            state.set_value(&old_name, window, cx);
        });

        // 在打开对话框前设置焦点，避免闪烁
        input.update(cx, |state, cx| {
            state.focus(window, cx);
        });

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let side = side;
            let view_clone = view.clone();
            let input_for_callback = input.clone();
            let old_path_for_callback = old_path.clone();

            dialog
                .title(t!("Common.rename").to_string())
                .w(gpui::px(360.))
                .child(Input::new(&input))
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("Common.rename").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let new_name = input_for_callback.read(cx).text().to_string();
                    if new_name.is_empty() {
                        return false;
                    }
                    if !is_valid_entry_name(&new_name) {
                        window.push_notification(Notification::error(t!("Error.invalid_name")), cx);
                        return false;
                    }

                    let _ = view_clone.update(cx, |this, cx| match side {
                        PanelSide::Local => {
                            let old_full_path = PathBuf::from(&old_path_for_callback);
                            let new_full_path = old_full_path
                                .parent()
                                .unwrap_or(&old_full_path)
                                .join(&new_name);

                            if let Err(e) = std::fs::rename(&old_full_path, &new_full_path) {
                                tracing::error!(
                                    "Failed to rename {} to {}: {}",
                                    old_full_path.display(),
                                    new_full_path.display(),
                                    e
                                );
                                window.push_notification(
                                    Notification::error(t!("Error.rename_failed", error = e)),
                                    cx,
                                );
                            } else {
                                window.close_dialog(cx);
                            }
                            this.refresh_local_dir(cx);
                        }
                        PanelSide::Remote => {
                            let Some(client) = this.sftp_client.clone() else {
                                return;
                            };

                            let old_remote_path = old_path_for_callback.clone();
                            let new_remote_path = if let Some(pos) = old_remote_path.rfind('/') {
                                format!("{}/{}", &old_remote_path[..pos], new_name)
                            } else {
                                new_name.clone()
                            };

                            let task = Tokio::spawn(cx, async move {
                                let mut client = client.lock().await;
                                client.rename(&old_remote_path, &new_remote_path).await
                            });

                            let view = cx.entity().clone();
                            window
                                .spawn(cx, async move |cx| match task.await {
                                    Ok(Ok(_)) => {
                                        let _ = view.update_in(cx, |this, window, cx| {
                                            window.close_dialog(cx);
                                            this.refresh_remote_dir(cx);
                                        });
                                    }
                                    Ok(Err(e)) => {
                                        tracing::error!("Failed to rename remote file: {}", e);
                                        let _ = view.update_in(cx, |_this, window, cx| {
                                            window.push_notification(
                                                Notification::error(t!(
                                                    "Error.rename_failed",
                                                    error = e
                                                )),
                                                cx,
                                            );
                                        });
                                    }
                                    Err(e) => {
                                        tracing::error!("Task error: {}", e);
                                        let _ = view.update_in(cx, |_this, window, cx| {
                                            window.push_notification(
                                                Notification::error(t!(
                                                    "Error.rename_failed",
                                                    error = e
                                                )),
                                                cx,
                                            );
                                        });
                                    }
                                })
                                .detach();
                        }
                    });
                    false
                })
        });
    }

    fn copy_file_name(&self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(name.to_string()));
        window.push_notification(
            Notification::success(t!("Notification.copied_filename")),
            cx,
        );
    }

    fn copy_absolute_path(&self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(path.to_string()));
        window.push_notification(Notification::success(t!("Notification.copied_path")), cx);
    }

    fn change_permissions(
        &mut self,
        name: &str,
        full_path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Placeholder.permission")));
        let view = cx.entity().downgrade();
        let file_name = name.to_string();
        let file_path = full_path.to_string();

        // 在打开对话框前设置焦点，避免闪烁
        input.update(cx, |state, cx| {
            state.focus(window, cx);
        });

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_clone = view.clone();
            let input_for_callback = input.clone();
            let path_for_callback = file_path.clone();

            dialog
                .title(t!("Dialog.change_permission_title", name = file_name).to_string())
                .w(gpui::px(360.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(t!("Notification.permission_hint").to_string())
                        .child(Input::new(&input)),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("Common.modify").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_, window, cx: &mut App| {
                    let perm_str = input_for_callback.read(cx).text().to_string();
                    if perm_str.is_empty() {
                        return false;
                    }

                    // 解析八进制权限值
                    let mode = match u32::from_str_radix(&perm_str, 8) {
                        Ok(m) if m <= 0o777 => m,
                        _ => {
                            window.push_notification(
                                Notification::error(t!("Notification.invalid_permission")),
                                cx,
                            );
                            return false;
                        }
                    };

                    let _ = view_clone.update(cx, |this, cx| {
                        let Some(client) = this.sftp_client.clone() else {
                            return;
                        };

                        let remote_path = path_for_callback.clone();
                        let task = Tokio::spawn(cx, async move {
                            let mut client = client.lock().await;
                            client.chmod(&remote_path, mode).await
                        });

                        let view = cx.entity().clone();
                        window
                            .spawn(cx, async move |cx| match task.await {
                                Ok(Ok(_)) => {
                                    let _ = view.update_in(cx, |this, window, cx| {
                                        window.close_dialog(cx);
                                        window.push_notification(
                                            Notification::success(t!(
                                                "Notification.permission_success"
                                            )),
                                            cx,
                                        );
                                        this.refresh_remote_dir(cx);
                                    });
                                }
                                Ok(Err(e)) => {
                                    tracing::error!("Failed to change permissions: {}", e);
                                    let _ = view.update_in(cx, |_this, window, cx| {
                                        window.push_notification(
                                            Notification::error(t!(
                                                "Error.permission_failed",
                                                error = e
                                            )),
                                            cx,
                                        );
                                    });
                                }
                                Err(e) => {
                                    tracing::error!("Task error: {}", e);
                                }
                            })
                            .detach();
                    });
                    false
                })
        });
    }

    fn open_in_terminal(&self, side: PanelSide, _window: &mut Window, cx: &mut Context<Self>) {
        match side {
            PanelSide::Local => {
                let path = self.local_current_path.to_string_lossy().to_string();
                cx.emit(SftpViewEvent::OpenLocalTerminal { working_dir: path });
            }
            PanelSide::Remote => {
                // 打开 SSH 终端连接到远程服务器
                cx.emit(SftpViewEvent::OpenSshTerminal {
                    connection: self.stored_connection.clone(),
                    working_dir: self.remote_current_path.to_string(),
                });
            }
        }
    }

    fn open_in_terminal_at(
        &self,
        path: &str,
        side: PanelSide,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match side {
            PanelSide::Local => {
                // 如果是文件，获取其所在目录
                let target_path = PathBuf::from(path);
                let target_path = if target_path.is_absolute() {
                    target_path
                } else {
                    self.local_current_path.join(path)
                };
                let dir_path = if target_path.is_file() {
                    target_path.parent().unwrap_or(&target_path).to_path_buf()
                } else {
                    target_path
                };
                let path_str = dir_path.to_string_lossy().to_string();
                cx.emit(SftpViewEvent::OpenLocalTerminal {
                    working_dir: path_str,
                });
            }
            PanelSide::Remote => {
                // 打开 SSH 终端连接到远程服务器
                let base_path = self.remote_current_path.as_str();
                let is_rooted = path.starts_with('/')
                    || path.starts_with("~")
                    || path.starts_with("./")
                    || path.starts_with("../");
                let has_base_prefix = !base_path.is_empty()
                    && (path == base_path || path.starts_with(&format!("{}/", base_path)));
                let working_dir = if is_rooted || has_base_prefix {
                    path.to_string()
                } else {
                    join_remote_path(base_path, path)
                };
                cx.emit(SftpViewEvent::OpenSshTerminal {
                    connection: self.stored_connection.clone(),
                    working_dir,
                });
            }
        }
    }

    fn toggle_hidden_files(&mut self, side: PanelSide, cx: &mut Context<Self>) {
        match side {
            PanelSide::Local => {
                self.local_panel.update(cx, |panel, cx| {
                    panel.toggle_show_hidden(cx);
                });
            }
            PanelSide::Remote => {
                self.remote_panel.update(cx, |panel, cx| {
                    panel.toggle_show_hidden(cx);
                });
            }
        }
    }

    fn select_and_upload_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.select_and_upload_files_to(self.remote_current_path.clone(), window, cx);
    }

    fn select_and_upload_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.select_and_upload_folder_to(self.remote_current_path.clone(), window, cx);
    }

    fn extract_archive(
        &mut self,
        name: String,
        full_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_extract.is_some() {
            window.push_notification(Notification::info(t!("Extract.running")), cx);
            return;
        }

        let Some(command) =
            build_remote_extract_command(&full_path, &name, ExtractConflictAction::Overwrite)
        else {
            window.push_notification(Notification::error(t!("Error.extract_unsupported")), cx);
            return;
        };

        let Some(check_command) = build_remote_extract_conflict_check_command(&full_path, &name)
        else {
            window.push_notification(Notification::error(t!("Error.extract_unsupported")), cx);
            return;
        };

        let session_manager = Arc::new(SshSessionManager::new(self.sftp_config.clone()));
        let view = cx.entity().clone();
        let task = Tokio::spawn(cx, async move {
            remote_extract_has_conflict(session_manager, &check_command).await
        });

        window
            .spawn(cx, async move |cx| match task.await {
                Ok(Ok(true)) => {
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.show_extract_conflict_dialog(name, full_path, command, window, cx);
                    });
                }
                Ok(Ok(false)) => {
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.start_extract_archive(name, full_path, command, window, cx);
                    });
                }
                Ok(Err(error)) => {
                    let message = t!("Error.extract_check_failed", error = error).to_string();
                    let _ = view.update_in(cx, |_this, window, cx| {
                        window.push_notification(Notification::error(message), cx);
                    });
                }
                Err(error) => {
                    let message = t!("Error.extract_check_failed", error = error).to_string();
                    let _ = view.update_in(cx, |_this, window, cx| {
                        window.push_notification(Notification::error(message), cx);
                    });
                }
            })
            .detach();
    }

    fn show_extract_conflict_dialog(
        &mut self,
        name: String,
        full_path: String,
        overwrite_command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(skip_command) =
            build_remote_extract_command(&full_path, &name, ExtractConflictAction::SkipExisting)
        else {
            window.push_notification(Notification::error(t!("Error.extract_unsupported")), cx);
            return;
        };

        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_skip = view.clone();
            let view_overwrite = view.clone();
            let skip_name = name.clone();
            let skip_path = full_path.clone();
            let overwrite_name = name.clone();
            let overwrite_path = full_path.clone();
            let skip_command = skip_command.clone();
            let overwrite_command = overwrite_command.clone();

            dialog
                .title(t!("Extract.conflict_title").to_string())
                .w(px(380.))
                .child(
                    div()
                        .text_sm()
                        .child(t!("Extract.conflict_message", name = name.clone())),
                )
                .child(
                    gpui_component::h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("extract-cancel")
                                .label(t!("Common.cancel").to_string())
                                .ghost()
                                .on_click(|_, window, cx| {
                                    window.close_dialog(cx);
                                }),
                        )
                        .child(
                            Button::new("extract-skip-existing")
                                .label(t!("Extract.skip_existing").to_string())
                                .ghost()
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = view_skip.update(cx, |this, cx| {
                                        this.start_extract_archive(
                                            skip_name.clone(),
                                            skip_path.clone(),
                                            skip_command.clone(),
                                            window,
                                            cx,
                                        );
                                    });
                                }),
                        )
                        .child(
                            Button::new("extract-overwrite")
                                .label(t!("Conflict.overwrite").to_string())
                                .primary()
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = view_overwrite.update(cx, |this, cx| {
                                        this.start_extract_archive(
                                            overwrite_name.clone(),
                                            overwrite_path.clone(),
                                            overwrite_command.clone(),
                                            window,
                                            cx,
                                        );
                                    });
                                }),
                        ),
                )
        });
    }

    fn start_extract_archive(
        &mut self,
        name: String,
        _full_path: String,
        command: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_extract.is_some() {
            window.push_notification(Notification::info(t!("Extract.running")), cx);
            return;
        }

        let background_task = self.register_non_cancellable_background_task(
            "sftp-extract",
            format!("{} · {name}", t!("Extract.running")),
            cx,
        );
        self.active_extract = Some(ActiveExtract { background_task });
        cx.notify();

        let session_manager = Arc::new(SshSessionManager::new(self.sftp_config.clone()));
        let view = cx.entity().clone();
        let task = Tokio::spawn(cx, async move {
            exec_remote_command(session_manager, &command).await
        });

        window
            .spawn(cx, async move |cx| match task.await {
                Ok(Ok(_)) => {
                    let _ = view.update_in(cx, |this, window, cx| {
                        if let Some(extract) = this.active_extract.take() {
                            extract.background_task.succeed(None, cx);
                        }
                        window.push_notification(
                            Notification::success(t!("Notification.extract_success")),
                            cx,
                        );
                        this.refresh_remote_dir(cx);
                    });
                }
                Ok(Err(error)) => {
                    let message = t!("Error.extract_failed", error = error).to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        if let Some(extract) = this.active_extract.take() {
                            extract.background_task.fail(message.clone(), cx);
                        }
                        window.push_notification(Notification::error(message), cx);
                    });
                }
                Err(error) => {
                    let message = t!("Error.extract_failed", error = error).to_string();
                    let _ = view.update_in(cx, |this, window, cx| {
                        if let Some(extract) = this.active_extract.take() {
                            extract.background_task.fail(message.clone(), cx);
                        }
                        window.push_notification(Notification::error(message), cx);
                    });
                }
            })
            .detach();
    }
}
