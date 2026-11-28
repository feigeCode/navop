use super::*;

impl TerminalView {
    pub(super) fn paste_clipboard_image_to_remote_cli(
        &mut self,
        image: Image,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_live_ssh_terminal(cx) {
            return;
        }
        let Some(ssh_config) = self
            .terminal
            .read(cx)
            .ssh_config()
            .map(|config| config.ssh_config.clone())
        else {
            window.push_notification(
                Notification::error(t!("TerminalView.clipboard_image_requires_ssh").to_string())
                    .autohide(true),
                cx,
            );
            return;
        };

        if image.bytes.is_empty() {
            window.push_notification(
                Notification::error(t!("TerminalView.clipboard_image_empty").to_string())
                    .autohide(true),
                cx,
            );
            return;
        }

        self.spawn_clipboard_image_upload(ssh_config, image, window, cx);
        window.push_notification(
            Notification::info(t!("TerminalView.clipboard_image_uploading").to_string())
                .autohide(true),
            cx,
        );
    }

    /// 本地终端没有可上传的远端，改为把剪贴板图片落到临时目录并粘贴本地路径，
    /// 这样 `nvim`、`xdg-open` 这类本地命令能直接拿到文件（#529）。
    pub(super) fn paste_clipboard_image_to_local_cli(
        &mut self,
        image: Image,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if image.bytes.is_empty() {
            window.push_notification(
                Notification::error(t!("TerminalView.clipboard_image_empty").to_string())
                    .autohide(true),
                cx,
            );
            return;
        }

        // macOS 剪贴板图片常是 TIFF，本地编辑器未必认识；统一转成 PNG 再落盘。
        let image = remote_image_preview::normalize_clipboard_image(image);
        let directory = std::env::temp_dir().join(LOCAL_PASTE_IMAGE_DIR);
        let path = local_clipboard_image_path(&directory, image.format, current_timestamp_millis());
        let write_result =
            std::fs::create_dir_all(&directory).and_then(|_| std::fs::write(&path, &image.bytes));

        match write_result {
            Ok(()) => {
                let path = path.to_string_lossy().to_string();
                self.paste_text(&path, window, cx);
                window.push_notification(
                    Notification::info(
                        t!("TerminalView.clipboard_image_saved_locally", path = path).to_string(),
                    )
                    .autohide(true),
                    cx,
                );
            }
            Err(error) => {
                window.push_notification(
                    Notification::error(
                        t!(
                            "TerminalView.clipboard_image_save_local_failed",
                            error = error.to_string()
                        )
                        .to_string(),
                    )
                    .autohide(true),
                    cx,
                );
            }
        }
    }

    pub(super) fn spawn_clipboard_image_upload(
        &mut self,
        ssh_config: ssh::SshConnectConfig,
        image: Image,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_live_ssh_terminal(cx) {
            return;
        }
        let window_handle = window.window_handle();
        let task = Tokio::spawn(cx, async move {
            // macOS 剪贴板图片是 TIFF，远端 CLI 通常读不了；先统一转成 PNG
            // 再决定远端文件名，保证扩展名与实际字节一致。
            let image = remote_image_preview::normalize_clipboard_image(image);
            let remote_path = remote_clipboard_image_path(image.format, current_timestamp_millis());
            let mut client = RusshSftpClient::connect(ssh_config).await?;
            client.write_file(&remote_path, &image.bytes).await?;
            Ok::<_, anyhow::Error>(remote_path)
        });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.handle_clipboard_image_upload_result(result, window_handle, cx);
            });
        })
        .detach();
    }

    pub(super) fn handle_clipboard_image_upload_result(
        &mut self,
        result: Result<Result<String, anyhow::Error>, tokio::task::JoinError>,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(Ok(path)) => {
                self.paste_remote_image_path(&path, cx);
                self.notify_clipboard_image_upload(
                    window_handle,
                    Notification::success(
                        t!("TerminalView.clipboard_image_uploaded", path = path).to_string(),
                    ),
                    cx,
                );
            }
            Ok(Err(error)) => self.notify_clipboard_image_upload(
                window_handle,
                Notification::error(
                    t!("TerminalView.clipboard_image_upload_failed", error = error).to_string(),
                ),
                cx,
            ),
            Err(error) => self.notify_clipboard_image_upload(
                window_handle,
                Notification::error(
                    t!("TerminalView.clipboard_image_task_failed", error = error).to_string(),
                ),
                cx,
            ),
        }
    }

    pub(super) fn notify_clipboard_image_upload(
        &self,
        window_handle: AnyWindowHandle,
        notification: Notification,
        cx: &mut Context<Self>,
    ) {
        let _ = cx.update_window(window_handle, |_, window, cx| {
            window.push_notification(notification.autohide(true), cx);
        });
    }

    pub(super) fn paste_remote_image_path(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.is_live_ssh_terminal(cx) {
            return;
        }
        // 图片路径粘贴同样读实时模式，避免 2004 过期包装（issue #198）。
        let mode = self.live_term_mode(cx);
        self.apply_paste_to_history_prompt(path, cx);
        self.write_to_pty(terminal_paste_bytes(path, mode), cx);
    }

    /// 粘贴代码块到终端（用于AI生成的代码）
    ///
    /// 内部调用 paste_text，保持统一的粘贴行为
    pub(super) fn paste_code_block(
        &mut self,
        code: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.paste_text(code, window, cx);
    }

    pub(super) fn paste_preview_text(text: &str) -> String {
        let preview = text
            .lines()
            .take(PASTE_PREVIEW_MAX_LINES)
            .collect::<Vec<_>>()
            .join("\n");
        if text.lines().count() > PASTE_PREVIEW_MAX_LINES {
            format!("{preview}\n...")
        } else {
            preview
        }
    }

    pub(super) fn paste_summary_text(text: &str) -> String {
        t!(
            "TerminalView.paste_summary",
            lines = multiline_non_empty_line_count(text),
            chars = text.chars().count()
        )
        .to_string()
    }
}

/// 弹窗预览最大行数（超出部分显示省略号）。
const PASTE_PREVIEW_MAX_LINES: usize = 6;

/// 多行粘贴合并为单行：换行折叠为空格，连续空白压成一个。
pub(super) fn join_paste_as_single_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
