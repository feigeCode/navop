//! 「生成密钥」里 Ed25519 之外的算法：ECDSA 与 RSA。
//!
//! 编码交给 `ssh-key`，输出结构与 `keygen.rs` 里手写的 Ed25519 一致：
//! 未加密的 `openssh-key-v1` 私钥、一行 `authorized_keys` 公钥、SHA256 指纹。

use ssh_key::{
    Comment, EcdsaCurve, HashAlg, LineEnding, PrivateKey,
    getrandom::SysRng,
    private::{EcdsaKeypair, KeypairData, RsaKeypair},
    rand_core::UnwrapErr,
};

use super::keygen::{GeneratedSshKey, generate_ed25519_key};

/// RSA 密钥位数：3072 位兼顾兼容性与生成耗时，4096 位在低配机器上要等数秒。
const RSA_KEY_SIZE: usize = 3072;

/// 「生成密钥」可选的算法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshKeyAlgorithm {
    /// Ed25519，默认且最通用。
    Ed25519,
    /// ECDSA NIST P-256。
    EcdsaP256,
    /// ECDSA NIST P-384。
    EcdsaP384,
    /// RSA 3072 位。
    Rsa3072,
}

impl SshKeyAlgorithm {
    /// 下拉框里的全部可选项，顺序即展示顺序。
    pub const ALL: [SshKeyAlgorithm; 4] = [
        SshKeyAlgorithm::Ed25519,
        SshKeyAlgorithm::EcdsaP256,
        SshKeyAlgorithm::EcdsaP384,
        SshKeyAlgorithm::Rsa3072,
    ];
}

/// 按指定算法生成一对 SSH 密钥。
///
/// Ed25519 仍走手写编码，保证 `keygen.rs` 里已有的格式约束不变；其余算法交给
/// `ssh-key` 生成并编码。RSA 生成较慢，调用方应放到后台线程执行。
pub fn generate_ssh_key(
    algorithm: SshKeyAlgorithm,
    comment: &str,
) -> Result<GeneratedSshKey, String> {
    match algorithm {
        SshKeyAlgorithm::Ed25519 => Ok(generate_ed25519_key(comment)),
        SshKeyAlgorithm::EcdsaP256 => generate_ecdsa_key(EcdsaCurve::NistP256, comment),
        SshKeyAlgorithm::EcdsaP384 => generate_ecdsa_key(EcdsaCurve::NistP384, comment),
        SshKeyAlgorithm::Rsa3072 => generate_rsa_key(comment),
    }
}

/// 生成 ECDSA 密钥；曲线支持取决于 `ssh-key` 的 p256 / p384 特性。
fn generate_ecdsa_key(curve: EcdsaCurve, comment: &str) -> Result<GeneratedSshKey, String> {
    let mut rng = UnwrapErr(SysRng);
    let keypair = EcdsaKeypair::random(&mut rng, curve).map_err(|error| error.to_string())?;
    encode_keypair(KeypairData::Ecdsa(keypair), comment)
}

/// 生成 RSA 密钥；3072 位通常需要零点几秒，慢机器上可能更久。
fn generate_rsa_key(comment: &str) -> Result<GeneratedSshKey, String> {
    let mut rng = UnwrapErr(SysRng);
    let keypair = RsaKeypair::random(&mut rng, RSA_KEY_SIZE).map_err(|error| error.to_string())?;
    encode_keypair(KeypairData::Rsa(keypair), comment)
}

/// 把密钥对编码成私钥文本、公钥文本与指纹三件套。
fn encode_keypair(keypair: KeypairData, comment: &str) -> Result<GeneratedSshKey, String> {
    let key = PrivateKey::new(keypair, Comment::from(comment.trim()))
        .map_err(|error| error.to_string())?;
    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    let public_key_openssh = key
        .public_key()
        .to_openssh()
        .map_err(|error| error.to_string())?;
    let private_key_openssh = key
        .to_openssh(LineEnding::LF)
        .map_err(|error| error.to_string())?
        .to_string();

    Ok(GeneratedSshKey {
        private_key_openssh,
        public_key_openssh,
        fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成的密钥要能被 `ssh-key` 自己解析回来，算法与注释都对得上。
    #[test]
    fn every_algorithm_parses_back_with_the_expected_name_and_comment() {
        let cases = [
            (SshKeyAlgorithm::EcdsaP256, "ecdsa-sha2-nistp256"),
            (SshKeyAlgorithm::EcdsaP384, "ecdsa-sha2-nistp384"),
            (SshKeyAlgorithm::Rsa3072, "ssh-rsa"),
        ];

        for (algorithm, name) in cases {
            let key = generate_ssh_key(algorithm, "工作机").expect("generated key");
            assert!(
                key.public_key_openssh.starts_with(&format!("{name} ")),
                "{algorithm:?} 的公钥算法名不对：{}",
                key.public_key_openssh
            );
            assert!(key.public_key_openssh.ends_with(" 工作机"));

            let parsed = PrivateKey::from_openssh(&key.private_key_openssh).expect("可解析");
            assert_eq!(parsed.algorithm().as_str(), name);
            assert!(!parsed.is_encrypted());
            assert_eq!(
                parsed.fingerprint(HashAlg::Sha256).to_string(),
                key.fingerprint
            );

            let public = parsed.public_key().to_openssh().expect("公钥编码");
            assert_eq!(public, key.public_key_openssh);
        }
    }

    #[test]
    fn fingerprints_are_sha256_and_differ_between_generated_keys() {
        let first = generate_ssh_key(SshKeyAlgorithm::EcdsaP256, "").expect("first");
        let second = generate_ssh_key(SshKeyAlgorithm::EcdsaP256, "").expect("second");

        assert!(first.fingerprint.starts_with("SHA256:"));
        assert_ne!(first.private_key_openssh, second.private_key_openssh);
        assert_ne!(first.fingerprint, second.fingerprint);
    }

    /// 用系统 `ssh-keygen` 交叉验证私钥真能被 OpenSSH 接受（RSA 生成慢，跳过）。
    #[cfg(unix)]
    #[test]
    fn openssh_accepts_the_generated_ecdsa_private_key() {
        use std::process::Command;

        let key = generate_ssh_key(SshKeyAlgorithm::EcdsaP256, "navop-test").expect("generated");
        let dir = std::env::temp_dir().join(format!("navop-keygen-ecdsa-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("id_ecdsa");
        std::fs::write(&path, &key.private_key_openssh).expect("write private key");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .expect("chmod private key");
        }

        let output = match Command::new("ssh-keygen")
            .args(["-y", "-f"])
            .arg(&path)
            .output()
        {
            Ok(output) => output,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&dir);
                eprintln!("跳过：无法执行 ssh-keygen（{error}）");
                return;
            }
        };
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            output.status.success(),
            "ssh-keygen 拒绝了生成的私钥：{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let derived = String::from_utf8_lossy(&output.stdout);
        assert!(
            derived.starts_with("ecdsa-sha2-nistp256 "),
            "推导出的公钥异常：{derived}"
        );
    }
}
