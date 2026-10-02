//! WebDAV 密码的落盘加密。
//!
//! 目标：让 `settings.json` 里不出现明文密码。
//!
//! 采用两级策略：
//! 1. 已解锁主密钥时，复用仓库现有的 `crypto::encrypt_password`（AES-256-GCM，密钥是主密钥派生值）。
//! 2. 未解锁主密钥时，退化为用一个内置的本地固定密钥加密，密文格式同样是 `ENC:base64(nonce + ciphertext)`。
//!    这与 `crate::key_storage` 保存主密钥文件时的做法同级——能挡住「打开配置文件直接抄走密码」，
//!    但挡不住能读本机文件又能读本程序二进制的攻击者。真正的强度依赖第 1 级主密钥。
//!
//! 读取时两种密文都能解开：先按主密钥试，失败再按本地固定密钥试，最后是历史遗留的明文（自动升级）。

use crate::crypto;

/// 主密钥不可用时的兜底密钥。
///
/// 刻意与 `key_storage::LOCAL_STORAGE_FIXED_KEY` 取值不同：即便本地密钥文件被读取，
/// 也不至于直接得到解 WebDAV 密码的同一把钥匙。
const LOCAL_FALLBACK_KEY: &[u8; 32] = b"navop-webdav-local-fallback-key!";

/// 加密 WebDAV 密码以便持久化。
///
/// 空串与已经是 `ENC:` 密文的值原样返回，避免重复加密。
pub fn seal(password: &str) -> String {
    if password.is_empty() || crypto::is_encrypted(password) {
        return password.to_string();
    }

    if crypto::has_master_key() {
        let sealed = crypto::encrypt_password(password);
        if crypto::is_encrypted(&sealed) {
            return sealed;
        }
    }

    crypto::encrypt_with_key(password, local_fallback_key())
}

/// 还原由 [`seal`] 加密的 WebDAV 密码。
///
/// 返回值为空串时按「未配置密码」处理。
pub fn open(stored: &str) -> String {
    if stored.is_empty() {
        return String::new();
    }

    // 历史遗留的明文 settings.json：直接放行，下次保存时会被 seal 升级为密文。
    if !crypto::is_encrypted(stored) {
        return stored.to_string();
    }

    if crypto::has_master_key() {
        let plaintext = crypto::decrypt_password(stored);
        if !plaintext.is_empty() {
            return plaintext;
        }
    }

    crypto::decrypt_with_key(stored, local_fallback_key()).unwrap_or_default()
}

fn local_fallback_key() -> &'static str {
    std::str::from_utf8(LOCAL_FALLBACK_KEY).unwrap_or("navop-webdav-local-fallback")
}

#[cfg(test)]
mod tests {
    use super::{LOCAL_FALLBACK_KEY, open, seal};

    #[test]
    fn empty_password_round_trips_as_empty() {
        assert_eq!("", seal(""));
        assert_eq!("", open(""));
    }

    #[test]
    fn sealed_password_is_never_stored_in_plaintext() {
        let sealed = seal("super-secret-password");

        assert!(sealed.starts_with("ENC:"));
        assert!(!sealed.contains("super-secret-password"));
    }

    #[test]
    fn sealing_twice_is_idempotent() {
        let once = seal("super-secret-password");
        let twice = seal(&once);

        assert_eq!(once, twice);
    }

    #[test]
    fn rounds_trip_through_the_fallback_key_when_no_master_key() {
        let sealed = seal("super-secret-password");

        assert_eq!("super-secret-password", open(&sealed));
    }

    #[test]
    fn legacy_plaintext_passwords_are_read_back_as_is() {
        assert_eq!("legacy-plain", open("legacy-plain"));
    }

    #[test]
    fn corrupted_ciphertext_yields_empty_password() {
        assert_eq!("", open("ENC:not-valid-base64!!"));
    }

    #[test]
    fn fallback_key_has_expected_length() {
        assert_eq!(32, LOCAL_FALLBACK_KEY.len());
    }
}
