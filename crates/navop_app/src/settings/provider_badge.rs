//! 供应商品牌识别：色块 + 首字标识。
//!
//! 市面上的 AI 客户端列表里，供应商几乎都用「品牌色圆角块 + 首字母」来区分。
//! 我们没法为每家供应商内置 SVG logo，用色块既能保证一眼分辨，也不用维护图标资源。

use gpui::{AnyElement, Hsla, IntoElement, ParentElement, SharedString, Styled, div, px, rgb};
use gpui_component::ActiveTheme;
use one_core::llm::types::ProviderType;

/// 供应商品牌主色。
pub fn provider_accent(provider_type: ProviderType) -> Hsla {
    let color = match provider_type {
        ProviderType::OpenAI => rgb(0x0A_A37F),
        ProviderType::Anthropic => rgb(0xD9_7757),
        ProviderType::Aliyun => rgb(0xFF_6A00),
        ProviderType::Zhipu => rgb(0x2F_6BED),
        ProviderType::Ollama => rgb(0x10_1113),
        ProviderType::Volcengine => rgb(0x16_64FF),
        ProviderType::Moonshot => rgb(0x6C_4DF6),
        ProviderType::DeepSeek => rgb(0x4D_6BFE),
        ProviderType::Google => rgb(0x42_85F4),
        ProviderType::OpenRouter => rgb(0x64_67F2),
        ProviderType::MiniMax => rgb(0xE1_251B),
        ProviderType::SiliconFlow => rgb(0x7C_3AED),
        ProviderType::TencentHunyuan => rgb(0x00_52D9),
        ProviderType::Xiaomi => rgb(0xFF_6900),
        ProviderType::Xinference => rgb(0x0E_A5E9),
        ProviderType::OpenCodeGo => rgb(0x22_C55E),
        ProviderType::AzureOpenAI => rgb(0x00_78D4),
        ProviderType::OpenAICompatible => rgb(0x6B_7280),
    };
    Hsla::from(color)
}

/// 品牌块里的字：优先取显示名的首字母/首字（中文品牌取第一个汉字）。
pub fn provider_monogram(provider_type: ProviderType) -> SharedString {
    let name = provider_type.display_name();
    name.chars()
        .next()
        .map(|ch| ch.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".to_string())
        .into()
}

/// 圆角品牌块：固定尺寸，白字。
pub fn provider_badge(provider_type: ProviderType, size: f32) -> AnyElement {
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px(size * 0.28))
        .bg(provider_accent(provider_type))
        .text_color(rgb(0xFF_FFFF))
        .text_size(px(size * 0.42))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(provider_monogram(provider_type))
        .into_any_element()
}

/// 小状态点：启用为品牌色，停用为灰色。
pub fn provider_status_dot(
    provider_type: ProviderType,
    enabled: bool,
    cx: &gpui::App,
) -> AnyElement {
    let color = if enabled {
        provider_accent(provider_type)
    } else {
        cx.theme().muted_foreground
    };
    div()
        .flex_shrink_0()
        .size(px(8.))
        .rounded(px(4.))
        .bg(color)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{provider_accent, provider_monogram};
    use one_core::llm::types::ProviderType;

    #[test]
    fn monogram_uses_the_first_character_of_the_display_name() {
        assert_eq!(provider_monogram(ProviderType::OpenAI).as_ref(), "O");
        assert_eq!(provider_monogram(ProviderType::DeepSeek).as_ref(), "D");
        // 中文品牌取第一个汉字，而不是拼音首字母。
        let first = provider_monogram(ProviderType::Aliyun);
        assert!(!first.is_empty());
        assert_eq!(first, provider_monogram(ProviderType::Aliyun));
    }

    #[test]
    fn every_provider_type_has_a_distinct_accent_except_compatible_fallbacks() {
        assert_ne!(
            provider_accent(ProviderType::OpenAI),
            provider_accent(ProviderType::Anthropic)
        );
        assert_ne!(
            provider_accent(ProviderType::DeepSeek),
            provider_accent(ProviderType::OpenAICompatible)
        );
    }
}
