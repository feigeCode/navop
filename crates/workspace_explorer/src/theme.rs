use gpui::{App, Hsla};
use gpui_component::{
    button::ButtonCustomVariant, highlighter::HighlightTheme, input::EditorStyleOverrides,
};
use one_ui::StatusBarColors;
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
        if self.background.l < 0.5 {
            HighlightTheme::default_dark()
        } else {
            HighlightTheme::default_light()
        }
    }

    /// The palette the code editor paints with.
    ///
    /// An editor is a surface inside this workspace, so it takes its whole
    /// palette from here rather than from the application theme: one surface,
    /// one set of colours. The gutter and the active line do not follow
    /// `background` on their own, so they are named explicitly — leaving them
    /// out is what draws a bright margin beside dark text.
    pub(crate) fn editor_style(&self) -> EditorStyleOverrides {
        EditorStyleOverrides {
            foreground: Some(self.foreground),
            muted_foreground: Some(self.muted_foreground),
            background: Some(self.background),
            border: Some(self.border),
            selection: Some(self.selection_background()),
            caret: Some(self.accent),
            highlight_styles: Some(self.highlight_theme()),
            editor_active_line: Some(self.muted),
            editor_gutter_background: Some(self.muted),
            editor_invisible: Some(self.muted_foreground),
        }
    }

    /// The palette the editor's status bar paints with.
    ///
    /// Same reasoning as [`Self::editor_style`]: the bar sits inside this
    /// workspace, so it takes the workspace's surfaces instead of leaving a
    /// strip of the application theme's chrome under the content.
    pub(crate) fn status_bar_colors(&self) -> StatusBarColors {
        StatusBarColors {
            background: Some(self.background),
            border: Some(self.border),
            muted_foreground: Some(self.muted_foreground),
            info: Some(self.accent),
            success: Some(self.success),
            warning: Some(self.warning),
            danger: Some(self.danger),
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
        assert_eq!(dark.selection_background(), dark.accent.opacity(0.24));
        assert_eq!(dark.selection_hover_background(), dark.accent.opacity(0.32));
    }

    /// A dark workspace theme, the shape a terminal preset produces.
    fn workspace_theme() -> WorkspaceTheme {
        WorkspaceTheme {
            background: gpui::rgb(0x0a0e14).into(),
            foreground: gpui::rgb(0x00d9ff).into(),
            muted: gpui::rgb(0x141922).into(),
            muted_foreground: gpui::rgb(0x7788aa).into(),
            border: gpui::rgb(0x223344).into(),
            accent: gpui::rgb(0x00d9ff).into(),
            accent_foreground: gpui::rgb(0x000000).into(),
            danger: gpui::rgb(0xff0000).into(),
            warning: gpui::rgb(0xffaa00).into(),
            success: gpui::rgb(0x00aa00).into(),
        }
    }

    #[test]
    fn the_editor_style_takes_its_surface_from_the_workspace() {
        let theme = workspace_theme();

        let style = theme.editor_style();

        assert_eq!(style.background, Some(theme.background));
        assert_eq!(style.foreground, Some(theme.foreground));
        assert_eq!(style.border, Some(theme.border));
        // The gutter and the active line do not follow `background` on their
        // own, so a theme that hands the editor its colours must name them:
        // leaving them out paints a bright margin beside dark text.
        assert_eq!(style.editor_gutter_background, Some(theme.muted));
        assert_eq!(style.editor_active_line, Some(theme.muted));
        assert_eq!(style.editor_invisible, Some(theme.muted_foreground));
        assert!(style.highlight_styles.is_some());
    }

    #[test]
    fn the_status_bar_takes_its_surface_from_the_workspace() {
        let theme = workspace_theme();

        let colors = theme.status_bar_colors();

        assert_eq!(colors.background, Some(theme.background));
        assert_eq!(colors.border, Some(theme.border));
        assert_eq!(colors.muted_foreground, Some(theme.muted_foreground));
        assert_eq!(colors.warning, Some(theme.warning));
    }
}
