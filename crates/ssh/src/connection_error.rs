use std::error::Error as StdError;
use std::fmt;

use anyhow::Error;
use rust_i18n::t;

/// A user-actionable SSH negotiation error shown when a connection has not
/// opted in to the legacy KEX compatibility list.
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

/// Add an actionable hint only when russh reports that no common key-exchange
/// algorithm exists and this connection has not enabled legacy algorithms.
///
/// Host-key changes, authentication failures, network errors, and failures in
/// other SSH algorithm categories are deliberately left unchanged.
pub fn add_legacy_algorithm_hint(error: Error, allow_legacy_algorithms: bool) -> Error {
    if allow_legacy_algorithms || !is_no_common_kex_algorithm(&error) {
        return error;
    }

    Error::new(LegacyAlgorithmRequired {
        message: format!("{error:#}\n\n{}", t!("Ssh.legacy_algorithms_required_hint")),
    })
}

fn is_no_common_kex_algorithm(error: &Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<russh::Error>().is_some_and(|error| {
            matches!(
                error,
                russh::Error::NoCommonAlgo {
                    kind: russh::AlgorithmKind::Kex,
                    ..
                }
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{LegacyAlgorithmRequired, add_legacy_algorithm_hint};

    fn no_common_algorithm(kind: russh::AlgorithmKind) -> anyhow::Error {
        russh::Error::NoCommonAlgo {
            kind,
            ours: vec!["ours".to_owned()],
            theirs: vec!["theirs".to_owned()],
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
    fn host_key_algorithm_negotiation_failure_does_not_add_legacy_kex_hint() {
        let error =
            add_legacy_algorithm_hint(no_common_algorithm(russh::AlgorithmKind::Key), false);

        assert!(error.downcast_ref::<LegacyAlgorithmRequired>().is_none());
        assert!(error.to_string().contains("No common Key algorithm"));
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
