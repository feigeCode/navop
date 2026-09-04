use super::*;

impl TerminalView {
    pub(super) fn send_close_confirmation(
        sender: &Arc<StdMutex<Option<tokio::sync::oneshot::Sender<bool>>>>,
        confirmed: bool,
    ) {
        if let Ok(mut guard) = sender.lock() {
            if let Some(sender) = guard.take() {
                let _ = sender.send(confirmed);
            }
        }
    }

    pub(super) fn close_terminal_now(&mut self, cx: &mut Context<Self>) {
        self.unregister_broadcast_input(cx);
        self.unregister_public_mcp_session(cx);
        self.release_active_connection(cx);
        self.cancel_zmodem_background_tasks(cx);
        self.terminal.read(cx).shutdown();
    }

    pub(super) fn confirm_local_terminal_close(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let has_unsaved_workspace_files = self
            .workspace_editor
            .as_ref()
            .is_some_and(|editor| editor.read(cx).has_dirty_tabs(cx));
        let (title, message, warning) = if has_unsaved_workspace_files {
            (
                t!("LocalTerminalClose.workspace_unsaved_title").to_string(),
                t!("LocalTerminalClose.workspace_unsaved_message").to_string(),
                t!("LocalTerminalClose.workspace_unsaved_warning").to_string(),
            )
        } else {
            (
                t!("LocalTerminalClose.title").to_string(),
                t!("LocalTerminalClose.message").to_string(),
                t!("LocalTerminalClose.warning").to_string(),
            )
        };
        let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
        let tx = Arc::new(StdMutex::new(Some(tx)));
        let tx_ok = tx.clone();
        let tx_cancel = tx;

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let tx_ok = tx_ok.clone();
            let tx_cancel = tx_cancel.clone();
            dialog
                .title(title.clone())
                .w(px(420.))
                .child(
                    v_flex()
                        .gap_2()
                        .child(message.clone())
                        .child(warning.clone()),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.close").to_string())
                        .cancel_text(t!("Common.cancel").to_string())
                        .show_cancel(true),
                )
                .on_ok(move |_, _, _| {
                    TerminalView::send_close_confirmation(&tx_ok, true);
                    true
                })
                .on_cancel(move |_, _, _| {
                    TerminalView::send_close_confirmation(&tx_cancel, false);
                    true
                })
                .overlay_closable(false)
                .close_button(false)
        });

        cx.spawn(async move |this, cx| {
            let confirmed = rx.await.unwrap_or(false);
            if confirmed {
                let _ = this.update(cx, |this, cx| this.close_terminal_now(cx));
            }
            confirmed
        })
    }

    pub(super) fn release_active_connection(&self, cx: &mut Context<Self>) {
        let Some(connection_id) = self.terminal.read(cx).connection_id() else {
            return;
        };

        cx.global_mut::<ActiveConnections>().remove(connection_id);
    }
}
