use gpui::{App, ColorExt as _, Hsla};
use gpui_component::{button::ButtonCustomVariant, highlighter::HighlightTheme};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct WorkspaceTheme {
    pub background: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub muted_foreground: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub accent_foreground: Hsla,
    pub danger: Hsla,
    pub warning: Hsla,
    pub success: Hsla,
}

impl WorkspaceTheme {
    /// Use a translucent accent for list selections. Terminal themes commonly
    /// use a high-contrast cursor color as `accent` (often pure white), which
    /// is appropriate for buttons but too strong as a full-width tree-row
    /// background.
    pub(crate) fn selection_background(&self) -> Hsla {
        self.accent.opacity(0.24)
    }

    pub(crate) fn selection_hover_background(&self) -> Hsla {
        self.accent.opacity(0.32)
    }

    pub(crate) fn highlight_theme(&self) -> Arc<HighlightTheme> {
        if self.background.lightness < 0.5 {
            HighlightTheme::default_dark()
        } else {
            HighlightTheme::default_light()
        }
    }

    pub(crate) fn button_style(&self, cx: &App) -> ButtonCustomVariant {
        ButtonCustomVariant::new(cx)
            .color(self.background)
            .foreground(self.foreground)
            .hover(self.border)
            .active(self.accent)
    }

    pub(crate) fn icon_button_style(&self, cx: &App) -> ButtonCustomVariant {
        ButtonCustomVariant::new(cx)
            .color(self.background.opacity(0.0))
            .foreground(self.foreground)
            .hover(self.muted)
            .active(self.muted)
    }

    pub(crate) fn danger_button_style(&self, cx: &App) -> ButtonCustomVariant {
        ButtonCustomVariant::new(cx)
            .color(self.danger)
            .foreground(self.accent_foreground)
            .hover(self.danger.opacity(0.85))
            .active(self.danger.opacity(0.75))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use palette::IntoColor;

    #[test]
    fn highlight_theme_follows_workspace_background() {
        let dark = WorkspaceTheme {
            background: gpui::rgb(0x111111).into_color(),
            foreground: gpui::rgb(0xeeeeee).into_color(),
            muted: gpui::rgb(0x222222).into_color(),
            muted_foreground: gpui::rgb(0x888888).into_color(),
            border: gpui::rgb(0x333333).into_color(),
            accent: gpui::rgb(0x444444).into_color(),
            accent_foreground: gpui::rgb(0xffffff).into_color(),
            danger: gpui::rgb(0xff0000).into_color(),
            warning: gpui::rgb(0xffaa00).into_color(),
            success: gpui::rgb(0x00aa00).into_color(),
        };
        let mut light = dark;
        light.background = gpui::rgb(0xf5f5f5).into_color();

        assert!(dark.highlight_theme().appearance.is_dark());
        assert!(!light.highlight_theme().appearance.is_dark());
        assert_eq!(dark.selection_background(), dark.accent.opacity(0.24));
        assert_eq!(dark.selection_hover_background(), dark.accent.opacity(0.32));
    }
}
