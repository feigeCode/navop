//! MIT-MAGIC-COOKIE-1 cookie 类型与 SSH 转发请求参数。
//!
//! MIT-MAGIC-COOKIE-1 的认证数据固定为 16 字节（X 协议规定），因此
//! cookie 类型直接使用定长数组，避免变长缓冲区带来的边界处理。

use std::fmt;

use rand::{RngCore, rngs::OsRng};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::{X11Error, X11Result};

pub const COOKIE_LEN: usize = 16;

/// 16 字节 MIT-MAGIC-COOKIE-1。属于持有者凭据，比较走常量时间，
/// Debug 输出只保留长度信息。
#[derive(Clone)]
pub struct MagicCookie {
    bytes: Zeroizing<[u8; COOKIE_LEN]>,
}

impl MagicCookie {
    /// 生成密码学随机 cookie（用于签发给 sshd 的 fake cookie）。
    pub fn generate() -> Self {
        let mut bytes = [0u8; COOKIE_LEN];
        OsRng.fill_bytes(&mut bytes);
        Self {
            bytes: Zeroizing::new(bytes),
        }
    }

    /// 从 32 位十六进制字符串还原。
    pub fn from_hex(text: &str) -> X11Result<Self> {
        let raw = hex::decode(text.trim())
            .map_err(|error| X11Error::CookieMalformed(error.to_string()))?;
        Self::from_slice(&raw)
    }

    /// 从字节切片拷贝，长度必须恰好 16。
    pub fn from_slice(raw: &[u8]) -> X11Result<Self> {
        let bytes: [u8; COOKIE_LEN] = raw.try_into().map_err(|_| {
            X11Error::CookieMalformed(format!("expected {COOKIE_LEN} bytes, got {}", raw.len()))
        })?;
        Ok(Self {
            bytes: Zeroizing::new(bytes),
        })
    }

    pub fn bytes(&self) -> &[u8; COOKIE_LEN] {
        &self.bytes
    }

    pub fn hex(&self) -> String {
        hex::encode(self.bytes.as_slice())
    }

    /// 与任意字节串做常量时间比较。
    pub fn matches(&self, presented: &[u8]) -> bool {
        presented.len() == COOKIE_LEN && bool::from(self.bytes.as_slice().ct_eq(presented))
    }
}

impl PartialEq for MagicCookie {
    fn eq(&self, other: &Self) -> bool {
        self.matches(other.bytes.as_slice())
    }
}

impl Eq for MagicCookie {}

impl fmt::Debug for MagicCookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MagicCookie(<{COOKIE_LEN} bytes, redacted>)")
    }
}

/// 发给 sshd 的 `x11-req` 请求参数（只含 fake cookie，真实 cookie 不出本机）。
pub struct ForwardRequest {
    /// sshd 是否只接受单个 X11 回连（对应 SSH 的 single connection 标志）。
    pub single_use: bool,
    /// 远端 DISPLAY 的 screen 号。
    pub screen: u32,
    cookie_hex: String,
}

impl ForwardRequest {
    /// SSH 与 X11 setup 报文里使用的认证协议名。
    pub const AUTH_NAME: &'static str = "MIT-MAGIC-COOKIE-1";

    pub(crate) fn new(single_use: bool, screen: u32, cookie_hex: String) -> Self {
        Self {
            single_use,
            screen,
            cookie_hex,
        }
    }

    pub fn auth_name(&self) -> &'static str {
        Self::AUTH_NAME
    }

    pub fn cookie_hex(&self) -> &str {
        &self.cookie_hex
    }
}

impl Drop for ForwardRequest {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.cookie_hex);
    }
}

impl fmt::Debug for ForwardRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForwardRequest")
            .field("single_use", &self.single_use)
            .field("screen", &self.screen)
            .field("cookie_hex", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "5f8c2e91a04b7d36e1c98f02b4537ad6";

    #[test]
    fn hex_codec_round_trip() {
        let cookie = MagicCookie::from_hex(&HEX.to_uppercase()).unwrap();
        assert_eq!(cookie.hex(), HEX);
    }

    #[test]
    fn from_slice_enforces_exact_length() {
        assert!(MagicCookie::from_slice(&[1u8; 15]).is_err());
        assert!(MagicCookie::from_slice(&[1u8; 16]).is_ok());
        assert!(MagicCookie::from_slice(&[1u8; 17]).is_err());
    }

    #[test]
    fn generated_cookies_are_distinct() {
        assert_ne!(MagicCookie::generate(), MagicCookie::generate());
    }

    #[test]
    fn comparison_is_length_aware() {
        let cookie = MagicCookie::from_hex(HEX).unwrap();
        assert!(cookie.matches(&cookie.bytes()[..]));
        assert!(!cookie.matches(&[0u8; 16]));
        assert!(!cookie.matches(&cookie.bytes()[..8]));
    }

    #[test]
    fn secrets_do_not_appear_in_debug_output() {
        let cookie = MagicCookie::from_hex(HEX).unwrap();
        assert!(!format!("{cookie:?}").contains("5f8c"));

        let request = ForwardRequest::new(false, 0, HEX.to_string());
        let debug = format!("{request:?}");
        assert!(!debug.contains("5f8c"));
        assert!(debug.contains("redacted"));
    }
}
