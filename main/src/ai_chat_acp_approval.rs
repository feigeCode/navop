#[cfg(test)]
mod tests;

use gpui::{
    AnyElement, App, AppContext, AsyncApp, Global, IntoElement, ParentElement, Styled, div,
    prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Sizable, Size, WindowExt,
    button::{Button, ButtonVariants as _},
    scroll::ScrollableElement,
    v_flex,
};
use one_core::ai_chat::{AcpPermissionOutcome, AcpPermissionRequest, set_acp_permission_provider};
use rust_i18n::t;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

pub struct GlobalAcpApprovalQueue {
    _sender: mpsc::UnboundedSender<ApprovalEnvelope>,
}

impl Global for GlobalAcpApprovalQueue {}

pub fn init(cx: &mut App) {
    if cx.has_global::<GlobalAcpApprovalQueue>() {
        return;
    }
    let (sender, mut receiver) = mpsc::unbounded_channel();
    cx.set_global(GlobalAcpApprovalQueue {
        _sender: sender.clone(),
    });
    set_acp_permission_provider(cx, move |request| {
        let sender = sender.clone();
        Box::pin(async move { request_acp_approval(sender, request).await })
    });
    let queue = Arc::new(Mutex::new(ApprovalQueueState::default()));
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(envelope) = receiver.recv().await {
            present_approval_request(queue.clone(), envelope, cx);
        }
        Ok::<(), anyhow::Error>(())
    })
    .detach();
}

async fn request_acp_approval(
    sender: mpsc::UnboundedSender<ApprovalEnvelope>,
    request: AcpPermissionRequest,
) -> AcpPermissionOutcome {
    let (response_tx, response_rx) = oneshot::channel();
    if sender
        .send(ApprovalEnvelope {
            request,
            response_tx,
        })
        .is_err()
    {
        return AcpPermissionOutcome::Cancelled;
    }
    match tokio::time::timeout(APPROVAL_TIMEOUT, response_rx).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(_)) | Err(_) => AcpPermissionOutcome::Cancelled,
    }
}

struct ApprovalEnvelope {
    request: AcpPermissionRequest,
    response_tx: oneshot::Sender<AcpPermissionOutcome>,
}

impl ApprovalEnvelope {
    fn cancel(self) {
        self.resolve(AcpPermissionOutcome::Cancelled);
    }

    fn resolve(self, outcome: AcpPermissionOutcome) {
        let _ = self.response_tx.send(outcome);
    }
}

#[derive(Default)]
struct ApprovalQueueState {
    active: Option<ApprovalEnvelope>,
    pending: VecDeque<ApprovalEnvelope>,
    presenting: bool,
}

struct ApprovalQueueSnapshot {
    active: AcpPermissionRequest,
    pending_count: usize,
}

impl ApprovalQueueState {
    fn enqueue(&mut self, envelope: ApprovalEnvelope) {
        if self.active.is_none() {
            self.active = Some(envelope);
            return;
        }
        self.pending.push_back(envelope);
    }

    fn begin_presentation(&mut self) -> Option<ApprovalQueueSnapshot> {
        if self.presenting {
            return None;
        }
        let active = self.active.as_ref()?;
        self.presenting = true;
        Some(ApprovalQueueSnapshot {
            active: active.request.clone(),
            pending_count: self.pending.len(),
        })
    }

    fn resolve_active(&mut self, outcome: AcpPermissionOutcome) -> bool {
        if let Some(active) = self.active.take() {
            active.resolve(outcome);
        }
        self.active = self.pending.pop_front();
        self.presenting = false;
        self.active.is_some()
    }

    fn cancel_all(&mut self) {
        self.presenting = false;
        if let Some(active) = self.active.take() {
            active.cancel();
        }
        for pending in self.pending.drain(..) {
            pending.cancel();
        }
    }
}

fn present_approval_request(
    queue: Arc<Mutex<ApprovalQueueState>>,
    envelope: ApprovalEnvelope,
    cx: &mut AsyncApp,
) {
    queue
        .lock()
        .expect("ACP approval queue lock poisoned")
        .enqueue(envelope);
    let shown = cx.update(|cx| present_next_approval(queue.clone(), cx));
    if !shown {
        queue
            .lock()
            .expect("ACP approval queue lock poisoned")
            .cancel_all();
    }
}

fn present_next_approval(queue: Arc<Mutex<ApprovalQueueState>>, cx: &mut App) -> bool {
    let Some(window_id) = cx.active_window() else {
        return false;
    };
    let snapshot = queue
        .lock()
        .expect("ACP approval queue lock poisoned")
        .begin_presentation();
    let Some(snapshot) = snapshot else {
        return true;
    };
    cx.update_window(window_id, move |_, window, cx| {
        show_approval_overlay(queue, snapshot, window, cx);
    })
    .is_ok()
}

fn show_approval_overlay(
    queue: Arc<Mutex<ApprovalQueueState>>,
    snapshot: ApprovalQueueSnapshot,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    let request = snapshot.active;
    let details = approval_details_text(&request);
    let pending_count = snapshot.pending_count;
    let options = request.options.clone();
    window.open_dialog(cx, move |dialog, _window, cx| {
        dialog
            .title(t!("AcpApproval.dialog_title").to_string())
            .w(px(520.0))
            .overlay_closable(false)
            .close_button(false)
            .keyboard(false)
            .child(
                v_flex()
                    .gap_3()
                    .child(div().text_sm().child(request.summary.clone()))
                    .when(pending_count > 0, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("AcpApproval.pending_count", count = pending_count)),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .max_h(px(180.0))
                            .overflow_y_scrollbar()
                            .child(details.clone()),
                    ),
            )
            .footer({
                let queue = queue.clone();
                let options = options.clone();
                move |_, _, _window, _cx| option_buttons(queue.clone(), options.clone())
            })
            .on_cancel({
                let queue = queue.clone();
                move |_, window, cx| {
                    resolve_current_approval(&queue, AcpPermissionOutcome::Cancelled, window, cx);
                    true
                }
            })
    });
}

fn option_buttons(
    queue: Arc<Mutex<ApprovalQueueState>>,
    options: Vec<one_core::ai_chat::AcpPermissionOption>,
) -> Vec<AnyElement> {
    let mut buttons = Vec::new();
    for option in options {
        let option_id = option.option_id.clone();
        let queue = queue.clone();
        let mut button = Button::new(format!("acp-approval-option-{option_id}"))
            .label(option.name)
            .with_size(Size::Small);
        if option.kind.starts_with("reject") {
            button = button.danger();
        } else if option.kind.starts_with("allow") {
            button = button.success();
        }
        buttons.push(
            button
                .on_click(move |_, window, cx| {
                    resolve_current_approval(
                        &queue,
                        AcpPermissionOutcome::Selected {
                            option_id: option_id.clone(),
                        },
                        window,
                        cx,
                    );
                })
                .into_any_element(),
        );
    }
    buttons
}

fn resolve_current_approval(
    queue: &Arc<Mutex<ApprovalQueueState>>,
    outcome: AcpPermissionOutcome,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    window.close_dialog(cx);
    let has_next = queue
        .lock()
        .expect("ACP approval queue lock poisoned")
        .resolve_active(outcome);
    if has_next {
        let queue = queue.clone();
        window.defer(cx, move |_, cx| {
            if !present_next_approval(queue.clone(), cx) {
                queue
                    .lock()
                    .expect("ACP approval queue lock poisoned")
                    .cancel_all();
            }
        });
    }
}

fn approval_details_text(request: &AcpPermissionRequest) -> String {
    serde_json::to_string_pretty(&request.details).unwrap_or_else(|_| request.details.to_string())
}
