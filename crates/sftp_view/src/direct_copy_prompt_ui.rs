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
    h_flex, v_flex,
};
use rust_i18n::t;
use sftp::{DirectCopyDecision, DirectCopyPreview, DirectCopyStrategy, ServerCopyAuthKind};
use std::sync::Arc;
use std::sync::atomic::Ordering;

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
        let direct_view = view.clone();
        let relay_view = view.clone();
        let cancel_view = view.clone();

        dialog
            .title(t!("Transfer.direct_copy_title").to_string())
            .w(px(720.))
            .child(prompt_content(&preview, cx))
            .on_cancel(move |_, window, cx| {
                finish_prompt(
                    &cancel_view,
                    task_id,
                    dialog_handle,
                    &cancel_response,
                    DirectCopyDecision::UseRelay,
                    window,
                    cx,
                );
                false
            })
            .footer(move |_, _, window, cx| {
                vec![
                    prompt_button(
                        "sftp-direct-copy-relay",
                        t!("Transfer.direct_copy_use_relay").to_string(),
                        DirectCopyDecision::UseRelay,
                        task_id,
                        dialog_handle,
                        relay_view.clone(),
                        relay_response.clone(),
                        false,
                    ),
                    prompt_button(
                        "sftp-direct-copy-confirm",
                        direct_button_label(preview.strategy),
                        DirectCopyDecision::UseDirect,
                        task_id,
                        dialog_handle,
                        direct_view.clone(),
                        direct_response.clone(),
                        true,
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
    decision: DirectCopyDecision,
    task_id: usize,
    dialog_handle: DialogHandle,
    view: WeakEntity<SftpView>,
    response: PromptDecisionSender,
    primary: bool,
) -> PromptButton {
    Box::new(move |_window, _cx| {
        let view = view.clone();
        let response = response.clone();
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
        if primary {
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
    send_prompt_decision(response, decision);
    let _ = view.update(cx, |this, _cx| {
        let _ = this.resolve_direct_copy_prompt(task_id, decision);
    });
    window.close_dialog_by_handle(dialog_handle, cx);
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
        .child(
            h_flex()
                .gap_2()
                .child(t!("Transfer.direct_copy_route").to_string())
                .child(format!("{source}  →  {target}")),
        )
        .child(
            t!(
                "Transfer.direct_copy_item_count",
                count = preview.item_count
            )
            .to_string(),
        )
        .child(
            div().p_3().rounded_md().bg(cx.theme().secondary).child(
                v_flex()
                    .gap_2()
                    .child(
                        t!(
                            "Transfer.direct_copy_navop_source_auth",
                            auth = auth_label(preview.navop_source_auth)
                        )
                        .to_string(),
                    )
                    .child(
                        t!(
                            "Transfer.direct_copy_navop_target_auth",
                            auth = auth_label(preview.navop_target_auth)
                        )
                        .to_string(),
                    ),
            ),
        )
        .child(t!("Transfer.direct_copy_auth_boundary").to_string())
        .child(t!("Transfer.direct_copy_security").to_string())
        .child(t!("Transfer.direct_copy_prompt").to_string())
        .into_any_element()
}

fn direct_button_label(strategy: DirectCopyStrategy) -> String {
    let strategy = match strategy {
        DirectCopyStrategy::Rsync => t!("Transfer.strategy_rsync").to_string(),
        DirectCopyStrategy::Scp => t!("Transfer.strategy_scp").to_string(),
    };
    t!("Transfer.direct_copy_use_direct", strategy = strategy).to_string()
}

fn auth_label(auth: ServerCopyAuthKind) -> String {
    match auth {
        ServerCopyAuthKind::Password => t!("Transfer.auth_password").to_string(),
        ServerCopyAuthKind::PrivateKeyFile => t!("Transfer.auth_private_key_file").to_string(),
        ServerCopyAuthKind::PrivateKeyContent => {
            t!("Transfer.auth_private_key_content").to_string()
        }
        ServerCopyAuthKind::Agent => t!("Transfer.auth_agent").to_string(),
        ServerCopyAuthKind::AutoPublicKey => t!("Transfer.auth_auto_public_key").to_string(),
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
    use super::format_endpoint;

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
}
