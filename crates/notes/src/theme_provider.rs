use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use cditor_app::{ThemeProvider, gui::GuiTheme};
use gpui::{Hsla, Rgba};

pub(crate) struct NavopThemeProvider {
    theme: RwLock<GuiTheme>,
    revision: AtomicU64,
}

impl NavopThemeProvider {
    pub(crate) fn new(theme: GuiTheme) -> Self {
        Self {
            theme: RwLock::new(theme),
            revision: AtomicU64::new(0),
        }
    }

    pub(crate) fn refresh(&self, theme: GuiTheme) -> bool {
        let mut current = self.theme.write().expect("notes theme state poisoned");
        if *current == theme {
            return false;
        }
        *current = theme;
        self.revision.fetch_add(1, Ordering::AcqRel);
        true
    }
}

impl ThemeProvider for NavopThemeProvider {
    fn theme(&self) -> GuiTheme {
        *self.theme.read().expect("notes theme state poisoned")
    }

    fn version(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
}

pub(crate) fn cditor_theme(
    background: Hsla,
    foreground: Hsla,
    muted: Hsla,
    border: Hsla,
    primary: Hsla,
    danger: Hsla,
) -> GuiTheme {
    let background = rgb24(background);
    let foreground = rgb24(foreground);
    let muted = rgb24(muted);
    let border = rgb24(border);
    let primary = rgb24(primary);
    let danger = rgb24(danger);
    GuiTheme {
        surface: background,
        page: background,
        panel: background,
        text: foreground,
        muted,
        border,
        strong_border: border,
        focused: primary,
        hover_surface: blend(background, foreground, 0.08),
        action_background: blend(background, primary, 0.18),
        action_hover_background: blend(background, foreground, 0.1),
        action_accent: primary,
        gutter_background: background,
        gutter_foreground: muted,
        prefix_text: foreground,
        quote_text: foreground,
        quote_bar: muted,
        callout_background: blend(background, foreground, 0.06),
        callout_border: border,
        callout_icon_background: blend(background, foreground, 0.1),
        checkbox_border: muted,
        checkbox_checked_background: primary,
        checkbox_checked_text: background,
        code_background: blend(background, foreground, 0.055),
        code_text: foreground,
        inline_code_background: blend(background, foreground, 0.09),
        inline_code_text: foreground,
        code_toolbar_background: blend(background, foreground, 0.035),
        code_toolbar_border: border,
        code_toolbar_text: muted,
        code_toolbar_icon: muted,
        code_toolbar_hover: blend(background, foreground, 0.1),
        table_header_background: blend(background, foreground, 0.055),
        table_active_border: primary,
        skeleton: blend(background, foreground, 0.08),
        danger,
        scrollbar: blend(background, foreground, 0.22),
        scrollbar_hover: blend(background, foreground, 0.35),
    }
}

fn rgb24(color: Hsla) -> u32 {
    let color = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    (channel(color.r) << 16) | (channel(color.g) << 8) | channel(color.b)
}

fn blend(base: u32, overlay: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let base = ((base >> shift) & 0xffu32) as f32;
        let overlay = ((overlay >> shift) & 0xffu32) as f32;
        (base + (overlay - base) * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgb;

    #[test]
    fn editor_surface_and_text_follow_the_host_theme() {
        let theme = cditor_theme(
            rgb(0x111111).into(),
            rgb(0xeeeeee).into(),
            rgb(0x999999).into(),
            rgb(0x333333).into(),
            rgb(0x4488ff).into(),
            rgb(0xff4455).into(),
        );

        assert_eq!(theme.page, 0x111111);
        assert_eq!(theme.surface, 0x111111);
        assert_eq!(theme.text, 0xeeeeee);
        assert_eq!(theme.border, 0x333333);
        assert_eq!(theme.action_accent, 0x4488ff);
        assert_ne!(theme.code_background, 0xf7f6f3);
    }
}
