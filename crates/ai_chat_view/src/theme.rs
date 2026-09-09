use std::cell::RefCell;

use gpui::{App, ColorExt, ElementId, HighlightStyle, Hsla, SharedString, StyleRefinement};
use gpui_component::{
    ActiveTheme,
    text::{TextView, TextViewStyle},
};

#[derive(Clone, Debug)]
pub struct AgentChatTheme {
    pub is_dark: bool,
    pub background: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub muted_foreground: Hsla,
    pub border: Hsla,
    pub panel: Hsla,
    pub panel_hover: Hsla,
    pub accent: Hsla,
    pub accent_foreground: Hsla,
    pub code_background: Hsla,
    pub code_foreground: Hsla,
    pub table_header: Hsla,
    pub table_row: Hsla,
    pub table_row_alt: Hsla,
    pub quote_border: Hsla,
    pub link: Hsla,
}

impl AgentChatTheme {
    pub fn from_app(cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            is_dark: theme.is_dark(),
            background: theme.background,
            foreground: theme.foreground,
            muted: theme.muted,
            muted_foreground: theme.muted_foreground,
            border: theme.border,
            panel: theme.muted,
            panel_hover: theme.muted.opacity(0.72),
            accent: theme.accent,
            accent_foreground: theme.accent_foreground,
            code_background: theme.muted,
            code_foreground: theme.foreground,
            table_header: theme.muted,
            table_row: theme.background,
            table_row_alt: theme.muted.opacity(0.35),
            quote_border: theme.border,
            link: theme.primary,
        }
    }

    pub fn markdown_style(&self) -> TextViewStyle {
        let mut code_block = StyleRefinement::default();
        code_block.background = Some(self.code_background.into());
        code_block.text.color = Some(self.code_foreground);
        let mut table_head = StyleRefinement::default();
        table_head.background = Some(self.table_header.into());
        table_head.text.color = Some(self.foreground);
        let mut table_cell = StyleRefinement::default();
        table_cell.background = Some(self.table_row.into());
        let mut style = TextViewStyle::default()
            .code_block(code_block)
            .inline_code(HighlightStyle {
                color: Some(self.code_foreground),
                background_color: Some(self.code_background),
                ..Default::default()
            })
            .table_head(table_head)
            .table_cell(table_cell);
        style.is_dark = self.is_dark;
        style
    }

    pub fn hover_background(&self) -> Hsla {
        if self.is_dark {
            self.panel_hover
        } else {
            self.accent.opacity(0.14)
        }
    }

    pub fn selection_background(&self) -> Hsla {
        self.accent.opacity(if self.is_dark { 0.22 } else { 0.30 })
    }
}

thread_local! {
    static ACTIVE_AGENT_CHAT_THEME: RefCell<Option<AgentChatTheme>> = const { RefCell::new(None) };
}

pub(crate) fn resolve_agent_chat_theme(theme: Option<&AgentChatTheme>, cx: &App) -> AgentChatTheme {
    theme
        .cloned()
        .unwrap_or_else(|| AgentChatTheme::from_app(cx))
}

pub(crate) fn active_agent_chat_theme(cx: &App) -> AgentChatTheme {
    ACTIVE_AGENT_CHAT_THEME
        .with(|theme| theme.borrow().clone())
        .unwrap_or_else(|| AgentChatTheme::from_app(cx))
}

pub(crate) fn with_agent_chat_theme<T>(theme: &AgentChatTheme, render: impl FnOnce() -> T) -> T {
    let previous = ACTIVE_AGENT_CHAT_THEME.with(|active| active.replace(Some(theme.clone())));
    let output = render();
    ACTIVE_AGENT_CHAT_THEME.with(|active| {
        active.replace(previous);
    });
    output
}

pub(crate) fn themed_markdown(
    id: impl Into<ElementId>,
    markdown: impl Into<SharedString>,
    theme: &AgentChatTheme,
) -> TextView {
    TextView::markdown(id, markdown).style(theme.markdown_style())
}

#[cfg(test)]
mod tests {
    use gpui::rgb;
    use palette::IntoColor as _;

    use super::*;

    fn color(hex: u32) -> Hsla {
        rgb(hex).into_color()
    }

    fn dark_theme() -> AgentChatTheme {
        AgentChatTheme {
            is_dark: true,
            background: color(0x020617),
            foreground: color(0xf8fafc),
            muted: color(0x0f172a),
            muted_foreground: color(0x94a3b8),
            border: color(0x334155),
            panel: color(0x0f172a),
            panel_hover: color(0x1e293b),
            accent: color(0x38bdf8),
            accent_foreground: color(0x001018),
            code_background: color(0x020617),
            code_foreground: color(0xe2e8f0),
            table_header: color(0x1e293b),
            table_row: color(0x020617),
            table_row_alt: color(0x111827),
            quote_border: color(0x475569),
            link: color(0x38bdf8),
        }
    }

    #[test]
    fn markdown_style_preserves_agent_chat_palette() {
        let theme = dark_theme();
        let style = theme.markdown_style();

        assert!(style.is_dark);
        assert_eq!(
            Some(theme.code_background.into()),
            style.code_block.background
        );
        assert_eq!(Some(theme.code_foreground), style.code_block.text.color);
        assert_eq!(
            Some(theme.code_background),
            style.inline_code.background_color
        );
        assert_eq!(Some(theme.code_foreground), style.inline_code.color);
        assert_eq!(Some(theme.table_header.into()), style.table_head.background);
        assert_eq!(Some(theme.table_row.into()), style.table_cell.background);
    }
}
