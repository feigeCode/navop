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
        let remote_path = remote_clipboard_image_path(image.format, current_timestamp_millis());
        let bytes = image.bytes;
        let window_handle = window.window_handle();
        let task = Tokio::spawn(cx, async move {
            let mut client = RusshSftpClient::connect(ssh_config).await?;
            client.write_file(&remote_path, &bytes).await?;
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
        let mode = self.terminal_frame_snapshot.mode;
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
        let preview = text.lines().take(PASTE_PREVIEW_MAX_LINES).collect::<Vec<_>>().join("\n");
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

    pub(super) fn show_paste_confirm_dialog(
        &mut self,
        text: String,
        title: String,
        message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preview_text = Self::paste_preview_text(&text);
        let summary_text = Self::paste_summary_text(&text);
        let single_line_available = multiline_non_empty_line_count(&text) > 1;
        let view = cx.entity().clone();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let view_ok = view.clone();
            let text_ok = text.clone();
            let view_single = view.clone();
            let text_single = text.clone();

            dialog
                .title(title.clone())
                .child(
                    v_flex()
                        .gap_2()
                        .min_h_0()
                        .child(div().text_sm().child(message.clone()))
                        .child(
                            h_flex()
                                .justify_between()
                                .child(div().text_xs().child(t!("TerminalView.paste_preview")))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(_cx.theme().muted_foreground)
                                        .child(summary_text.clone()),
                                ),
                        )
                        .child(
                            v_flex()
                                .id("paste-preview")
                                .max_h(px(160.0))
                                .overflow_y_scroll()
                                .text_xs()
                                .child(preview_text.clone()),
                        )
                        .when(single_line_available, |this| {
                            this.child(
                                h_flex().gap_1().child(
                                    Button::new("paste-single-line")
                                        .label(t!("TerminalView.paste_as_single_line"))
                                        .small()
                                        .outline()
                                        .on_click(move |_event, window, cx| {
                                            let joined = join_paste_as_single_line(&text_single);
                                            window.close_dialog(cx);
                                            view_single.update(cx, |this, cx| {
                                                this.paste_text_unchecked(&joined, window, cx);
                                            });
                                        }),
                                ),
                            )
                        })
                        .into_any_element(),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .ok_text(t!("Common.ok"))
                        .cancel_text(t!("Common.cancel")),
                )
                .on_ok(move |_event, window, cx| {
                    view_ok.update(cx, |this, cx| {
                        this.paste_text_unchecked(&text_ok, window, cx);
                    });
                    true
                })
        });
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
