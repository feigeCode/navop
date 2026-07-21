use gpui::{App, Hsla};
use gpui_component::{
    button::ButtonCustomVariant, highlighter::HighlightTheme, tab::LocalTabStyle,
};
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
    pub(crate) fn highlight_theme(&self) -> Arc<HighlightTheme> {
        if self.background.l < 0.5 {
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

    pub(crate) fn tab_style(&self) -> LocalTabStyle {
        LocalTabStyle {
            bar_background: self.muted,
            background: self.muted,
            foreground: self.muted_foreground,
            hover_background: self.border,
            hover_foreground: self.foreground,
            selected_background: self.background,
            selected_foreground: self.foreground,
            disabled_foreground: self.muted_foreground,
            border: self.border,
            accent: self.accent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_theme_follows_workspace_background() {
        let dark = WorkspaceTheme {
            background: gpui::rgb(0x111111).into(),
            foreground: gpui::rgb(0xeeeeee).into(),
            muted: gpui::rgb(0x222222).into(),
            muted_foreground: gpui::rgb(0x888888).into(),
            border: gpui::rgb(0x333333).into(),
            accent: gpui::rgb(0x444444).into(),
            accent_foreground: gpui::rgb(0xffffff).into(),
            danger: gpui::rgb(0xff0000).into(),
            warning: gpui::rgb(0xffaa00).into(),
            success: gpui::rgb(0x00aa00).into(),
        };
        let mut light = dark;
        light.background = gpui::rgb(0xf5f5f5).into();

        assert!(dark.highlight_theme().appearance.is_dark());
        assert!(!light.highlight_theme().appearance.is_dark());
        assert_eq!(dark.tab_style().selected_background, dark.background);
        assert_eq!(dark.tab_style().bar_background, dark.muted);
    }
}
