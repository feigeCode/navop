use std::error::Error as StdError;
use std::fmt;

use anyhow::Error;
use rust_i18n::t;

/// A user-actionable SSH negotiation error shown when a connection has not
/// opted in to the legacy algorithm compatibility list.
#[derive(Debug)]
pub struct LegacyAlgorithmRequired {
    message: String,
}

impl fmt::Display for LegacyAlgorithmRequired {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl StdError for LegacyAlgorithmRequired {}

/// A private key loading/decoding failure with an actionable remediation hint.
#[derive(Debug)]
pub(crate) struct PrivateKeyLoadFailed {
    message: String,
}

impl fmt::Display for PrivateKeyLoadFailed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl StdError for PrivateKeyLoadFailed {}

/// 跳板机 direct-tcpip 通道被远端拒绝（含服务器回传的原因码与描述）。
///
/// 典型场景是 JumpServer 这类堡垒机默认禁用端口转发：客户端能登录跳板机，
/// 但 `channel_open_direct_tcpip` 被以 `AdministrativelyProhibited` 拒绝，
/// 界面上只剩 `Failed to open channel (AdministrativelyProhibited)` 一个枚举名，
/// 用户无从知道该关掉「跳板机」直连资产。
#[derive(Debug)]
pub struct JumpChannelRejected {
    message: String,
}

impl fmt::Display for JumpChannelRejected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl StdError for JumpChannelRejected {}

/// Wrap private-key loading failures so the terminal does not reduce a wrong
/// passphrase or malformed key to russh's opaque `SshKey: cryptographic error`.
pub(crate) fn add_private_key_load_hint(error: Error) -> Error {
    Error::new(PrivateKeyLoadFailed {
        message: format!("{}\n\n{error:#}", t!("Ssh.private_key_load_failed")),
    })
}

/// Add an actionable hint when russh reports that no common legacy-key
/// algorithm exists and this connection has not enabled legacy algorithms.
///
/// Both legacy key-exchange algorithms and a server whose only host-key
/// algorithm is `ssh-dss` are covered; newer-capable servers are not affected.
/// Host-key changes, authentication failures, network errors, and failures in
/// other SSH algorithm categories are deliberately left unchanged.
pub fn add_legacy_algorithm_hint(error: Error, allow_legacy_algorithms: bool) -> Error {
    if allow_legacy_algorithms || !is_no_common_algorithm_with_legacy_remedy(&error) {
        return error;
    }

    Error::new(LegacyAlgorithmRequired {
        message: format!("{error:#}\n\n{}", t!("Ssh.legacy_algorithms_required_hint")),
    })
}

/// 给「跳板机 direct-tcpip 通道被拒」的错误追加可执行的处理建议。
///
/// `description` 是服务器在 CHANNEL_OPEN_FAILURE 里回传的描述文本（可能为空）。
/// 仅当错误链里确实是 `russh::Error::ChannelOpenFailure` 时才包装，其余错误原样返回。
pub fn add_jump_channel_rejected_hint(error: Error, description: &str) -> Error {
    let Some(reason) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<russh::Error>())
        .and_then(|error| match error {
            russh::Error::ChannelOpenFailure(reason) => Some(reason.clone()),
            _ => None,
        })
    else {
        return error;
    };

    let reason_code = reason.code();

    let mut message = format!(
        "{error:#}\n\n{}",
        t!("Ssh.jump_channel_rejected_hint", reason = reason_code)
    );
    let trimmed = description.trim();
    if !trimmed.is_empty() {
        message.push_str(&format!(
            "\n\n{}",
            t!("Ssh.server_reason", reason = trimmed)
        ));
    }

    Error::new(JumpChannelRejected { message })
}

fn is_no_common_algorithm_with_legacy_remedy(error: &Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<russh::Error>()
            .is_some_and(|error| match error {
                russh::Error::NoCommonAlgo { kind, theirs, .. } => matches_kind(kind, theirs),
                _ => false,
            })
    })
}

/// The legacy compatibility list only covers SHA-1 KEX, the DSA host-key
/// algorithm, and hmac-sha1, so a hint is only useful for those failures.
fn matches_kind(kind: &russh::AlgorithmKind, theirs: &[String]) -> bool {
    match kind {
        russh::AlgorithmKind::Kex => true,
        russh::AlgorithmKind::Key => theirs
            .iter()
            .any(|name| matches!(name.as_str(), "ssh-dss" | "ssh-dss-cert-v01@openssh.com")),
        russh::AlgorithmKind::Mac => theirs.iter().any(|name| name == "hmac-sha1"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LegacyAlgorithmRequired, add_jump_channel_rejected_hint, add_legacy_algorithm_hint,
    };

    fn no_common_algorithm(kind: russh::AlgorithmKind) -> anyhow::Error {
        no_common_algorithm_with_theirs(kind, vec!["theirs".to_owned()])
    }

    fn no_common_algorithm_with_theirs(
        kind: russh::AlgorithmKind,
        theirs: Vec<String>,
    ) -> anyhow::Error {
        russh::Error::NoCommonAlgo {
            kind,
            ours: vec!["ours".to_owned()],
            theirs,
        }
        .into()
    }

    #[test]
    fn disabled_legacy_algorithms_adds_hint_for_kex_negotiation_failure() {
        let error = add_legacy_algorithm_hint(
            no_common_algorithm(russh::AlgorithmKind::Kex).context("handshake failed"),
            false,
        );

        assert!(error.downcast_ref::<LegacyAlgorithmRequired>().is_some());
        assert!(error.to_string().contains("No common Kex algorithm"));
        assert!(error.to_string().contains("Allow Legacy SSH Algorithms"));
    }

    #[test]
    fn enabled_legacy_algorithms_do_not_suggest_enabling_the_option_again() {
        let error = add_legacy_algorithm_hint(no_common_algorithm(russh::AlgorithmKind::Kex), true);

        assert!(error.downcast_ref::<LegacyAlgorithmRequired>().is_none());
        assert!(!error.to_string().contains("Allow Legacy SSH Algorithms"));
    }

    #[test]
    fn ssh_dss_host_key_negotiation_failure_suggests_enabling_legacy_algorithms() {
        let error = add_legacy_algorithm_hint(
            no_common_algorithm_with_theirs(russh::AlgorithmKind::Key, vec!["ssh-dss".to_owned()]),
            false,
        );

        assert!(error.downcast_ref::<LegacyAlgorithmRequired>().is_some());
        assert!(error.to_string().contains("No common Key algorithm"));
        assert!(error.to_string().contains("Allow Legacy SSH Algorithms"));
    }

    #[test]
    fn unrelated_host_key_algorithm_failure_does_not_suggest_legacy_algorithms() {
        let error = add_legacy_algorithm_hint(
            no_common_algorithm_with_theirs(
                russh::AlgorithmKind::Key,
                vec!["sk-ssh-ed25519@openssh.com".to_owned()],
            ),
            false,
        );

        assert!(error.downcast_ref::<LegacyAlgorithmRequired>().is_none());
        assert!(error.to_string().contains("No common Key algorithm"));
    }

    #[test]
    fn hmac_sha1_mac_negotiation_failure_suggests_enabling_legacy_algorithms() {
        let error = add_legacy_algorithm_hint(
            no_common_algorithm_with_theirs(
                russh::AlgorithmKind::Mac,
                vec!["hmac-sha1".to_owned()],
            ),
            false,
        );

        assert!(error.downcast_ref::<LegacyAlgorithmRequired>().is_some());
        assert!(error.to_string().contains("No common Mac algorithm"));
        assert!(error.to_string().contains("Allow Legacy SSH Algorithms"));
    }

    #[test]
    fn unsupported_mac_negotiation_failure_does_not_suggest_legacy_algorithms() {
        let error = add_legacy_algorithm_hint(
            no_common_algorithm_with_theirs(russh::AlgorithmKind::Mac, vec!["hmac-md5".to_owned()]),
            false,
        );

        assert!(error.downcast_ref::<LegacyAlgorithmRequired>().is_none());
        assert!(error.to_string().contains("No common Mac algorithm"));
    }

    #[test]
    fn unrelated_errors_do_not_add_legacy_kex_hint() {
        for message in [
            "changed SSH host key",
            "Password authentication failed",
            "SSH connection timed out",
        ] {
            let error = add_legacy_algorithm_hint(anyhow::anyhow!(message), false);

            assert!(error.downcast_ref::<LegacyAlgorithmRequired>().is_none());
            assert_eq!(error.to_string(), message);
        }
    }

    fn administratively_prohibited() -> anyhow::Error {
        russh::Error::ChannelOpenFailure(russh::ChannelOpenFailure::AdministrativelyProhibited)
            .into()
    }

    fn connect_failed() -> anyhow::Error {
        russh::Error::ChannelOpenFailure(russh::ChannelOpenFailure::ConnectFailed).into()
    }

    #[test]
    fn jump_channel_rejection_includes_hint_and_server_description() {
        let error = add_jump_channel_rejected_hint(
            administratively_prohibited(),
            "port forwarding is disabled",
        );

        assert!(error.downcast_ref::<super::JumpChannelRejected>().is_some());
        assert!(error.to_string().contains("Failed to open channel"));
        assert!(error.to_string().contains("jump server"));
        assert!(error.to_string().contains("port forwarding is disabled"));
    }

    #[test]
    fn jump_channel_rejection_without_description_keeps_hint() {
        let error = add_jump_channel_rejected_hint(administratively_prohibited(), "  ");

        assert!(error.downcast_ref::<super::JumpChannelRejected>().is_some());
        assert!(error.to_string().contains("jump server"));
    }

    #[test]
    fn jump_channel_hint_covers_connect_failed_rejections() {
        let error = add_jump_channel_rejected_hint(connect_failed(), "");

        assert!(error.downcast_ref::<super::JumpChannelRejected>().is_some());
        assert!(error.to_string().contains("Failed to open channel"));
    }

    #[test]
    fn unrelated_errors_are_not_wrapped_as_jump_channel_rejections() {
        let error = add_jump_channel_rejected_hint(
            anyhow::anyhow!("Password authentication failed"),
            "ignored",
        );

        assert!(error.downcast_ref::<super::JumpChannelRejected>().is_none());
        assert_eq!(error.to_string(), "Password authentication failed");
    }
}
