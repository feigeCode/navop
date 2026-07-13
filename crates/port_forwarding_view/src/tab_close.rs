use std::sync::{Arc, Mutex};

use gpui::{Context, IntoElement, ParentElement, Task, Window};
use gpui_component::WindowExt;
use gpui_component::button::{Button, ButtonVariants};
use rust_i18n::t;
use tokio::sync::oneshot;

use crate::tab::PortForwardingTab;

pub(crate) fn try_close(
    tab: &mut PortForwardingTab,
    _tab_id: &str,
    window: &mut Window,
    cx: &mut Context<PortForwardingTab>,
) -> Task<bool> {
    if tab.state.can_close_without_prompt() {
        return Task::ready(true);
    }
    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let view = cx.entity();
    window.open_dialog(cx, move |dialog, _window, _cx| {
        let cancel = tx.clone();
        let confirm = tx.clone();
        let view = view.clone();
        dialog
            .title(t!("PortForwardingTab.close_title").to_string())
            .overlay_closable(false)
            .close_button(false)
            .footer(move |_, _, _, _| {
                vec![
                    cancel_button(cancel.clone()),
                    confirm_button(confirm.clone(), view.clone()),
                ]
            })
            .child(t!("PortForwardingTab.close_warning").to_string())
    });
    cx.spawn(async move |_this, _cx| rx.await.unwrap_or(false))
}

fn cancel_button(tx: Arc<Mutex<Option<oneshot::Sender<bool>>>>) -> gpui::AnyElement {
    Button::new("cancel-stop-forwarding")
        .label(t!("Common.cancel").to_string())
        .on_click(move |_, window, cx| {
            window.close_dialog(cx);
            if let Some(tx) = tx.lock().ok().and_then(|mut tx| tx.take()) {
                let _ = tx.send(false);
            }
        })
        .into_any_element()
}

fn confirm_button(
    tx: Arc<Mutex<Option<oneshot::Sender<bool>>>>,
    view: gpui::Entity<PortForwardingTab>,
) -> gpui::AnyElement {
    Button::new("confirm-stop-forwarding")
        .label(t!("PortForwardingTab.stop_and_close").to_string())
        .danger()
        .on_click(move |_, window, cx| {
            window.close_dialog(cx);
            if let Some(tx) = tx.lock().ok().and_then(|mut tx| tx.take()) {
                view.update(cx, |tab, cx| tab.stop_for_close(Some(tx), cx));
            }
        })
        .into_any_element()
}
