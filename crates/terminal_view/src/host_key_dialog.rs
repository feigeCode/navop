use gpui::{AnyElement, App, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{ActiveTheme, StyledExt, h_flex, v_flex};
use rust_i18n::t;
use ssh::{HostKeyDetails, HostKeyIdentity, HostKeyVerifier};
use terminal::terminal::HostKeyVerificationReason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostKeyDialogPresentation {
    pub(crate) title_key: &'static str,
    pub(crate) message_key: &'static str,
    pub(crate) save_label_key: &'static str,
    pub(crate) expected: Vec<HostKeyDetails>,
    pub(crate) is_changed: bool,
}

pub(crate) fn host_key_dialog_presentation(
    reason: &HostKeyVerificationReason,
) -> HostKeyDialogPresentation {
    match reason {
        HostKeyVerificationReason::Unknown => HostKeyDialogPresentation {
            title_key: "SshSession.host_key_title",
            message_key: "SshSession.host_key_message",
            save_label_key: "SshSession.host_key_accept_save",
            expected: Vec::new(),
            is_changed: false,
        },
        HostKeyVerificationReason::Changed { expected } => HostKeyDialogPresentation {
            title_key: "SshSession.host_key_changed_title",
            message_key: "SshSession.host_key_changed_message",
            save_label_key: "SshSession.host_key_update_save",
            expected: expected.clone(),
            is_changed: true,
        },
    }
}

pub(crate) fn render_host_key_details_card(
    identity: String,
    presented: HostKeyDetails,
    presentation: &HostKeyDialogPresentation,
    cx: &App,
) -> AnyElement {
    let label_color = cx.theme().muted_foreground;
    let detail_row = |label: String, value: AnyElement| {
        h_flex()
            .gap_2()
            .child(
                div()
                    .w(px(170.))
                    .flex_shrink_0()
                    .text_color(label_color)
                    .child(label),
            )
            .child(value)
    };
    let section_title = |title: String| div().mt_1().text_sm().font_semibold().child(title);

    let mut card = v_flex()
        .gap_2()
        .p_3()
        .rounded_md()
        .bg(cx.theme().secondary)
        .child(detail_row(
            t!("SshSession.host_key_identity").to_string(),
            div().child(identity).into_any_element(),
        ));

    if presentation.is_changed {
        card = card.child(section_title(
            t!("SshSession.host_key_presented").to_string(),
        ));
    }

    card = card
        .child(detail_row(
            t!("SshSession.host_key_algorithm").to_string(),
            div().child(presented.algorithm).into_any_element(),
        ))
        .child(detail_row(
            t!("SshSession.host_key_fingerprint").to_string(),
            div()
                .text_xs()
                .child(presented.fingerprint)
                .into_any_element(),
        ));

    if presentation.is_changed {
        card = card.child(section_title(
            t!("SshSession.host_key_expected").to_string(),
        ));
        for expected in &presentation.expected {
            card = card
                .child(detail_row(
                    t!("SshSession.host_key_algorithm").to_string(),
                    div().child(expected.algorithm.clone()).into_any_element(),
                ))
                .child(detail_row(
                    t!("SshSession.host_key_fingerprint").to_string(),
                    div()
                        .text_xs()
                        .child(expected.fingerprint.clone())
                        .into_any_element(),
                ));
        }
    }

    card.into_any_element()
}

pub(crate) fn verifier_with_confirmed_host_key(
    verifier: HostKeyVerifier,
    identity: HostKeyIdentity,
    presented: HostKeyDetails,
    reason: &HostKeyVerificationReason,
    persist: bool,
) -> HostKeyVerifier {
    match reason {
        HostKeyVerificationReason::Unknown => {
            verifier.with_confirmed_key(identity, presented, persist)
        }
        HostKeyVerificationReason::Changed { .. } => {
            verifier.with_confirmed_changed_key(identity, presented, persist)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::host_key_dialog_presentation;
    use ssh::HostKeyDetails;
    use terminal::terminal::HostKeyVerificationReason;

    #[test]
    fn unknown_host_key_dialog_keeps_first_trust_copy() {
        let presentation = host_key_dialog_presentation(&HostKeyVerificationReason::Unknown);

        assert_eq!(presentation.title_key, "SshSession.host_key_title");
        assert_eq!(presentation.message_key, "SshSession.host_key_message");
        assert_eq!(
            presentation.save_label_key,
            "SshSession.host_key_accept_save"
        );
        assert!(presentation.expected.is_empty());
        assert!(!presentation.is_changed);
    }

    #[test]
    fn changed_host_key_dialog_uses_replacement_warning_and_keeps_all_old_keys() {
        let expected = vec![
            HostKeyDetails {
                algorithm: "ssh-ed25519".to_string(),
                fingerprint: "SHA256:old".to_string(),
            },
            HostKeyDetails {
                algorithm: "ecdsa-sha2-nistp256".to_string(),
                fingerprint: "SHA256:older".to_string(),
            },
        ];

        let presentation = host_key_dialog_presentation(&HostKeyVerificationReason::Changed {
            expected: expected.clone(),
        });

        assert_eq!(presentation.title_key, "SshSession.host_key_changed_title");
        assert_eq!(
            presentation.message_key,
            "SshSession.host_key_changed_message"
        );
        assert_eq!(
            presentation.save_label_key,
            "SshSession.host_key_update_save"
        );
        assert_eq!(presentation.expected, expected);
        assert!(presentation.is_changed);
    }
}
