use super::{ConnectionState, LeftRemoteConnectionState, SftpView};
use gpui::{
    AnyElement, App, Context, IntoElement, ParentElement, Styled, WeakEntity, Window, div, px,
};
use gpui_component::{
    ActiveTheme, DialogHandle, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use rust_i18n::t;
use ssh::{HostKeyDetails, HostKeyIdentity, HostKeyRejection, HostKeyVerifier};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HostKeyPromptReason {
    Unknown,
    Changed { expected: Vec<HostKeyDetails> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostKeyPromptRequest {
    identity: HostKeyIdentity,
    presented: HostKeyDetails,
    reason: HostKeyPromptReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostKeyPromptTarget {
    Main {
        generation: u64,
    },
    Left {
        connection_id: Option<i64>,
        generation: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostKeyPromptDecision {
    Reject,
    AcceptOnce,
    AcceptAndSave,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HostKeyPromptPresentation {
    title_key: &'static str,
    message_key: &'static str,
    save_label_key: &'static str,
    expected: Vec<HostKeyDetails>,
    is_changed: bool,
}

pub(crate) fn host_key_prompt_request(error: &anyhow::Error) -> Option<HostKeyPromptRequest> {
    error
        .chain()
        .find_map(|cause| match cause.downcast_ref::<HostKeyRejection>()? {
            HostKeyRejection::Unknown {
                identity,
                presented,
            } => Some(HostKeyPromptRequest {
                identity: identity.clone(),
                presented: presented.clone(),
                reason: HostKeyPromptReason::Unknown,
            }),
            HostKeyRejection::Changed {
                identity,
                presented,
                expected,
            } => Some(HostKeyPromptRequest {
                identity: identity.clone(),
                presented: presented.clone(),
                reason: HostKeyPromptReason::Changed {
                    expected: expected.clone(),
                },
            }),
            HostKeyRejection::Revoked { .. } | HostKeyRejection::StoreUnavailable { .. } => None,
        })
}

fn host_key_prompt_presentation(reason: &HostKeyPromptReason) -> HostKeyPromptPresentation {
    match reason {
        HostKeyPromptReason::Unknown => HostKeyPromptPresentation {
            title_key: "HostKey.title",
            message_key: "HostKey.message",
            save_label_key: "HostKey.accept_save",
            expected: Vec::new(),
            is_changed: false,
        },
        HostKeyPromptReason::Changed { expected } => HostKeyPromptPresentation {
            title_key: "HostKey.changed_title",
            message_key: "HostKey.changed_message",
            save_label_key: "HostKey.update_save",
            expected: expected.clone(),
            is_changed: true,
        },
    }
}

fn verifier_with_confirmed_host_key(
    verifier: HostKeyVerifier,
    identity: HostKeyIdentity,
    presented: HostKeyDetails,
    reason: &HostKeyPromptReason,
    persist: bool,
) -> HostKeyVerifier {
    match reason {
        HostKeyPromptReason::Unknown => verifier.with_confirmed_key(identity, presented, persist),
        HostKeyPromptReason::Changed { .. } => {
            verifier.with_confirmed_changed_key(identity, presented, persist)
        }
    }
}

impl HostKeyPromptTarget {
    fn is_current(self, view: &SftpView) -> bool {
        match self {
            Self::Main { generation } => view.is_current_connection_generation(generation),
            Self::Left {
                connection_id,
                generation,
            } => {
                view.left_remote_id() == connection_id
                    && view.is_current_left_connection_generation(generation)
            }
        }
    }
}

impl SftpView {
    pub(crate) fn show_host_key_prompt(
        &mut self,
        target: HostKeyPromptTarget,
        request: HostKeyPromptRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() || !target.is_current(self) {
            return;
        }

        let view = cx.entity().downgrade();
        let identity = request.identity.to_string();
        let presentation = host_key_prompt_presentation(&request.reason);

        window.open_dialog_with_handle(cx, move |dialog_handle, dialog, _window, cx| {
            let reject_view = view.clone();
            let accept_once_view = view.clone();
            let accept_save_view = view.clone();

            let reject_request = request.clone();
            let accept_once_request = request.clone();
            let accept_save_request = request.clone();

            dialog
                .title(t!(presentation.title_key).to_string())
                .w(px(560.))
                .child(
                    v_flex()
                        .gap_3()
                        .child(t!(presentation.message_key).to_string())
                        .child(render_host_key_details_card(
                            identity.clone(),
                            request.presented.clone(),
                            &presentation,
                            cx,
                        )),
                )
                .footer(move |_, _, _window, _cx| {
                    vec![
                        host_key_prompt_button(
                            "sftp-host-key-reject",
                            t!("HostKey.reject").to_string(),
                            true,
                            dialog_handle,
                            target,
                            reject_request.clone(),
                            reject_view.clone(),
                            HostKeyPromptDecision::Reject,
                        ),
                        host_key_prompt_button(
                            "sftp-host-key-accept-once",
                            t!("HostKey.accept_once").to_string(),
                            false,
                            dialog_handle,
                            target,
                            accept_once_request.clone(),
                            accept_once_view.clone(),
                            HostKeyPromptDecision::AcceptOnce,
                        ),
                        host_key_prompt_button(
                            "sftp-host-key-accept-save",
                            t!(presentation.save_label_key).to_string(),
                            false,
                            dialog_handle,
                            target,
                            accept_save_request.clone(),
                            accept_save_view.clone(),
                            HostKeyPromptDecision::AcceptAndSave,
                        )
                        .primary(),
                    ]
                    .into_iter()
                    .map(IntoElement::into_any_element)
                    .collect()
                })
                .overlay_closable(false)
                .close_button(false)
                .keyboard(false)
        });
    }

    fn respond_to_host_key_prompt(
        &mut self,
        target: HostKeyPromptTarget,
        request: HostKeyPromptRequest,
        decision: HostKeyPromptDecision,
        cx: &mut Context<Self>,
    ) {
        if self.close_state.is_closing() || !target.is_current(self) {
            return;
        }

        if decision == HostKeyPromptDecision::Reject {
            match target {
                HostKeyPromptTarget::Main { .. } => {
                    self.connection_state = ConnectionState::Disconnected {
                        error: Some(t!("HostKey.rejected").to_string()),
                    };
                    self.set_connection_active(false, cx);
                }
                HostKeyPromptTarget::Left { .. } => {
                    self.set_left_connection_error(t!("HostKey.rejected").to_string(), cx);
                }
            }
            cx.notify();
            return;
        }

        let persist = decision == HostKeyPromptDecision::AcceptAndSave;
        match target {
            HostKeyPromptTarget::Main { .. } => {
                self.sftp_config.host_key_verifier = verifier_with_confirmed_host_key(
                    self.sftp_config.host_key_verifier.clone(),
                    request.identity,
                    request.presented,
                    &request.reason,
                    persist,
                );
                self.reconnect(cx);
            }
            HostKeyPromptTarget::Left { .. } => {
                let Some(endpoint) = self.left_remote.as_mut() else {
                    return;
                };
                endpoint.config.host_key_verifier = verifier_with_confirmed_host_key(
                    endpoint.config.host_key_verifier.clone(),
                    request.identity,
                    request.presented,
                    &request.reason,
                    persist,
                );
                endpoint.state = LeftRemoteConnectionState::Connecting;
                endpoint.loading = false;
                self.connect_left_remote(cx);
                cx.notify();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn host_key_prompt_button(
    id: &'static str,
    label: String,
    danger: bool,
    dialog_handle: DialogHandle,
    target: HostKeyPromptTarget,
    request: HostKeyPromptRequest,
    view: WeakEntity<SftpView>,
    decision: HostKeyPromptDecision,
) -> Button {
    let button = Button::new(id).label(label).on_click(move |_, window, cx| {
        window.close_dialog_by_handle(dialog_handle, cx);
        let request = request.clone();
        let _ = view.update(cx, |this, cx| {
            this.respond_to_host_key_prompt(target, request, decision, cx);
        });
    });

    if danger {
        button.danger()
    } else {
        button.ghost()
    }
}

fn render_host_key_details_card(
    identity: String,
    presented: HostKeyDetails,
    presentation: &HostKeyPromptPresentation,
    cx: &App,
) -> AnyElement {
    let label_color = cx.theme().muted_foreground;
    let detail_row = |label: String, value: AnyElement| {
        h_flex()
            .gap_2()
            .items_start()
            .child(
                div()
                    .w(px(170.))
                    .flex_shrink_0()
                    .text_color(label_color)
                    .child(label),
            )
            .child(div().min_w_0().flex_1().child(value))
    };
    let section_title = |title: String| div().mt_1().text_sm().font_semibold().child(title);

    let mut card = v_flex()
        .gap_2()
        .p_3()
        .rounded_md()
        .bg(cx.theme().secondary)
        .child(detail_row(
            t!("HostKey.identity").to_string(),
            div().child(identity).into_any_element(),
        ));

    if presentation.is_changed {
        card = card.child(section_title(t!("HostKey.presented").to_string()));
    }

    card = card
        .child(detail_row(
            t!("HostKey.algorithm").to_string(),
            div().child(presented.algorithm).into_any_element(),
        ))
        .child(detail_row(
            t!("HostKey.fingerprint").to_string(),
            div()
                .text_xs()
                .child(presented.fingerprint)
                .into_any_element(),
        ));

    if presentation.is_changed {
        card = card.child(section_title(t!("HostKey.expected").to_string()));
        for expected in &presentation.expected {
            card = card
                .child(detail_row(
                    t!("HostKey.algorithm").to_string(),
                    div().child(expected.algorithm.clone()).into_any_element(),
                ))
                .child(detail_row(
                    t!("HostKey.fingerprint").to_string(),
                    div()
                        .text_xs()
                        .child(expected.fingerprint.clone())
                        .into_any_element(),
                ));
        }
    }

    card.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{HostKeyPromptReason, host_key_prompt_presentation, host_key_prompt_request};
    use ssh::{HostKeyDetails, HostKeyIdentity, HostKeyProxyType, HostKeyRejection, HostKeyRoute};

    fn identity() -> HostKeyIdentity {
        HostKeyIdentity::new("host.example", 22, HostKeyRoute::Direct)
    }

    fn details(fingerprint: &str) -> HostKeyDetails {
        HostKeyDetails {
            algorithm: "ssh-ed25519".to_string(),
            fingerprint: fingerprint.to_string(),
        }
    }

    #[test]
    fn wrapped_unknown_host_key_error_becomes_a_prompt_request() {
        let error = anyhow::Error::new(HostKeyRejection::Unknown {
            identity: identity(),
            presented: details("SHA256:new"),
        })
        .context("SFTP connection failed");

        let request = host_key_prompt_request(&error).expect("host-key prompt");

        assert_eq!(request.identity, identity());
        assert_eq!(request.presented, details("SHA256:new"));
        assert_eq!(request.reason, HostKeyPromptReason::Unknown);
    }

    #[test]
    fn changed_host_key_request_keeps_all_previous_fingerprints() {
        let expected = vec![details("SHA256:old"), details("SHA256:older")];
        let error = anyhow::Error::new(HostKeyRejection::Changed {
            identity: identity(),
            presented: details("SHA256:new"),
            expected: expected.clone(),
        });

        let request = host_key_prompt_request(&error).expect("changed host-key prompt");

        assert_eq!(
            request.reason,
            HostKeyPromptReason::Changed {
                expected: expected.clone()
            }
        );
        let presentation = host_key_prompt_presentation(&request.reason);
        assert!(presentation.is_changed);
        assert_eq!(presentation.expected, expected);
        assert_eq!(presentation.save_label_key, "HostKey.update_save");
    }

    #[test]
    fn revoked_and_store_errors_remain_non_interactive_failures() {
        let revoked = anyhow::Error::new(HostKeyRejection::Revoked {
            identity: identity(),
            presented: details("SHA256:revoked"),
        });
        let store_unavailable = anyhow::Error::new(HostKeyRejection::StoreUnavailable {
            identity: HostKeyIdentity::new(
                "host.example",
                22,
                HostKeyRoute::Proxy {
                    proxy_type: HostKeyProxyType::Socks5,
                    host: "proxy.example".to_string(),
                    port: 1080,
                },
            ),
            presented: details("SHA256:new"),
            reason: "permission denied".to_string(),
        });

        assert!(host_key_prompt_request(&revoked).is_none());
        assert!(host_key_prompt_request(&store_unavailable).is_none());
    }
}
