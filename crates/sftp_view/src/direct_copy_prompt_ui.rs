use super::SftpView;
use super::direct_copy_prompt::{
    ActiveDirectCopyPrompt, DirectCopyPromptRequest, PromptDecisionSender, send_prompt_decision,
};
use gpui::{
    AnyElement, AnyWindowHandle, AppContext, Context, IntoElement, ParentElement, Styled,
    WeakEntity, Window, div, px,
};
use gpui_component::{
    ActiveTheme, DialogHandle, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::ScrollableElement,
    v_flex,
};
use rust_i18n::t;
use sftp::{DirectCopyDecision, DirectCopyPreview, DirectCopyStrategy};
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PromptAction {
    Cancel,
    UseRelay,
    UseDirect,
}

impl PromptAction {
    fn decision(self) -> DirectCopyDecision {
        match self {
            Self::Cancel => DirectCopyDecision::Cancel,
            Self::UseRelay => DirectCopyDecision::UseRelay,
            Self::UseDirect => DirectCopyDecision::UseDirect,
        }
    }

    fn is_primary(self) -> bool {
        self == Self::UseDirect
    }
}

impl SftpView {
    pub(crate) fn open_direct_copy_prompt(
        &mut self,
        request: DirectCopyPromptRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drop_stale_direct_copy_prompt();
        if self.close_state.is_closing()
            || request.cancelled.load(Ordering::Relaxed)
            || self.direct_copy_prompt.is_some()
        {
            send_prompt_decision(&request.response, DirectCopyDecision::UseRelay);
            return;
        }

        let task_id = request.task_id;
        let response = Arc::downgrade(&request.response);
        let view = cx.entity().downgrade();
        let window_handle = window.window_handle();
        let dialog_handle = open_prompt_dialog(view, request, window, cx);
        self.direct_copy_prompt = Some(ActiveDirectCopyPrompt {
            task_id,
            dialog_handle,
            window_handle,
            response,
        });
    }

    pub(crate) fn close_direct_copy_prompt_for_task(
        &mut self,
        task_id: usize,
        cx: &mut Context<Self>,
    ) {
        let Some((dialog_handle, window_handle)) =
            self.resolve_direct_copy_prompt(task_id, DirectCopyDecision::UseRelay)
        else {
            return;
        };
        close_direct_copy_dialog(dialog_handle, window_handle, cx);
    }

    pub(crate) fn close_active_direct_copy_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(task_id) = self
            .direct_copy_prompt
            .as_ref()
            .map(|prompt| prompt.task_id)
        else {
            return;
        };
        self.close_direct_copy_prompt_for_task(task_id, cx);
    }

    fn resolve_direct_copy_prompt(
        &mut self,
        task_id: usize,
        decision: DirectCopyDecision,
    ) -> Option<(DialogHandle, AnyWindowHandle)> {
        let Some(prompt) = self.direct_copy_prompt.take() else {
            return None;
        };
        if prompt.task_id != task_id {
            self.direct_copy_prompt = Some(prompt);
            return None;
        }
        if let Some(response) = prompt.response.upgrade() {
            send_prompt_decision(&response, decision);
        }
        Some((prompt.dialog_handle, prompt.window_handle))
    }

    fn drop_stale_direct_copy_prompt(&mut self) {
        if self
            .direct_copy_prompt
            .as_ref()
            .is_some_and(|prompt| prompt.response.upgrade().is_none())
        {
            self.direct_copy_prompt = None;
        }
    }
}

fn close_direct_copy_dialog(
    dialog_handle: DialogHandle,
    window_handle: AnyWindowHandle,
    cx: &mut Context<SftpView>,
) {
    let _ = cx.update_window(window_handle, |_, window, cx| {
        window.close_dialog_by_handle(dialog_handle, cx)
    });
}

fn open_prompt_dialog(
    view: WeakEntity<SftpView>,
    request: DirectCopyPromptRequest,
    window: &mut Window,
    cx: &mut Context<SftpView>,
) -> DialogHandle {
    let task_id = request.task_id;
    let preview = request.preview;
    let response = request.response;

    window.open_dialog_with_handle(cx, move |dialog_handle, dialog, _window, cx| {
        let direct_response = response.clone();
        let relay_response = response.clone();
        let cancel_response = response.clone();
        let escape_response = response.clone();
        let direct_view = view.clone();
        let relay_view = view.clone();
        let cancel_view = view.clone();
        let escape_view = view.clone();

        dialog
            .title(t!("Transfer.direct_copy_title").to_string())
            .w(px(600.))
            .child(prompt_content(&preview, cx))
            .on_cancel(move |_, window, cx| {
                finish_prompt(
                    &escape_view,
                    task_id,
                    dialog_handle,
                    &escape_response,
                    PromptAction::Cancel.decision(),
                    window,
                    cx,
                );
                false
            })
            .footer(move |_, _, window, cx| {
                vec![
                    prompt_button(
                        "sftp-direct-copy-cancel",
                        t!("Transfer.direct_copy_cancel").to_string(),
                        PromptAction::Cancel,
                        task_id,
                        dialog_handle,
                        cancel_view.clone(),
                        cancel_response.clone(),
                    ),
                    prompt_button(
                        "sftp-direct-copy-relay",
                        t!("Transfer.direct_copy_use_relay").to_string(),
                        PromptAction::UseRelay,
                        task_id,
                        dialog_handle,
                        relay_view.clone(),
                        relay_response.clone(),
                    ),
                    prompt_button(
                        "sftp-direct-copy-confirm",
                        t!("Transfer.direct_copy_use_direct").to_string(),
                        PromptAction::UseDirect,
                        task_id,
                        dialog_handle,
                        direct_view.clone(),
                        direct_response.clone(),
                    ),
                ]
                .into_iter()
                .map(|button| button(window, cx))
                .collect()
            })
            .overlay_closable(false)
            .close_button(false)
    })
}

type PromptButton = Box<dyn Fn(&mut Window, &mut gpui::App) -> AnyElement>;

#[allow(clippy::too_many_arguments)]
fn prompt_button(
    id: &'static str,
    label: String,
    action: PromptAction,
    task_id: usize,
    dialog_handle: DialogHandle,
    view: WeakEntity<SftpView>,
    response: PromptDecisionSender,
) -> PromptButton {
    Box::new(move |_window, _cx| {
        let view = view.clone();
        let response = response.clone();
        let decision = action.decision();
        let button = Button::new(id)
            .label(label.clone())
            .on_click(move |_, window, cx| {
                finish_prompt(
                    &view,
                    task_id,
                    dialog_handle,
                    &response,
                    decision,
                    window,
                    cx,
                );
            });
        if action.is_primary() {
            button.primary().into_any_element()
        } else {
            button.ghost().into_any_element()
        }
    })
}

fn finish_prompt(
    view: &WeakEntity<SftpView>,
    task_id: usize,
    dialog_handle: DialogHandle,
    response: &PromptDecisionSender,
    decision: DirectCopyDecision,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let _ = view.update(cx, |this, _cx| {
        this.update_server_copy_route_status(task_id, decision);
    });
    send_prompt_decision(response, decision);
    let _ = view.update(cx, |this, _cx| {
        let _ = this.resolve_direct_copy_prompt(task_id, decision);
    });
    window.close_dialog_by_handle(dialog_handle, cx);
}

impl SftpView {
    fn update_server_copy_route_status(&mut self, task_id: usize, decision: DirectCopyDecision) {
        let status = match decision {
            DirectCopyDecision::UseDirect => {
                Some(t!("Transfer.direct_copy_authenticating").to_string())
            }
            DirectCopyDecision::UseRelay => Some(t!("Transfer.relay_copy_starting").to_string()),
            DirectCopyDecision::Cancel => None,
        };
        let Some(status) = status else {
            return;
        };
        let Some(task) = self
            .transfer_queue
            .tasks
            .iter()
            .find(|task| task.id == task_id)
        else {
            return;
        };
        if task.state != super::TransferTaskState::Running
            || task.shared_progress.cancelled.load(Ordering::Relaxed)
            || !matches!(&task.operation, super::TransferOperation::ServerCopy(_))
        {
            return;
        }
        task.shared_progress
            .scanning
            .store(false, Ordering::Relaxed);
        if let Ok(mut current_file) = task.shared_progress.current_file.write() {
            *current_file = Some(status);
        }
    }
}

fn prompt_content(preview: &DirectCopyPreview, cx: &mut gpui::App) -> AnyElement {
    let source = format_endpoint(
        &preview.source_username,
        &preview.source_host,
        preview.source_port,
    );
    let target = format_endpoint(
        &preview.target_username,
        &preview.target_host,
        preview.target_port,
    );
    v_flex()
        .gap_3()
        .max_h(px(520.))
        .overflow_y_scrollbar()
        .child(t!("Transfer.direct_copy_intro", count = preview.item_count).to_string())
        .child(
            h_flex()
                .gap_2()
                .child(t!("Transfer.direct_copy_route").to_string())
                .child(format!("{source}  →  {target}")),
        )
        .child(
            div().p_3().rounded_md().bg(cx.theme().secondary).child(
                v_flex()
                    .gap_2()
                    .child(
                        t!(
                            "Transfer.direct_copy_server_auth_heading",
                            strategy = strategy_label(preview.strategy)
                        )
                        .to_string(),
                    )
                    .child(t!("Transfer.direct_copy_auth_boundary").to_string())
                    .child(t!("Transfer.direct_copy_security").to_string()),
            ),
        )
        .into_any_element()
}

fn strategy_label(strategy: DirectCopyStrategy) -> String {
    match strategy {
        DirectCopyStrategy::Rsync => t!("Transfer.strategy_rsync").to_string(),
        DirectCopyStrategy::Scp => t!("Transfer.strategy_scp").to_string(),
    }
}

fn format_endpoint(username: &str, host: &str, port: u16) -> String {
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    format!("{username}@{host}:{port}")
}

#[cfg(test)]
mod tests {
    use super::{PromptAction, format_endpoint};
    use sftp::DirectCopyDecision;

    #[test]
    fn endpoint_display_brackets_ipv6_hosts() {
        assert_eq!(
            "root@[2001:db8::1]:22",
            format_endpoint("root", "2001:db8::1", 22)
        );
        assert_eq!(
            "root@example.com:2222",
            format_endpoint("root", "example.com", 2222)
        );
    }

    #[test]
    fn prompt_actions_map_to_distinct_copy_decisions() {
        assert_eq!(DirectCopyDecision::Cancel, PromptAction::Cancel.decision());
        assert_eq!(
            DirectCopyDecision::UseRelay,
            PromptAction::UseRelay.decision()
        );
        assert_eq!(
            DirectCopyDecision::UseDirect,
            PromptAction::UseDirect.decision()
        );
        assert!(PromptAction::UseDirect.is_primary());
        assert!(!PromptAction::UseRelay.is_primary());
        assert!(!PromptAction::Cancel.is_primary());
    }
}
