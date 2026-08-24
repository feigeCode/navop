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
    use super::{LegacyAlgorithmRequired, add_legacy_algorithm_hint};

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
}
