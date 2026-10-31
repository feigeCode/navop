//! Redis 键名的二进制安全文本表示。
//!
//! Redis 的键是任意字节序列，而本 crate 面向 UI 的接口统一用 `String` 传递键名
//! （见 [`crate::connection::RedisConnection`]）。若直接把服务端返回的键字节按 UTF-8
//! 解析，遇到 JDK 原生序列化对象、MessagePack 之类的二进制键就会整块失败：
//! redis-rs 报 `Cannot convert from UTF-8`（issue #485）。
//!
//! 这里的做法是给每个键一个「显示形态」，真正下发命令前再还原成原始字节：
//!
//! - [`encode`]：字节 → 显示形态。可打印的 UTF-8 键原样返回；其余键返回
//!   `U+0001` 前缀加逐字节 `\xNN` 转义，例如 Java 序列化键 `AC ED 00 05` 显示为
//!   `\u{1}\xac\xed\x00\x05`（前缀不可见，只用于与普通键区分）。
//! - [`decode`]：显示形态 → 字节。普通键恒等（零开销借用），转义键还原真实字节；
//!   转义格式不合法时按字面量处理，避免误伤用户手工输入的键名。
//!
//! 因此列表中显示的键字符串既可以直接当节点 id，也可以原样回传进
//! [`crate::connection::RedisConnection`] 的任意方法，由方法边界上的 [`decode`]
//! 还原成线上字节。

use std::borrow::Cow;

/// 转义形态的私有前缀：U+0001（SOH）。正常键名几乎不会以它开头。
const ESCAPE_PREFIX: char = '\u{1}';

/// 把服务端返回的键原始字节编码成可显示、可还原的字符串。
pub fn encode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) if !text.starts_with(ESCAPE_PREFIX) => text.to_owned(),
        _ => {
            let mut escaped = String::with_capacity(bytes.len() * 4 + 1);
            escaped.push(ESCAPE_PREFIX);
            for byte in bytes {
                escaped.push_str(&format!("\\x{byte:02x}"));
            }
            escaped
        }
    }
}

/// 把 [`encode`] 产生的显示形态还原成真实字节；普通键名原样借回，无额外分配。
pub fn decode(key: &str) -> Cow<'_, [u8]> {
    let Some(escaped) = key.strip_prefix(ESCAPE_PREFIX) else {
        return Cow::Borrowed(key.as_bytes());
    };
    match unescape(escaped) {
        Some(bytes) => Cow::Owned(bytes),
        // 非法转义（用户手输、历史数据）按字面量下发，保证旧行为不变。
        None => Cow::Borrowed(key.as_bytes()),
    }
}

/// 解析 `\xNN` 序列；任一处不合法则整体返回 `None`。
fn unescape(escaped: &str) -> Option<Vec<u8>> {
    let bytes = escaped.as_bytes();
    // 空载荷不是合法转义（空键的显示形态就是空串），按字面量处理。
    if bytes.is_empty() || bytes.len() % 4 != 0 {
        return None;
    }
    let mut decoded = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        if chunk[0] != b'\\' || chunk[1] != b'x' {
            return None;
        }
        let high = hex_digit(chunk[2])?;
        let low = hex_digit(chunk[3])?;
        decoded.push(high << 4 | low);
    }
    Some(decoded)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Java 序列化对象键：非法 UTF-8 字节（issue #485 的实际数据形态）。
    const JAVA_SERIALIZED: &[u8] = &[0xac, 0xed, 0x00, 0x05, 0x74, 0x00, 0x03, 0x6b, 0x65, 0x79];

    #[test]
    fn plain_keys_roundtrip_without_allocation() {
        for key in ["user:1", "普通键", "", "a b\t c"] {
            let encoded = encode(key.as_bytes());
            assert_eq!(key, encoded, "可打印 UTF-8 键应原样显示");
            assert!(
                matches!(decode(&encoded), Cow::Borrowed(_)),
                "普通键应零分配借回"
            );
            assert_eq!(key.as_bytes(), decode(&encoded).as_ref());
        }
    }

    #[test]
    fn binary_keys_roundtrip_through_escaped_text() {
        let encoded = encode(JAVA_SERIALIZED);

        assert!(encoded.starts_with('\u{1}'));
        assert_eq!(
            "\u{1}\\xac\\xed\\x00\\x05\\x74\\x00\\x03\\x6b\\x65\\x79", encoded,
            "非法 UTF-8 键应逐字节转义"
        );
        assert_eq!(JAVA_SERIALIZED, decode(&encoded).as_ref());
    }

    #[test]
    fn keys_starting_with_escape_prefix_stay_unambiguous() {
        let raw = "\u{1}plain".as_bytes();

        let encoded = encode(raw);

        assert_ne!("\u{1}plain", encoded, "以 U+0001 开头的合法键也要转义");
        assert_eq!(raw, decode(&encoded).as_ref());
    }

    #[test]
    fn malformed_escape_is_treated_as_literal_key() {
        for key in ["\u{1}", "\u{1}\\xzz", "\u{1}\\xac\\x", "\u{1}普通"] {
            assert_eq!(key.as_bytes(), decode(key).as_ref());
        }
    }

    #[test]
    fn decode_accepts_uppercase_hex() {
        assert_eq!(
            JAVA_SERIALIZED,
            decode("\u{1}\\xAC\\xED\\x00\\x05\\x74\\x00\\x03\\x6B\\x65\\x79").as_ref()
        );
    }
}
