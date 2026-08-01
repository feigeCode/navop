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
        let algorithm = request.presented.algorithm;
        let fingerprint = request.presented.fingerprint;

        window.open_dialog(cx, move |dialog, _window, cx| {
            let reject_view = view.clone();
            let accept_once_view = view.clone();
            let accept_save_view = view.clone();

            dialog
                .title(t!("SshSession.host_key_title").to_string())
                .w(px(520.))
                .child(
                    v_flex()
                        .gap_3()
                        .child(t!("SshSession.host_key_message").to_string())
                        .child(
                            v_flex()
                                .gap_2()
                                .p_3()
                                .rounded_md()
                                .bg(cx.theme().secondary)
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .w(px(150.))
                                                .text_color(cx.theme().muted_foreground)
                                                .child(
                                                    t!("SshSession.host_key_identity").to_string(),
                                                ),
                                        )
                                        .child(identity.clone()),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .w(px(150.))
                                                .text_color(cx.theme().muted_foreground)
                                                .child(
                                                    t!("SshSession.host_key_algorithm").to_string(),
                                                ),
                                        )
                                        .child(algorithm.clone()),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .w(px(150.))
                                                .text_color(cx.theme().muted_foreground)
                                                .child(
                                                    t!("SshSession.host_key_fingerprint")
                                                        .to_string(),
                                                ),
                                        )
                                        .child(div().text_xs().child(fingerprint.clone())),
                                ),
                        ),
                )
                .footer(move |_, _, _window, _cx| {
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
                            .label(t!("SshSession.host_key_accept_save").to_string())
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
                })
                .overlay_closable(false)
                .close_button(false)
                .keyboard(false)
        });
    }
}
