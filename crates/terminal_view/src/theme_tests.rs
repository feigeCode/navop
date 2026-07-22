use super::{
    TerminalTheme, available_monospace_fonts, default_font_fallbacks, default_monospace_font,
    normalize_terminal_primary_font, terminal_cell_width_from_advance,
    terminal_cell_width_from_advances,
};
use gpui::{Pixels, px};
use gpui_component::{Theme, ThemeColor};

#[test]
fn application_theme_reuses_application_semantic_colors() {
    let app_theme = Theme::from(ThemeColor::dark().as_ref());
    let terminal_theme = TerminalTheme::from_application_theme(&app_theme);

    assert_eq!(app_theme.background, terminal_theme.background);
    assert_eq!(app_theme.foreground, terminal_theme.foreground);
    assert_eq!(app_theme.primary, terminal_theme.cursor);
    assert_eq!(app_theme.selection, terminal_theme.selection);
}

#[test]
fn terminal_default_fallbacks_put_cjk_before_emoji_and_symbols() {
    let fallbacks = default_font_fallbacks()
        .into_iter()
        .map(|font| font.to_string())
        .collect::<Vec<_>>();

    for cjk_font in ["PingFang SC", "Noto Sans CJK SC", "Noto Sans Mono CJK SC"] {
        if let Some(cjk_index) = fallbacks.iter().position(|font| font == cjk_font) {
            for symbol_font in ["Apple Color Emoji", "Apple Symbols", "Noto Color Emoji"] {
                if let Some(symbol_index) = fallbacks.iter().position(|font| font == symbol_font) {
                    assert!(cjk_index < symbol_index);
                }
            }
        }
    }
}

#[test]
fn terminal_primary_font_options_exclude_fallback_only_cjk_fonts() {
    let fonts = available_monospace_fonts();

    assert!(!fonts.contains(&"Noto Sans Mono CJK SC"));
    assert!(!fonts.contains(&"Source Han Mono SC"));
}

#[test]
fn terminal_primary_font_normalizes_fallback_only_cjk_fonts() {
    for font in [
        "Noto Sans Mono CJK SC",
        "Source Han Mono SC",
        "PingFang SC",
        "Microsoft YaHei",
        "SimSun",
        "Apple Color Emoji",
    ] {
        assert_eq!(
            default_monospace_font(),
            normalize_terminal_primary_font(font)
        );
    }
    assert_eq!(
        "JetBrains Mono",
        normalize_terminal_primary_font("JetBrains Mono")
    );
}

#[test]
fn terminal_cell_width_keeps_measured_width_unless_extreme() {
    fn assert_px_close(expected: Pixels, actual: Pixels) {
        let expected = f32::from(expected);
        let actual = f32::from(actual);
        assert!((expected - actual).abs() < 0.001);
    }

    assert_px_close(
        px(14.0),
        terminal_cell_width_from_advance(px(14.0), px(14.0)),
    );
    assert_px_close(
        px(8.4),
        terminal_cell_width_from_advance(px(14.0), px(20.0)),
    );
    assert_px_close(px(8.4), terminal_cell_width_from_advance(px(14.0), px(2.0)));
    assert_px_close(px(8.0), terminal_cell_width_from_advance(px(14.0), px(8.0)));
}

#[test]
fn terminal_cell_width_uses_widest_representative_advance() {
    fn assert_px_close(expected: Pixels, actual: Pixels) {
        let expected = f32::from(expected);
        let actual = f32::from(actual);
        assert!((expected - actual).abs() < 0.001);
    }

    assert_px_close(
        px(10.0),
        terminal_cell_width_from_advances(px(14.0), [px(8.0), px(10.0), px(9.0)]),
    );
    assert_px_close(
        px(8.4),
        terminal_cell_width_from_advances(px(14.0), std::iter::empty()),
    );
}
