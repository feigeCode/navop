use std::error::Error as StdError;
use std::fmt;
use std::io::Read as _;
use std::path::Path;

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

/// PuTTY `.ppk` 文件的固定开头（v2 / v3）。
const PPK_FILE_MAGIC: &str = "PuTTY-User-Key-File-";

/// 判断私钥文本是否为 PuTTY 的 `.ppk` 封装。
///
/// `.ppk` 以 `PuTTY-User-Key-File-2:` 或 `PuTTY-User-Key-File-3:` 开头；
/// 这里容忍 BOM 和空行，避免复制粘贴时首行多出空白导致漏判。
pub(crate) fn looks_like_ppk_key(content: &str) -> bool {
    content
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with(PPK_FILE_MAGIC)
}

/// 判断私钥文件是否为 PuTTY 的 `.ppk`。
///
/// 只读文件头，不为了探测把整把（可能加密的）密钥读进内存；文件不存在或
/// 不可读时返回 `false`，让后续加载流程给出原有的错误。
pub(crate) fn is_ppk_key_file(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut buffer = [0u8; 64];
    let Ok(read) = file.take(buffer.len() as u64).read(&mut buffer) else {
        return false;
    };
    looks_like_ppk_key(&String::from_utf8_lossy(&buffer[..read]))
}

/// `.ppk` 是 PuTTY 自有的封装格式，russh 不支持；若不显式识别，
/// 用户只会看到 `SshKey: cryptographic error`，误以为是口令错误。
pub(crate) fn ppk_unsupported_error() -> Error {
    Error::new(PrivateKeyLoadFailed {
        message: t!("Ssh.private_key_ppk_unsupported").to_string(),
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
    use std::fs;

    use super::{
        LegacyAlgorithmRequired, add_jump_channel_rejected_hint, add_legacy_algorithm_hint,
        is_ppk_key_file, looks_like_ppk_key, ppk_unsupported_error,
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
    fn ppk_signature_is_detected_in_pasted_content() {
        assert!(looks_like_ppk_key(
            "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\n"
        ));
        assert!(looks_like_ppk_key(
            "\u{feff}\n  PuTTY-User-Key-File-2: ssh-rsa\n"
        ));
        assert!(!looks_like_ppk_key(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk=\n"
        ));
        assert!(!looks_like_ppk_key("not a key"));
    }

    #[test]
    fn ppk_signature_is_detected_in_a_key_file() {
        let dir = std::env::temp_dir().join(format!(
            "navop-ppk-detect-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("系统时间应晚于 UNIX_EPOCH")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("应可创建临时目录");
        let ppk_path = dir.join("id_ed25519.ppk");
        fs::write(
            &ppk_path,
            "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\n",
        )
        .expect("应可写入 .ppk 文件");
        let openssh_path = dir.join("id_ed25519");
        fs::write(
            &openssh_path,
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk=\n",
        )
        .expect("应可写入 OpenSSH 私钥");

        assert!(is_ppk_key_file(&ppk_path));
        assert!(!is_ppk_key_file(&openssh_path));
        assert!(!is_ppk_key_file(&dir.join("missing.ppk")));

        fs::remove_dir_all(&dir).expect("应可清理临时目录");
    }

    #[test]
    fn ppk_error_explains_the_conversion_step() {
        let message = ppk_unsupported_error().to_string();

        assert!(
            message.contains(".ppk"),
            "错误信息应指出格式，实际: {message}"
        );
        assert!(
            message.contains("PuTTYgen"),
            "错误信息应给出转换工具，实际: {message}"
        );
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
