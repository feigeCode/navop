//! SSH 密钥生成：生成 Ed25519 密钥对并编码成 OpenSSH 使用的文本格式。
//!
//! 生成的私钥遵循 `PROTOCOL.key` 定义的 `openssh-key-v1` 结构（未加密的
//! `none` cipher），可以被 OpenSSH / PuTTY / Xshell 等工具直接导入；公钥则是
//! `~/.ssh/authorized_keys` 里使用的一行文本。

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::SigningKey;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

/// `openssh-key-v1` 私钥块的固定魔数。
const OPENSSH_PRIVATE_KEY_MAGIC: &[u8] = b"openssh-key-v1\0";
/// Ed25519 在 OpenSSH 里的算法名。
const ED25519_KEY_TYPE: &[u8] = b"ssh-ed25519";
/// OpenSSH 私钥 PEM 的 base64 行宽。
const PEM_LINE_WIDTH: usize = 70;

/// 一次生成出来的 SSH 密钥对。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSshKey {
    /// `-----BEGIN OPENSSH PRIVATE KEY-----` 开头的私钥文本。
    pub private_key_openssh: String,
    /// `ssh-ed25519 <base64> <comment>` 形式的公钥文本。
    pub public_key_openssh: String,
    /// `SHA256:<base64>` 形式的公钥指纹。
    pub fingerprint: String,
}

/// 生成一对 Ed25519 密钥，`comment` 会写进私钥和公钥（通常填凭据名称）。
pub fn generate_ed25519_key(comment: &str) -> GeneratedSshKey {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let signing_key = SigningKey::from_bytes(&seed);
    encode_ed25519_key(&signing_key, comment)
}

/// 把给定的 Ed25519 私钥编码成 OpenSSH 文本格式。
///
/// 单独抽出来是为了让测试可以用固定种子构造密钥，验证编码结果稳定。
fn encode_ed25519_key(signing_key: &SigningKey, comment: &str) -> GeneratedSshKey {
    let public_bytes = signing_key.verifying_key().to_bytes();
    let public_blob = encode_public_key_blob(&public_bytes);
    let private_blob = encode_private_section(signing_key, &public_bytes, comment);

    let fingerprint = format!("SHA256:{}", STANDARD.encode(Sha256::digest(&public_blob)));
    let comment = comment.trim();
    let public_key_openssh = if comment.is_empty() {
        format!("ssh-ed25519 {}", STANDARD.encode(&public_blob))
    } else {
        format!("ssh-ed25519 {} {}", STANDARD.encode(&public_blob), comment)
    };

    GeneratedSshKey {
        private_key_openssh: encode_private_key_pem(&public_blob, &private_blob),
        public_key_openssh,
        fingerprint,
    }
}

/// OpenSSH 公钥二进制：`string "ssh-ed25519"` + `string <32 字节公钥>`。
fn encode_public_key_blob(public_bytes: &[u8; 32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(4 + ED25519_KEY_TYPE.len() + 4 + public_bytes.len());
    write_string(&mut blob, ED25519_KEY_TYPE);
    write_string(&mut blob, public_bytes);
    blob
}

/// 私钥区段：两个相同的校验整数 + 密钥本体 + 注释 + 1..=n 的填充，总长按 8 字节对齐。
fn encode_private_section(
    signing_key: &SigningKey,
    public_bytes: &[u8; 32],
    comment: &str,
) -> Vec<u8> {
    let mut section = Vec::new();
    let check = OsRng.next_u32();
    section.extend_from_slice(&check.to_be_bytes());
    section.extend_from_slice(&check.to_be_bytes());
    write_string(&mut section, ED25519_KEY_TYPE);
    write_string(&mut section, public_bytes);

    // Ed25519 私钥在 OpenSSH 里是 32 字节种子 + 32 字节公钥。
    let mut private_bytes = [0u8; 64];
    private_bytes[..32].copy_from_slice(&signing_key.to_bytes());
    private_bytes[32..].copy_from_slice(public_bytes);
    write_string(&mut section, &private_bytes);

    write_string(&mut section, comment.trim().as_bytes());

    let mut padding = 1u8;
    while section.len() % 8 != 0 {
        section.push(padding);
        padding = padding.wrapping_add(1).max(1);
    }

    section
}

/// 外层结构：魔数 + cipher/kdf 参数 + 公钥 + 私钥区段，再做 PEM 包装。
fn encode_private_key_pem(public_blob: &[u8], private_section: &[u8]) -> String {
    let mut blob = Vec::new();
    blob.extend_from_slice(OPENSSH_PRIVATE_KEY_MAGIC);
    write_string(&mut blob, b"none");
    write_string(&mut blob, b"none");
    write_string(&mut blob, b"");
    blob.extend_from_slice(&1u32.to_be_bytes());
    write_string(&mut blob, public_blob);
    write_string(&mut blob, private_section);

    let encoded = STANDARD.encode(&blob);
    let mut pem = String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n");
    for chunk in encoded.as_bytes().chunks(PEM_LINE_WIDTH) {
        pem.push_str(std::str::from_utf8(chunk).expect("base64 一定是合法 UTF-8"));
        pem.push('\n');
    }
    pem.push_str("-----END OPENSSH PRIVATE KEY-----\n");
    pem
}

/// 按 SSH wire format 写入 `uint32 长度 + 内容`。
fn write_string(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn fixed_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn decode_base64_body(pem: &str) -> Vec<u8> {
        let body: String = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect::<Vec<_>>()
            .concat();
        STANDARD.decode(body).expect("私钥正文应是合法 base64")
    }

    fn read_string(blob: &[u8], cursor: &mut usize) -> Vec<u8> {
        let length = u32::from_be_bytes(blob[*cursor..*cursor + 4].try_into().unwrap()) as usize;
        *cursor += 4;
        let value = blob[*cursor..*cursor + length].to_vec();
        *cursor += length;
        value
    }

    #[test]
    fn generated_key_is_a_valid_openssh_private_key() {
        let key = encode_ed25519_key(&fixed_key(), "工作机");

        assert!(
            key.private_key_openssh
                .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----\n")
        );
        assert!(
            key.private_key_openssh
                .ends_with("-----END OPENSSH PRIVATE KEY-----\n")
        );

        let blob = decode_base64_body(&key.private_key_openssh);
        assert!(blob.starts_with(OPENSSH_PRIVATE_KEY_MAGIC));

        let mut cursor = OPENSSH_PRIVATE_KEY_MAGIC.len();
        assert_eq!(read_string(&blob, &mut cursor), b"none");
        assert_eq!(read_string(&blob, &mut cursor), b"none");
        assert!(read_string(&blob, &mut cursor).is_empty());

        let key_count = u32::from_be_bytes(blob[cursor..cursor + 4].try_into().unwrap());
        cursor += 4;
        assert_eq!(key_count, 1);

        let public_blob = read_string(&blob, &mut cursor);
        let private_section = read_string(&blob, &mut cursor);
        assert_eq!(cursor, blob.len(), "私钥 blob 不应有尾随数据");

        // 外层公钥和私钥区段里的公钥必须一致。
        let mut public_cursor = 0;
        assert_eq!(
            read_string(&public_blob, &mut public_cursor),
            ED25519_KEY_TYPE
        );
        let outer_public = read_string(&public_blob, &mut public_cursor);
        assert_eq!(outer_public, fixed_key().verifying_key().to_bytes());

        let mut section_cursor = 0;
        let check_a = u32::from_be_bytes(
            private_section[section_cursor..section_cursor + 4]
                .try_into()
                .unwrap(),
        );
        section_cursor += 4;
        let check_b = u32::from_be_bytes(
            private_section[section_cursor..section_cursor + 4]
                .try_into()
                .unwrap(),
        );
        section_cursor += 4;
        assert_eq!(check_a, check_b, "两个校验整数必须相同");
        assert_eq!(
            read_string(&private_section, &mut section_cursor),
            ED25519_KEY_TYPE
        );
        assert_eq!(
            read_string(&private_section, &mut section_cursor),
            outer_public
        );

        let private_bytes = read_string(&private_section, &mut section_cursor);
        assert_eq!(private_bytes.len(), 64);
        assert_eq!(&private_bytes[..32], &fixed_key().to_bytes());
        assert_eq!(&private_bytes[32..], &outer_public[..]);
        assert_eq!(
            read_string(&private_section, &mut section_cursor),
            "工作机".as_bytes()
        );

        assert_eq!(
            private_section.len() % 8,
            0,
            "私钥区段长度必须按 8 字节对齐"
        );
    }

    #[test]
    fn generated_public_key_matches_the_private_key_and_carries_the_comment() {
        let key = encode_ed25519_key(&fixed_key(), "工作机");

        let mut parts = key.public_key_openssh.split(' ');
        assert_eq!(parts.next(), Some("ssh-ed25519"));
        let encoded = parts.next().expect("公钥 base64");
        assert_eq!(parts.next(), Some("工作机"));
        assert_eq!(parts.next(), None);

        let blob = STANDARD.decode(encoded).expect("公钥应是合法 base64");
        let mut cursor = 0;
        assert_eq!(read_string(&blob, &mut cursor), ED25519_KEY_TYPE);
        assert_eq!(
            read_string(&blob, &mut cursor),
            fixed_key().verifying_key().to_bytes()
        );
    }

    #[test]
    fn fingerprint_is_the_sha256_of_the_public_key_blob() {
        let key = generate_ed25519_key("");

        let encoded = key
            .public_key_openssh
            .strip_prefix("ssh-ed25519 ")
            .expect("公钥前缀");
        let blob = STANDARD.decode(encoded).expect("公钥应是合法 base64");
        let expected = format!("SHA256:{}", STANDARD.encode(Sha256::digest(&blob)));

        assert_eq!(key.fingerprint, expected);
        assert!(!key.public_key_openssh.ends_with(' '));
    }

    /// 用系统自带的 `ssh-keygen` 交叉验证生成的私钥确实能被 OpenSSH 接受。
    #[cfg(unix)]
    #[test]
    fn openssh_accepts_the_generated_private_key() {
        use std::process::Command;

        let key = generate_ed25519_key("navop-test");
        let dir = std::env::temp_dir().join(format!("navop-keygen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("id_ed25519");
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
            derived.starts_with("ssh-ed25519 "),
            "ssh-keygen 推导出的公钥异常：{derived}"
        );
        assert_eq!(
            derived.trim().split(' ').nth(1),
            key.public_key_openssh.split(' ').nth(1)
        );
    }

    #[test]
    fn two_generated_keys_use_different_seeds() {
        let first = generate_ed25519_key("a");
        let second = generate_ed25519_key("a");

        assert_ne!(first.private_key_openssh, second.private_key_openssh);
        assert_ne!(first.public_key_openssh, second.public_key_openssh);
    }
}
