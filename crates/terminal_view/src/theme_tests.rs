use super::{
    APPLICATION_THEME_NAME, TerminalTheme, available_monospace_fonts, default_font_fallbacks,
    default_monospace_font, normalize_terminal_primary_font, terminal_cell_width_from_advance,
    terminal_cell_width_from_advances,
};
use gpui::{Pixels, px};
use gpui_component::{Theme, ThemeColor};

#[test]
fn dark_terminal_theme_reuses_application_semantic_colors() {
    let app_theme = Theme::from(ThemeColor::dark().as_ref());
    let terminal_theme = TerminalTheme::from_application_theme(&app_theme);

    assert_eq!(app_theme.background, terminal_theme.background);
    assert_eq!(app_theme.foreground, terminal_theme.foreground);
    assert_eq!(app_theme.primary, terminal_theme.cursor);
    assert_eq!(app_theme.selection, terminal_theme.selection);
}

#[test]
fn light_terminal_theme_softens_the_canvas_and_default_text() {
    let app_theme = Theme::from(ThemeColor::light().as_ref());
    let terminal_theme = TerminalTheme::from_application_theme(&app_theme);

    assert!(terminal_theme.background.lightness < app_theme.background.lightness);
    assert!(terminal_theme.background.lightness >= 0.97);
    assert!(terminal_theme.foreground.lightness > app_theme.foreground.lightness);
    assert!(terminal_theme.foreground.lightness <= 0.32);
    assert_eq!(app_theme.primary, terminal_theme.cursor);
    assert_eq!(app_theme.selection, terminal_theme.selection);
}

#[test]
fn application_terminal_theme_tracks_application_colors() {
    let dark_app_theme = Theme::from(ThemeColor::dark().as_ref());
    let light_app_theme = Theme::from(ThemeColor::light().as_ref());

    let dark_terminal_theme = TerminalTheme::resolve(APPLICATION_THEME_NAME, &dark_app_theme);
    let light_terminal_theme = TerminalTheme::resolve(APPLICATION_THEME_NAME, &light_app_theme);

    assert_eq!(APPLICATION_THEME_NAME, dark_terminal_theme.name);
    assert_eq!(APPLICATION_THEME_NAME, light_terminal_theme.name);
    assert_ne!(dark_terminal_theme, light_terminal_theme);
}

#[test]
fn fixed_terminal_theme_does_not_depend_on_application_theme() {
    let dark_app_theme = Theme::from(ThemeColor::dark().as_ref());
    let light_app_theme = Theme::from(ThemeColor::light().as_ref());

    assert_eq!(
        TerminalTheme::resolve("ocean", &dark_app_theme),
        TerminalTheme::resolve("ocean", &light_app_theme)
    );
}

#[test]
fn unknown_terminal_theme_falls_back_to_application() {
    let app_theme = Theme::from(ThemeColor::dark().as_ref());

    assert_eq!(
        TerminalTheme::from_application_theme(&app_theme),
        TerminalTheme::resolve("not-a-theme", &app_theme)
    );
    assert_eq!(
        TerminalTheme::from_application_theme(&app_theme),
        TerminalTheme::resolve("   ", &app_theme)
    );
}

#[test]
fn all_terminal_theme_names_are_stable_and_resolvable() {
    let app_theme = Theme::from(ThemeColor::dark().as_ref());
    let expected_names = [
        APPLICATION_THEME_NAME,
        "midnight",
        "daylight",
        "ink",
        "paper",
        "ocean",
        "obsidian",
        "lotus",
        "neon_blue",
        "matrix",
        "crimson",
        "slate",
        "aurora",
        "orchid",
        "ember",
        "sandstone",
        "frost",
    ];
    let themes = TerminalTheme::all(&app_theme);

    assert_eq!(expected_names.len(), themes.len());
    assert_eq!(
        expected_names,
        themes
            .iter()
            .map(|theme| theme.name)
            .collect::<Vec<_>>()
            .as_slice()
    );
    for name in expected_names {
        assert_eq!(
            name,
            TerminalTheme::find_by_name(name, &app_theme)
                .expect("内置终端主题应可按名称解析")
                .name
        );
    }
}

#[test]
fn every_terminal_palette_keeps_primary_and_secondary_text_separated_from_backgrounds() {
    let app_theme = Theme::from(ThemeColor::dark().as_ref());

    for theme in TerminalTheme::all(&app_theme) {
        let colors = theme.colors();
        assert!(
            lightness_distance(colors.foreground, colors.background) >= 0.3,
            "{} foreground is too close to the background",
            theme.name
        );
        assert!(
            lightness_distance(colors.muted_foreground, colors.background) >= 0.25,
            "{} muted foreground is too close to the background",
            theme.name
        );
        assert!(
            lightness_distance(colors.foreground, colors.muted) >= 0.25,
            "{} foreground is too close to the muted surface",
            theme.name
        );
    }
}

fn lightness_distance(left: gpui::Hsla, right: gpui::Hsla) -> f32 {
    (left.lightness - right.lightness).abs()
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
