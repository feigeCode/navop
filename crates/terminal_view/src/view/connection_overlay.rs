use super::*;

impl TerminalView {
    pub(super) fn render_connection_overlay(
        &self,
        can_reconnect: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let connection_state = self.terminal.read(cx).connection_state().clone();
        let is_connecting = matches!(connection_state, ConnectionState::Connecting);
        let error_msg = match &connection_state {
            ConnectionState::Disconnected { error } => error.clone(),
            _ => None,
        };
        let mfa_request = self.terminal.read(cx).ssh_mfa_request();
        let has_mfa_request = mfa_request.is_some();

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(Hsla {
                h: 0.,
                s: 0.,
                l: 0.,
                a: 0.7,
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_4()
                    .p_6()
                    .bg(rgb(0x2d2d2d))
                    .rounded_lg()
                    .shadow_lg()
                    .w(px(560.0))
                    .max_w(px(640.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Icon::new(if is_connecting {
                                    IconName::Loader
                                } else {
                                    IconName::CircleX
                                })
                                .color()
                                .with_size(px(24.0))
                                .text_color(if is_connecting {
                                    rgb(0xfbbf24)
                                } else {
                                    rgb(0xef4444)
                                }),
                            )
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(0xffffff))
                                    .child(if is_connecting {
                                        t!("SshSession.connecting")
                                    } else {
                                        t!("SshSession.connection_lost")
                                    }),
                            ),
                    )
                    .when_some(error_msg, |this, msg| {
                        this.child(
                            div()
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .bg(rgb(0x1f1f1f))
                                .text_sm()
                                .text_color(rgb(0xef4444))
                                .max_w(px(480.0))
                                .max_h(px(160.0))
                                .overflow_y_scrollbar()
                                .child(msg),
                        )
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x9ca3af))
                            .child(if is_connecting {
                                t!("SshSession.establishing")
                            } else {
                                t!("SshSession.disconnected")
                            }),
                    )
                    .when_some(mfa_request, |this, request| {
                        this.child(
                            v_flex()
                                .gap_2()
                                .w_full()
                                .items_center()
                                .children((0..request.prompts.len()).map(|index| {
                                    let input = self.ssh_mfa_inputs.get(index);
                                    div()
                                        .w(px(320.0))
                                        .when_some(input, |this, input| {
                                            let input_element = if input.echo {
                                                Input::new(&input.input).into_any_element()
                                            } else {
                                                Input::new(&input.input)
                                                    .mask_toggle()
                                                    .into_any_element()
                                            };
                                            this.child(input_element)
                                        })
                                        .into_any_element()
                                }))
                                .child(
                                    h_flex().justify_center().child(
                                        Button::new("submit-ssh-mfa")
                                            .label(t!("Common.ok"))
                                            .primary()
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.submit_ssh_mfa(window, cx);
                                            })),
                                    ),
                                ),
                        )
                    })
                    .when(
                        can_reconnect && !is_connecting && !has_mfa_request,
                        |this| {
                            this.child(
                                Button::new("reconnect-btn")
                                    .label(t!("SshSession.reconnect"))
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reconnect(window, cx);
                                    })),
                            )
                        },
                    ),
            )
    }
}
