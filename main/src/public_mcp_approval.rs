mod channel;
#[cfg(test)]
mod protocol_tests;
mod queue;

use channel::channel_approver;
use queue::{ApprovalQueueSnapshot, ApprovalQueueState};

use gpui::{App, AppContext, AsyncApp, Global, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, WindowExt, button::ButtonVariant, dialog::DialogButtonProps,
    scroll::ScrollableElement, v_flex,
};
use public_mcp::approval::{
    PublicMcpApprovalManager, PublicMcpApprovalOutcome, PublicMcpApprovalRequest,
};
use public_mcp::approval_grants::PublicMcpApprovalGrantStore;
use rust_i18n::t;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);
const ACP_APPROVAL_GRANT_TIMEOUT: Duration = Duration::from_secs(15);

pub struct GlobalPublicMcpApprovalQueue {
    manager: PublicMcpApprovalManager,
}

impl Global for GlobalPublicMcpApprovalQueue {}

pub fn init(cx: &mut App) {
    if cx.has_global::<GlobalPublicMcpApprovalQueue>() {
        return;
    }

    let grants = PublicMcpApprovalGrantStore::new(ACP_APPROVAL_GRANT_TIMEOUT);
    let (approver, mut receiver) = channel_approver(APPROVAL_TIMEOUT, grants.clone());
    let manager = PublicMcpApprovalManager::new(Arc::new(approver));
    cx.set_global(GlobalPublicMcpApprovalQueue {
        manager: manager.clone(),
    });
    ai_chat_view::set_acp_permission_grant_provider(cx, move |request, option| {
        if !option.kind.starts_with("allow") {
            return None;
        }
        let arguments = request.raw_input()?.clone();
        let grant_id = grants.register(arguments)?;
        let grants = grants.clone();
        Some(ai_chat_view::AcpPermissionGrant::new(move || {
            grants.revoke(&grant_id);
        }))
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

pub fn approval_manager(cx: &App) -> PublicMcpApprovalManager {
    cx.try_global::<GlobalPublicMcpApprovalQueue>()
        .map(|queue| queue.manager.clone())
        .unwrap_or_default()
}

struct ApprovalEnvelope {
    request: PublicMcpApprovalRequest,
    response_tx: oneshot::Sender<PublicMcpApprovalOutcome>,
}

impl ApprovalEnvelope {
    #[cfg(test)]
    fn approve(self) {
        let _ = self.response_tx.send(PublicMcpApprovalOutcome::Approved);
    }

    fn deny(self, reason: impl Into<String>) {
        self.resolve(PublicMcpApprovalOutcome::Denied {
            reason: Some(reason.into()),
        });
    }

    fn resolve(self, outcome: PublicMcpApprovalOutcome) {
        let _ = self.response_tx.send(outcome);
    }
}

fn present_approval_request(
    queue: Arc<Mutex<ApprovalQueueState>>,
    envelope: ApprovalEnvelope,
    cx: &mut AsyncApp,
) {
    queue
        .lock()
        .expect("approval queue lock poisoned")
        .enqueue(envelope);

    let shown = cx.update(|cx| present_next_approval(queue.clone(), cx));
    if !shown {
        deny_all_pending_approvals(
            &queue,
            "no active window is available for public MCP approval",
        );
    }
}

fn present_next_approval(queue: Arc<Mutex<ApprovalQueueState>>, cx: &mut App) -> bool {
    let Some(window_id) = cx.active_window() else {
        return false;
    };
    let snapshot = queue
        .lock()
        .expect("approval queue lock poisoned")
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
    let summary = request.summary;
    let pending_count = snapshot.pending_count;

    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title(t!("McpApproval.dialog_title").to_string())
            .confirm()
            .overlay_closable(false)
            .keyboard(false)
            .button_props(
                DialogButtonProps::default()
                    .ok_text(t!("McpApproval.approve"))
                    .ok_variant(ButtonVariant::Success)
                    .cancel_text(t!("McpApproval.deny"))
                    .cancel_variant(ButtonVariant::Danger),
            )
            .child(
                v_flex()
                    .gap_3()
                    .child(div().text_sm().child(summary.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(_cx.theme().muted_foreground)
                            .child(t!("McpApproval.pending_count", count = pending_count)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(_cx.theme().muted_foreground)
                            .max_h(px(180.0))
                            .overflow_y_scrollbar()
                            .child(details.clone()),
                    ),
            )
            .on_ok({
                let queue = queue.clone();
                move |_, window, cx| {
                    resolve_current_approval(
                        &queue,
                        PublicMcpApprovalOutcome::Approved,
                        window,
                        cx,
                    );
                    true
                }
            })
            .on_cancel({
                let queue = queue.clone();
                move |_, window, cx| {
                    resolve_current_approval(
                        &queue,
                        PublicMcpApprovalOutcome::Denied {
                            reason: Some("operator denied public MCP request".to_string()),
                        },
                        window,
                        cx,
                    );
                    true
                }
            })
    });
}

fn resolve_current_approval(
    queue: &Arc<Mutex<ApprovalQueueState>>,
    outcome: PublicMcpApprovalOutcome,
    window: &mut gpui::Window,
    cx: &mut App,
) {
    let has_next = queue
        .lock()
        .expect("approval queue lock poisoned")
        .resolve_active(outcome);
    if has_next {
        let queue = queue.clone();
        window.defer(cx, move |_, cx| {
            if !present_next_approval(queue.clone(), cx) {
                deny_all_pending_approvals(
                    &queue,
                    "no active window is available for public MCP approval",
                );
            }
        });
    }
}

fn deny_all_pending_approvals(queue: &Arc<Mutex<ApprovalQueueState>>, reason: &'static str) {
    queue
        .lock()
        .expect("approval queue lock poisoned")
        .deny_all(reason);
}

fn approval_details_text(request: &PublicMcpApprovalRequest) -> String {
    serde_json::to_string_pretty(&request.details).unwrap_or_else(|_| request.details.to_string())
}
