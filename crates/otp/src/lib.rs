//! 一次性验证码（TOTP，RFC 6238）解析与生成。
//!
//! 本 crate 只处理算法与文本格式，不依赖 UI 或网络：
//! - 解析用户输入：`otpauth://totp/...` URI（含 `secret`/`digits`/`period`/`algorithm`）
//!   或裸 base32 密钥；
//! - 生成验证码：HMAC-SHA1/SHA256/SHA512 + 动态截断（RFC 4226 §5.3）；
//! - 计算当前时间窗剩余秒数，供界面倒计时展示。
//!
//! 归一化后的密钥以大写、去空白、去 `=` 填充的 base32 保存，便于直接比较与再次序列化。

use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use thiserror::Error;

/// 默认验证码位数（Google Authenticator 等主流实现的默认值）。
pub const DEFAULT_DIGITS: u32 = 6;
/// 默认时间窗长度（秒）。
pub const DEFAULT_PERIOD: u64 = 30;
/// 允许的验证码位数范围。
pub const MIN_DIGITS: u32 = 6;
/// 允许的验证码位数范围。
pub const MAX_DIGITS: u32 = 8;
/// 允许的时间窗长度范围（秒）。
pub const MAX_PERIOD: u64 = 3600;

/// TOTP 哈希算法。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OtpAlgorithm {
    /// HMAC-SHA1，绝大多数服务端与认证器应用使用。
    #[default]
    Sha1,
    /// HMAC-SHA256。
    Sha256,
    /// HMAC-SHA512。
    Sha512,
}

impl OtpAlgorithm {
    /// `otpauth://` 查询参数里使用的规范写法（`SHA1` / `SHA256` / `SHA512`）。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha1 => "SHA1",
            Self::Sha256 => "SHA256",
            Self::Sha512 => "SHA512",
        }
    }
}

impl fmt::Display for OtpAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OtpAlgorithm {
    type Err = OtpError;

    /// 解析算法名：大小写不敏感，兼容 `SHA-1` 这类带连字符的写法。
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_uppercase().replace('-', "");
        match normalized.as_str() {
            "SHA1" => Ok(Self::Sha1),
            "SHA256" => Ok(Self::Sha256),
            "SHA512" => Ok(Self::Sha512),
            _ => Err(OtpError::InvalidAlgorithm),
        }
    }
}

/// 一次性验证码密钥及其参数。
///
/// 字段都带 serde 默认值：旧数据只存了 `secret` 时按 6 位/30 秒/SHA1 解释。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtpSecret {
    /// 归一化后的 base32 密钥（大写、无空白、无 `=` 填充）。
    pub secret: String,
    /// 验证码位数（6..=8）。
    #[serde(default = "default_digits")]
    pub digits: u32,
    /// 时间窗长度（秒，1..=3600）。
    #[serde(default = "default_period")]
    pub period: u64,
    /// 哈希算法。
    #[serde(default)]
    pub algorithm: OtpAlgorithm,
    /// 服务名（`otpauth://` URI 的 issuer）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// 账户名（`otpauth://` URI 标签里 `:` 之后的部分）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

fn default_digits() -> u32 {
    DEFAULT_DIGITS
}

fn default_period() -> u64 {
    DEFAULT_PERIOD
}

impl OtpSecret {
    /// 用裸 base32 密钥构造默认参数（6 位 / 30 秒 / SHA1）的配置。
    pub fn from_base32(secret: &str) -> Result<Self, OtpError> {
        Ok(Self {
            secret: normalize_secret(secret)?,
            digits: DEFAULT_DIGITS,
            period: DEFAULT_PERIOD,
            algorithm: OtpAlgorithm::default(),
            issuer: None,
            account: None,
        })
    }

    /// 解析用户输入：`otpauth://` URI 或裸 base32 密钥。
    ///
    /// 空白会被忽略，因此「粘贴图片」失败后手工粘贴的带空格密钥也能识别。
    pub fn parse(input: &str) -> Result<Self, OtpError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(OtpError::EmptyInput);
        }

        if trimmed.len() >= 8 && trimmed[..8].eq_ignore_ascii_case("otpauth:") {
            return parse_otpauth_uri(trimmed);
        }
        if trimmed.contains("://") {
            let scheme = trimmed.split("://").next().unwrap_or_default();
            return Err(OtpError::UnsupportedScheme(scheme.to_string()));
        }

        Self::from_base32(trimmed)
    }

    /// 校验各参数是否在允许范围内（构造后再次调用也能拦住手改的越界值）。
    pub fn validate(&self) -> Result<(), OtpError> {
        if !(MIN_DIGITS..=MAX_DIGITS).contains(&self.digits) {
            return Err(OtpError::InvalidDigits);
        }
        if self.period == 0 || self.period > MAX_PERIOD {
            return Err(OtpError::InvalidPeriod);
        }
        normalize_secret(&self.secret)?;
        Ok(())
    }

    /// 生成 `unix_seconds` 所在的 TOTP 验证码。
    pub fn generate(&self, unix_seconds: u64) -> Result<String, OtpError> {
        self.validate()?;
        let key = base32_decode(&self.secret)?;
        let counter = unix_seconds / self.period;
        let digest = hmac_digest(self.algorithm, &key, &counter.to_be_bytes())?;

        // RFC 4226 §5.3 动态截断：末字节低 4 位给出偏移，取 4 字节再抹掉符号位。
        let offset = (digest[digest.len() - 1] & 0x0f) as usize;
        let truncated = (u32::from(digest[offset] & 0x7f) << 24)
            | (u32::from(digest[offset + 1]) << 16)
            | (u32::from(digest[offset + 2]) << 8)
            | u32::from(digest[offset + 3]);
        let modulus = 10u32.pow(self.digits);
        Ok(format!(
            "{:0width$}",
            truncated % modulus,
            width = self.digits as usize
        ))
    }

    /// 生成当前时刻的 TOTP 验证码。
    pub fn generate_now(&self) -> Result<String, OtpError> {
        self.generate(current_unix_seconds())
    }

    /// 当前时间窗剩余秒数（1..=period）。
    #[must_use]
    pub fn remaining_seconds(&self, unix_seconds: u64) -> u64 {
        let period = self.period.max(1);
        let elapsed = unix_seconds % period;
        if elapsed == 0 {
            period
        } else {
            period - elapsed
        }
    }

    /// 重新拼出 `otpauth://` URI，供再次编辑时回填输入框。
    #[must_use]
    pub fn otpauth_uri(&self) -> String {
        let label = match (self.issuer.as_deref(), self.account.as_deref()) {
            (Some(issuer), Some(account)) => format!("{issuer}:{account}"),
            (Some(issuer), None) => issuer.to_string(),
            (None, Some(account)) => account.to_string(),
            (None, None) => "Navop".to_string(),
        };
        let mut uri = format!(
            "otpauth://totp/{}?secret={}",
            percent_encode(&label),
            self.secret
        );
        if let Some(issuer) = &self.issuer {
            uri.push_str("&issuer=");
            uri.push_str(&percent_encode(issuer));
        }
        uri.push_str("&algorithm=");
        uri.push_str(self.algorithm.as_str());
        uri.push_str("&digits=");
        uri.push_str(&self.digits.to_string());
        uri.push_str("&period=");
        uri.push_str(&self.period.to_string());
        uri
    }

    /// 参数摘要（如 `6 位 / 30 秒 / SHA1`），用于表单里的校验成功提示。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} 位 / {} 秒 / {}",
            self.digits, self.period, self.algorithm
        )
    }
}

/// 当前 Unix 时间戳（秒）；系统时钟早于 epoch 时返回 0。
#[must_use]
pub fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// 归一化 base32 密钥：去空白与连字符、转大写、去 `=` 填充，并校验字符集。
pub fn normalize_secret(input: &str) -> Result<String, OtpError> {
    let filtered: String = input
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '-')
        .collect();
    let unpadded = filtered.trim_end_matches('=').to_ascii_uppercase();
    if unpadded.is_empty() {
        return Err(OtpError::InvalidBase32);
    }
    if !unpadded
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ('2'..='7').contains(&ch))
    {
        return Err(OtpError::InvalidBase32);
    }
    Ok(unpadded)
}

/// 解码 base32（RFC 4648，无填充）；输入需已归一化。
fn base32_decode(input: &str) -> Result<Vec<u8>, OtpError> {
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    let mut out = Vec::with_capacity(input.len() * 5 / 8);
    for ch in input.chars() {
        let value = match ch {
            'A'..='Z' => ch as u32 - 'A' as u32,
            '2'..='7' => ch as u32 - '2' as u32 + 26,
            _ => return Err(OtpError::InvalidBase32),
        };
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    if out.is_empty() {
        return Err(OtpError::InvalidBase32);
    }
    Ok(out)
}

fn hmac_digest(algorithm: OtpAlgorithm, key: &[u8], message: &[u8]) -> Result<Vec<u8>, OtpError> {
    match algorithm {
        OtpAlgorithm::Sha1 => {
            let mut mac = Hmac::<Sha1>::new_from_slice(key).map_err(|_| OtpError::InvalidBase32)?;
            mac.update(message);
            Ok(mac.finalize().into_bytes().to_vec())
        }
        OtpAlgorithm::Sha256 => {
            let mut mac =
                Hmac::<Sha256>::new_from_slice(key).map_err(|_| OtpError::InvalidBase32)?;
            mac.update(message);
            Ok(mac.finalize().into_bytes().to_vec())
        }
        OtpAlgorithm::Sha512 => {
            let mut mac =
                Hmac::<Sha512>::new_from_slice(key).map_err(|_| OtpError::InvalidBase32)?;
            mac.update(message);
            Ok(mac.finalize().into_bytes().to_vec())
        }
    }
}

fn parse_otpauth_uri(input: &str) -> Result<OtpSecret, OtpError> {
    let rest = &input["otpauth:".len()..];
    let rest = rest.strip_prefix("//").ok_or(OtpError::MalformedUri)?;

    // 先切掉 fragment，再按 `?` 分查询串。
    let rest = rest.split('#').next().unwrap_or_default();
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };

    let (kind, label) = match path.split_once('/') {
        Some((kind, label)) => (kind, Some(label)),
        None => (path, None),
    };
    if kind.eq_ignore_ascii_case("hotp") {
        return Err(OtpError::UnsupportedType(kind.to_string()));
    }
    if !kind.eq_ignore_ascii_case("totp") {
        return Err(OtpError::UnsupportedType(kind.to_string()));
    }

    let label = label.map(percent_decode).unwrap_or_default();
    let label = label.split('?').next().unwrap_or_default().trim();
    let (label_issuer, account) = match label.split_once(':') {
        Some((issuer, account)) => (Some(issuer.trim()), Some(account.trim())),
        None => (None, Some(label)),
    };

    let mut secret: Option<String> = None;
    let mut digits = DEFAULT_DIGITS;
    let mut period = DEFAULT_PERIOD;
    let mut algorithm = OtpAlgorithm::default();
    let mut query_issuer: Option<String> = None;

    for pair in query.unwrap_or_default().split('&') {
        let (key, value) = match pair.split_once('=') {
            Some((key, value)) => (key, value),
            None => continue,
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "secret" => secret = Some(percent_decode(value)),
            "issuer" => {
                let decoded = percent_decode(value);
                if !decoded.trim().is_empty() {
                    query_issuer = Some(decoded.trim().to_string());
                }
            }
            "digits" => {
                let parsed = value
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| OtpError::InvalidDigits)?;
                if !(MIN_DIGITS..=MAX_DIGITS).contains(&parsed) {
                    return Err(OtpError::InvalidDigits);
                }
                digits = parsed;
            }
            "period" => {
                let parsed = value
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| OtpError::InvalidPeriod)?;
                if parsed == 0 || parsed > MAX_PERIOD {
                    return Err(OtpError::InvalidPeriod);
                }
                period = parsed;
            }
            "algorithm" => algorithm = OtpAlgorithm::from_str(value)?,
            _ => {}
        }
    }

    let secret = secret.ok_or(OtpError::MissingSecret)?;
    let issuer = query_issuer
        .or_else(|| label_issuer.map(ToOwned::to_owned))
        .filter(|issuer| !issuer.is_empty());
    let account = account
        .map(ToOwned::to_owned)
        .filter(|account| !account.is_empty());

    let parsed = OtpSecret {
        secret: normalize_secret(&secret)?,
        digits,
        period,
        algorithm,
        issuer,
        account,
    };
    parsed.validate()?;
    Ok(parsed)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        // `%XX` 只在两个后继字节都是 ASCII 十六进制时才算转义，避免切裂多字节字符。
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                out.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        if bytes[index] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[index]);
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// 一次性验证码解析/生成错误。界面按变体映射到本地化文案。
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum OtpError {
    /// 输入为空。
    #[error("one-time password input is empty")]
    EmptyInput,
    /// 不是 `otpauth://` 链接。
    #[error("unsupported one-time password URI scheme: {0}")]
    UnsupportedScheme(String),
    /// 只支持基于时间的 TOTP，不支持 HOTP。
    #[error("only TOTP is supported, got {0}")]
    UnsupportedType(String),
    /// `otpauth://` 缺少 `secret` 参数。
    #[error("otpauth URI is missing the secret parameter")]
    MissingSecret,
    /// 密钥不是合法 base32。
    #[error("secret is not valid base32")]
    InvalidBase32,
    /// 验证码位数越界。
    #[error("digits must be between 6 and 8")]
    InvalidDigits,
    /// 时间窗长度越界。
    #[error("period must be between 1 and 3600 seconds")]
    InvalidPeriod,
    /// 哈希算法无法识别。
    #[error("unsupported TOTP algorithm")]
    InvalidAlgorithm,
    /// URI 结构损坏。
    #[error("otpauth URI is malformed")]
    MalformedUri,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用 base32 编码（RFC 4648，无填充），用于构造 RFC 6238 的种子。
    fn base32_encode(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
        let mut out = String::new();
        let mut buffer: u32 = 0;
        let mut bits: u32 = 0;
        for byte in bytes {
            buffer = (buffer << 8) | u32::from(*byte);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                out.push(ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
            }
            buffer &= (1 << bits) - 1;
        }
        if bits > 0 {
            out.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
        }
        out
    }

    fn rfc_secret(algorithm: OtpAlgorithm) -> OtpSecret {
        let seed = match algorithm {
            OtpAlgorithm::Sha1 => "12345678901234567890".to_string(),
            OtpAlgorithm::Sha256 => "12345678901234567890123456789012".to_string(),
            OtpAlgorithm::Sha512 => {
                "1234567890123456789012345678901234567890123456789012345678901234".to_string()
            }
        };
        OtpSecret {
            secret: base32_encode(seed.as_bytes()),
            digits: 8,
            period: 30,
            algorithm,
            issuer: None,
            account: None,
        }
    }

    #[test]
    fn base32_round_trips_known_value() {
        assert_eq!(base32_decode("GEZDGNBVGY3TQOJQ").unwrap(), b"1234567890");
        assert_eq!(base32_encode(b"1234567890"), "GEZDGNBVGY3TQOJQ");
        // 归一化：小写、空格、连字符、填充都要被吃掉。
        assert_eq!(
            normalize_secret("gezd gnbv-gy3t qojq====").unwrap(),
            "GEZDGNBVGY3TQOJQ"
        );
    }

    #[test]
    fn rejects_invalid_base32() {
        assert_eq!(normalize_secret(""), Err(OtpError::InvalidBase32));
        assert_eq!(normalize_secret("ABC1"), Err(OtpError::InvalidBase32));
        assert_eq!(normalize_secret("ABC0"), Err(OtpError::InvalidBase32));
        assert_eq!(normalize_secret("==="), Err(OtpError::InvalidBase32));
    }

    #[test]
    fn matches_rfc_6238_sha1_vectors() {
        let secret = rfc_secret(OtpAlgorithm::Sha1);
        let vectors = [
            (59, "94287082"),
            (1_111_111_109, "07081804"),
            (1_111_111_111, "14050471"),
            (1_234_567_890, "89005924"),
            (2_000_000_000, "69279037"),
            (20_000_000_000, "65353130"),
        ];
        for (unix_seconds, expected) in vectors {
            assert_eq!(
                secret.generate(unix_seconds).unwrap(),
                expected,
                "SHA1 seed at {unix_seconds}"
            );
        }
    }

    #[test]
    fn matches_rfc_6238_sha256_vectors() {
        let secret = rfc_secret(OtpAlgorithm::Sha256);
        let vectors = [
            (59, "46119246"),
            (1_111_111_109, "68084774"),
            (1_111_111_111, "67062674"),
            (1_234_567_890, "91819424"),
            (2_000_000_000, "90698825"),
            (20_000_000_000, "77737706"),
        ];
        for (unix_seconds, expected) in vectors {
            assert_eq!(
                secret.generate(unix_seconds).unwrap(),
                expected,
                "SHA256 seed at {unix_seconds}"
            );
        }
    }

    #[test]
    fn matches_rfc_6238_sha512_vectors() {
        let secret = rfc_secret(OtpAlgorithm::Sha512);
        let vectors = [
            (59, "90693936"),
            (1_111_111_109, "25091201"),
            (1_111_111_111, "99943326"),
            (1_234_567_890, "93441116"),
            (2_000_000_000, "38618901"),
            (20_000_000_000, "47863826"),
        ];
        for (unix_seconds, expected) in vectors {
            assert_eq!(
                secret.generate(unix_seconds).unwrap(),
                expected,
                "SHA512 seed at {unix_seconds}"
            );
        }
    }

    #[test]
    fn parses_otpauth_uri_with_issuer_and_parameters() {
        let parsed = OtpSecret::parse(
            "otpauth://totp/ACME%20Co:john@example.com?secret=GEZDGNBVGY3TQOJQ&issuer=ACME%20Co&algorithm=SHA256&digits=8&period=60",
        )
        .unwrap();
        assert_eq!(parsed.secret, "GEZDGNBVGY3TQOJQ");
        assert_eq!(parsed.digits, 8);
        assert_eq!(parsed.period, 60);
        assert_eq!(parsed.algorithm, OtpAlgorithm::Sha256);
        assert_eq!(parsed.issuer.as_deref(), Some("ACME Co"));
        assert_eq!(parsed.account.as_deref(), Some("john@example.com"));
    }

    #[test]
    fn parses_plain_base32_input() {
        let parsed = OtpSecret::parse("  gezd gnbv gy3t qojq  ").unwrap();
        assert_eq!(parsed.secret, "GEZDGNBVGY3TQOJQ");
        assert_eq!(parsed.digits, DEFAULT_DIGITS);
        assert_eq!(parsed.period, DEFAULT_PERIOD);
        assert_eq!(parsed.algorithm, OtpAlgorithm::Sha1);
        assert!(parsed.issuer.is_none());
    }

    #[test]
    fn derivation_and_summary_round_trip() {
        let parsed = OtpSecret::parse(
            "otpauth://totp/Navop:dev%40example.com?secret=GEZDGNBVGY3TQOJQ&issuer=Navop&digits=7&period=45&algorithm=SHA512",
        )
        .unwrap();
        assert_eq!(parsed.summary(), "7 位 / 45 秒 / SHA512");
        let uri = parsed.otpauth_uri();
        assert_eq!(OtpSecret::parse(&uri).unwrap(), parsed);
    }

    #[test]
    fn rejects_unsupported_inputs() {
        assert_eq!(OtpSecret::parse("   "), Err(OtpError::EmptyInput));
        assert_eq!(
            OtpSecret::parse("https://example.com/x"),
            Err(OtpError::UnsupportedScheme("https".to_string()))
        );
        assert!(matches!(
            OtpSecret::parse("otpauth://hotp/x?secret=GEZDGNBVGY3TQOJQ&counter=1"),
            Err(OtpError::UnsupportedType(_))
        ));
        assert_eq!(
            OtpSecret::parse("otpauth://totp/x?issuer=ACME"),
            Err(OtpError::MissingSecret)
        );
        assert_eq!(
            OtpSecret::parse("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&digits=9"),
            Err(OtpError::InvalidDigits)
        );
        assert_eq!(
            OtpSecret::parse("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&period=0"),
            Err(OtpError::InvalidPeriod)
        );
        assert_eq!(
            OtpSecret::parse("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&algorithm=MD5"),
            Err(OtpError::InvalidAlgorithm)
        );
        assert_eq!(
            OtpSecret::parse("otpauth://totp/x?secret=ABC!"),
            Err(OtpError::InvalidBase32)
        );
    }

    #[test]
    fn remaining_seconds_counts_down_within_window() {
        let secret = OtpSecret::from_base32("GEZDGNBVGY3TQOJQ").unwrap();
        assert_eq!(secret.remaining_seconds(0), 30);
        assert_eq!(secret.remaining_seconds(1), 29);
        assert_eq!(secret.remaining_seconds(29), 1);
        assert_eq!(secret.remaining_seconds(30), 30);
    }

    #[test]
    fn serializes_with_and_without_optional_parameters() {
        let secret = OtpSecret::from_base32("GEZDGNBVGY3TQOJQ").unwrap();
        let json = serde_json::to_string(&secret).unwrap();
        assert_eq!(
            json,
            r#"{"secret":"GEZDGNBVGY3TQOJQ","digits":6,"period":30,"algorithm":"SHA1"}"#
        );
        // 旧数据只有 secret 时按默认参数补齐。
        let parsed: OtpSecret = serde_json::from_str(r#"{"secret":"GEZDGNBVGY3TQOJQ"}"#).unwrap();
        assert_eq!(parsed.digits, DEFAULT_DIGITS);
        assert_eq!(parsed.period, DEFAULT_PERIOD);
        assert_eq!(parsed.algorithm, OtpAlgorithm::Sha1);
    }

    #[test]
    fn algorithm_names_are_case_insensitive() {
        assert_eq!(
            OtpAlgorithm::from_str("sha-256").unwrap(),
            OtpAlgorithm::Sha256
        );
        assert_eq!(OtpAlgorithm::Sha512.to_string(), "SHA512");
    }
}
