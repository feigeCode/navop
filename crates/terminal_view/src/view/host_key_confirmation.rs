use super::*;

impl TerminalView {
    pub(super) fn show_host_key_verification_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.terminal.read(cx).host_key_verification_request() else {
            return;
        };

        let view = cx.entity().downgrade();
        let identity = request.identity.to_string();
        let presented = request.presented;
        let presentation = host_key_dialog_presentation(&request.reason);

        window.open_dialog(cx, move |dialog, _window, cx| {
            let reject_view = view.clone();
            let accept_once_view = view.clone();
            let accept_save_view = view.clone();

            dialog
                .title(t!(presentation.title_key).to_string())
                .w(px(520.))
                .child(
                    v_flex()
                        .gap_3()
                        .child(t!(presentation.message_key).to_string())
                        .child(render_host_key_details_card(
                            identity.clone(),
                            presented.clone(),
                            &presentation,
                            cx,
                        )),
                )
                .footer(DialogFooter::new().children({
                    let reject_view = reject_view.clone();
                    let accept_once_view = accept_once_view.clone();
                    let accept_save_view = accept_save_view.clone();
                    vec![
                        Button::new("ssh-host-key-reject")
                            .label(t!("SshSession.host_key_reject").to_string())
                            .danger()
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                let _ = reject_view.update(cx, |this, cx| {
                                    this.terminal.update(cx, |terminal, cx| {
                                        terminal.respond_to_host_key_verification(
                                            HostKeyVerificationDecision::Reject,
                                            t!("SshSession.host_key_rejected").to_string(),
                                            cx,
                                        );
                                    });
                                });
                            })
                            .into_any_element(),
                        Button::new("ssh-host-key-accept-once")
                            .label(t!("SshSession.host_key_accept_once").to_string())
                            .ghost()
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                let _ = accept_once_view.update(cx, |this, cx| {
                                    this.focus_terminal_after_connect = true;
                                    this.terminal.update(cx, |terminal, cx| {
                                        terminal.respond_to_host_key_verification(
                                            HostKeyVerificationDecision::AcceptOnce,
                                            t!("SshSession.host_key_rejected").to_string(),
                                            cx,
                                        );
                                    });
                                });
                            })
                            .into_any_element(),
                        Button::new("ssh-host-key-accept-save")
                            .label(t!(presentation.save_label_key).to_string())
                            .primary()
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                let _ = accept_save_view.update(cx, |this, cx| {
                                    this.focus_terminal_after_connect = true;
                                    this.terminal.update(cx, |terminal, cx| {
                                        terminal.respond_to_host_key_verification(
                                            HostKeyVerificationDecision::AcceptAndSave,
                                            t!("SshSession.host_key_rejected").to_string(),
                                            cx,
                                        );
                                    });
                                });
                            })
                            .into_any_element(),
                    ]
                }))
                .overlay_closable(false)
                .close_button(false)
                .keyboard(false)
        });
    }
}
