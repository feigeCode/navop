use std::sync::{Arc, Mutex as StdMutex};

use gpui::{Context, ParentElement as _, Styled as _, Task, Window, px};
use gpui_component::dialog::DialogButtonProps;
use gpui_component::{WindowExt as _, v_flex};
use rust_i18n::t;

use super::TerminalWorkspace;

impl TerminalWorkspace {
    pub(super) fn close_all_task(&self, cx: &mut Context<Self>) -> Task<bool> {
        let panes = self.panes.values().cloned().collect::<Vec<_>>();
        cx.spawn(async move |_this, cx| {
            for pane in panes {
                pane.update(cx, |pane, cx| pane.close_now(cx));
            }
            true
        })
    }

    pub(super) fn confirm_close_all(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
        let tx = Arc::new(StdMutex::new(Some(tx)));
        let ok_tx = tx.clone();
        let cancel_tx = tx;
        let pane_count = self.panes.len();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let ok_tx = ok_tx.clone();
            let cancel_tx = cancel_tx.clone();
            dialog
                .title(t!("TerminalWorkspace.close_title").to_string())
                .w(px(440.0))
                .child(
                    v_flex().gap_2().child(
                        t!("TerminalWorkspace.close_message", count = pane_count).to_string(),
                    ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.close").to_string())
                        .cancel_text(t!("Common.cancel").to_string())
                        .show_cancel(true),
                )
                .on_ok(move |_, _, _| {
                    send_confirmation(&ok_tx, true);
                    true
                })
                .on_cancel(move |_, _, _| {
                    send_confirmation(&cancel_tx, false);
                    true
                })
                .overlay_closable(false)
                .close_button(false)
        });
        let panes = self.panes.values().cloned().collect::<Vec<_>>();
        cx.spawn(async move |_this, cx| {
            if !rx.await.unwrap_or(false) {
                return false;
            }
            for pane in panes {
                pane.update(cx, |pane, cx| pane.close_now(cx));
            }
            true
        })
    }
}

fn send_confirmation(
    sender: &Arc<StdMutex<Option<tokio::sync::oneshot::Sender<bool>>>>,
    confirmed: bool,
) {
    if let Ok(mut sender) = sender.lock() {
        if let Some(sender) = sender.take() {
            let _ = sender.send(confirmed);
        }
    }
}
