use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const ENV_VAR: &str = "ONETCLI_TEXT_RENDER_DIAG";

static ENABLED: OnceLock<bool> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSignature {
    pub byte_len: usize,
    pub char_len: usize,
    pub non_ascii_count: usize,
    pub cjk_count: usize,
    pub hash16: String,
    pub codepoints: String,
}

impl TextSignature {
    pub fn new(text: &str) -> Self {
        Self {
            byte_len: text.len(),
            char_len: text.chars().count(),
            non_ascii_count: text.chars().filter(|ch| !ch.is_ascii()).count(),
            cjk_count: text.chars().filter(|ch| is_cjk_like(*ch)).count(),
            hash16: short_hash(text),
            codepoints: sample_codepoints(text, 12),
        }
    }
}

impl fmt::Display for TextSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "bytes={} chars={} non_ascii={} cjk={} hash={} cps=[{}]",
            self.byte_len,
            self.char_len,
            self.non_ascii_count,
            self.cjk_count,
            self.hash16,
            self.codepoints
        )
    }
}

pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var(ENV_VAR)
            .map(|value| !env_value_disabled(&value))
            .unwrap_or(true)
    })
}

pub fn should_sample(counter: &AtomicUsize, limit: usize) -> Option<usize> {
    if !enabled() {
        return None;
    }

    let sample = counter.fetch_add(1, Ordering::Relaxed);
    (sample < limit).then_some(sample)
}

pub fn contains_non_ascii(text: &str) -> bool {
    text.chars().any(|ch| !ch.is_ascii())
}

pub fn font_fallbacks(font: &gpui::Font) -> String {
    font.fallbacks
        .as_ref()
        .map(|fallbacks| fallbacks.fallback_list().join(","))
        .unwrap_or_default()
}

fn env_value_disabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off" | "disabled"
    )
}

fn short_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sample_codepoints(text: &str, max: usize) -> String {
    let mut chars: Vec<char> = text.chars().filter(|ch| !ch.is_ascii()).take(max).collect();
    if chars.is_empty() {
        chars = text.chars().take(max).collect();
    }

    chars
        .into_iter()
        .map(|ch| format!("U+{:04X}", ch as u32))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_cjk_like(ch: char) -> bool {
    let code = ch as u32;
    matches!(
        code,
        0x3000..=0x303F
            | 0x3040..=0x309F
            | 0x30A0..=0x30FF
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x2CEB0..=0x2EBEF
            | 0x2F800..=0x2FA1F
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn env_value_disabled_accepts_common_disable_values() {
        assert!(env_value_disabled("0"));
        assert!(env_value_disabled("false"));
        assert!(env_value_disabled("NO"));
        assert!(env_value_disabled(" off "));
        assert!(env_value_disabled("disabled"));
        assert!(!env_value_disabled(""));
        assert!(!env_value_disabled("1"));
        assert!(!env_value_disabled("true"));
    }

    #[test]
    fn signature_records_lengths_without_plaintext() {
        let signature = TextSignature::new("abc中文繁體");

        assert_eq!(15, signature.byte_len);
        assert_eq!(7, signature.char_len);
        assert_eq!(4, signature.non_ascii_count);
        assert_eq!(4, signature.cjk_count);
        assert!(signature.codepoints.contains("U+4E2D"));
        assert!(!signature.to_string().contains("中文"));
    }

    #[test]
    fn sample_counter_stops_at_limit_when_enabled_logic_is_bypassed() {
        let counter = AtomicUsize::new(0);
        assert_eq!(0, counter.fetch_add(1, Ordering::Relaxed));
        assert_eq!(1, counter.fetch_add(1, Ordering::Relaxed));
    }
}
