//! Locale normalization for the editor's `rust-i18n` catalog.

/// Maps Navop/system locale identifiers to the locales bundled by this crate.
pub(crate) fn normalize_locale(locale: &str) -> &'static str {
    let normalized = locale.trim().replace('_', "-").to_ascii_lowercase();
    if normalized == "zh"
        || normalized.starts_with("zh-cn")
        || normalized.starts_with("zh-sg")
        || normalized.starts_with("zh-hans")
    {
        "zh-CN"
    } else if normalized.starts_with("zh-hk")
        || normalized.starts_with("zh-mo")
        || normalized.starts_with("zh-tw")
        || normalized.starts_with("zh-hant")
    {
        "zh-HK"
    } else {
        "en"
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_locale;

    #[test]
    fn normalizes_supported_chinese_locale_families() {
        assert_eq!(normalize_locale("zh-CN"), "zh-CN");
        assert_eq!(normalize_locale("zh_Hans_SG"), "zh-CN");
        assert_eq!(normalize_locale("zh-HK"), "zh-HK");
        assert_eq!(normalize_locale("zh_Hant_TW"), "zh-HK");
    }

    #[test]
    fn falls_back_to_english() {
        assert_eq!(normalize_locale("en-US"), "en");
        assert_eq!(normalize_locale("fr-FR"), "en");
        assert_eq!(normalize_locale(""), "en");
    }
}
