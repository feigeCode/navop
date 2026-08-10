use super::*;
use gpui_component::scroll::ScrollableElement;

impl TerminalView {
    pub(super) fn render_connection_banner(
        &self,
        can_reconnect: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (is_connecting, error_msg) = self.connection_status(cx);
        let is_reconnecting_ssh = self.reconnect_success_pending
            && self.terminal.read(cx).connection_kind() == TerminalConnectionKind::Ssh;
        let theme = cx.theme();
        div()
            .absolute()
            .top(px(16.0))
            .left(px(16.0))
            .right(px(16.0))
            .flex()
            .justify_center()
            .child(
                h_flex()
                    .min_w_0()
                    .max_w(px(760.0))
                    .gap_3()
                    .px_4()
                    .py_3()
                    .bg(theme.popover)
                    .border_1()
                    .border_color(theme.border)
                    .rounded_lg()
                    .shadow_lg()
                    .child(self.render_connection_icon(is_connecting, cx))
                    .child(self.render_connection_summary(
                        is_connecting,
                        is_reconnecting_ssh,
                        error_msg,
                        cx,
                    ))
                    .when(can_reconnect && !is_connecting, |this| {
                        this.child(
                            Button::new("reconnect-btn")
                                .label(t!("SshSession.reconnect"))
                                .primary()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.reconnect(window, cx);
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_connection_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .items_center()
                    .gap_4()
                    .p_6()
                    .w(px(560.0))
                    .max_w(px(640.0))
                    .bg(theme.popover)
                    .border_1()
                    .border_color(theme.border)
                    .rounded_lg()
                    .shadow_lg()
                    .child(self.render_connection_dialog_title(cx))
                    .when_some(self.render_ssh_auth_form(cx), |this, form| this.child(form)),
            )
            .into_any_element()
    }

    fn connection_status(&self, cx: &App) -> (bool, Option<String>) {
        match self.terminal.read(cx).connection_state() {
            ConnectionState::Connecting => (true, None),
            ConnectionState::Disconnected { error } => (false, error.clone()),
            ConnectionState::Connected => (false, None),
        }
    }

    fn render_connection_icon(&self, is_connecting: bool, cx: &App) -> Icon {
        let theme = cx.theme();
        Icon::new(if is_connecting {
            IconName::Loader
        } else {
            IconName::CircleX
        })
        .color()
        .with_size(px(20.0))
        .text_color(if is_connecting {
            theme.warning
        } else {
            theme.danger
        })
    }

    fn render_connection_summary(
        &self,
        is_connecting: bool,
        is_reconnecting_ssh: bool,
        error_msg: Option<String>,
        cx: &App,
    ) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.popover_foreground)
                    .child(if is_connecting {
                        t!("SshSession.connecting")
                    } else {
                        t!("SshSession.connection_lost")
                    }),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(if is_connecting {
                        if is_reconnecting_ssh {
                            t!("SshSession.reconnecting_preserves_terminal")
                        } else {
                            t!("SshSession.connecting_preserves_terminal")
                        }
                    } else {
                        t!("SshSession.disconnected_preserves_terminal")
                    }),
            )
            .when_some(error_msg, |this, message| {
                this.child(
                    div()
                        .w_full()
                        .max_h(px(180.0))
                        .overflow_scrollbar()
                        .whitespace_normal()
                        .text_sm()
                        .text_color(theme.danger)
                        .child(message),
                )
            })
            .into_any_element()
    }

    fn render_connection_dialog_title(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let title = if self.terminal.read(cx).ssh_credential_request().is_some() {
            t!("SshSession.credentials_required")
        } else {
            t!("SshSession.authentication_required")
        };
        h_flex()
            .gap_2()
            .child(
                Icon::new(IconName::Loader)
                    .color()
                    .with_size(px(24.0))
                    .text_color(theme.warning),
            )
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.popover_foreground)
                    .child(title),
            )
            .into_any_element()
    }

    fn render_ssh_auth_form(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.render_ssh_credential_form(cx)
            .or_else(|| self.render_ssh_mfa_form(cx))
    }

    fn render_ssh_credential_form(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let request = self.terminal.read(cx).ssh_credential_request()?;
        let inputs = self.ssh_credential_inputs.as_ref()?;
        if inputs.request.generation() != request.generation() {
            return None;
        }

        Some(
            v_flex()
                .gap_3()
                .w_full()
                .items_center()
                .child(
                    div()
                        .w(px(400.0))
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("SshSession.credentials_hint")),
                )
                .when_some(inputs.username.as_ref(), |this, input| {
                    this.child(
                        v_flex()
                            .w(px(400.0))
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("SshSession.username")),
                            )
                            .child(Input::new(input)),
                    )
                })
                .when_some(inputs.password.as_ref(), |this, input| {
                    this.child(
                        v_flex()
                            .w(px(400.0))
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("SshSession.password")),
                            )
                            .child(Input::new(input).mask_toggle()),
                    )
                })
                .child(
                    Button::new("submit-ssh-credentials")
                        .label(t!("Common.ok"))
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.submit_ssh_credentials(window, cx);
                        })),
                )
                .into_any_element(),
        )
    }

    fn render_ssh_mfa_form(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let request = self.terminal.read(cx).ssh_mfa_request()?;
        Some(
            v_flex()
                .gap_2()
                .w_full()
                .items_center()
                .children((0..request.prompts.len()).filter_map(|index| {
                    let input = self.ssh_mfa_inputs.get(index)?;
                    let input = if input.echo {
                        Input::new(&input.input).into_any_element()
                    } else {
                        Input::new(&input.input).mask_toggle().into_any_element()
                    };
                    Some(div().w(px(320.0)).child(input))
                }))
                .child(
                    Button::new("submit-ssh-mfa")
                        .label(t!("Common.ok"))
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.submit_ssh_mfa(window, cx);
                        })),
                )
                .into_any_element(),
        )
    }
}
