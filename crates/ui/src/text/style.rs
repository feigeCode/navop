use std::sync::Arc;

use gpui::{Hsla, Pixels, Rems, StyleRefinement, px, rems};

use crate::highlighter::HighlightTheme;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkdownPalette {
    pub is_dark: bool,
    pub foreground: Hsla,
    pub muted_foreground: Hsla,
    pub border: Hsla,
    pub code_background: Hsla,
    pub code_foreground: Hsla,
    pub table_header: Hsla,
    pub table_row: Hsla,
    pub table_row_alt: Hsla,
    pub quote_border: Hsla,
    pub link: Hsla,
}

/// TextViewStyle used to customize the style for [`TextView`].
#[derive(Clone)]
pub struct TextViewStyle {
    /// Gap of each paragraphs, default is 1 rem.
    pub paragraph_gap: Rems,
    /// Base font size for headings, default is 14px.
    pub heading_base_font_size: Pixels,
    /// Function to calculate heading font size based on heading level (1-6).
    ///
    /// The first parameter is the heading level (1-6), the second parameter is the base font size.
    /// The second parameter is the base font size.
    pub heading_font_size: Option<Arc<dyn Fn(u8, Pixels) -> Pixels + Send + Sync + 'static>>,
    /// Highlight theme for code blocks. Default: [`HighlightTheme::default_light()`]
    pub highlight_theme: Arc<HighlightTheme>,
    /// The style refinement for code blocks.
    pub code_block: StyleRefinement,
    pub is_dark: bool,
    pub foreground: Option<Hsla>,
    pub muted_foreground: Option<Hsla>,
    pub border: Option<Hsla>,
    pub code_background: Option<Hsla>,
    pub code_foreground: Option<Hsla>,
    pub table_header: Option<Hsla>,
    pub table_row: Option<Hsla>,
    pub table_row_alt: Option<Hsla>,
    pub quote_border: Option<Hsla>,
    pub link: Option<Hsla>,
}

impl PartialEq for TextViewStyle {
    fn eq(&self, other: &Self) -> bool {
        self.paragraph_gap == other.paragraph_gap
            && self.heading_base_font_size == other.heading_base_font_size
            && self.highlight_theme == other.highlight_theme
            && self.is_dark == other.is_dark
            && self.foreground == other.foreground
            && self.muted_foreground == other.muted_foreground
            && self.border == other.border
            && self.code_background == other.code_background
            && self.code_foreground == other.code_foreground
            && self.table_header == other.table_header
            && self.table_row == other.table_row
            && self.table_row_alt == other.table_row_alt
            && self.quote_border == other.quote_border
            && self.link == other.link
    }
}

impl Default for TextViewStyle {
    fn default() -> Self {
        Self {
            paragraph_gap: rems(1.),
            heading_base_font_size: px(14.),
            heading_font_size: None,
            highlight_theme: HighlightTheme::default_light().clone(),
            code_block: StyleRefinement::default(),
            is_dark: false,
            foreground: None,
            muted_foreground: None,
            border: None,
            code_background: None,
            code_foreground: None,
            table_header: None,
            table_row: None,
            table_row_alt: None,
            quote_border: None,
            link: None,
        }
    }
}

impl TextViewStyle {
    /// Set paragraph gap, default is 1 rem.
    pub fn paragraph_gap(mut self, gap: Rems) -> Self {
        self.paragraph_gap = gap;
        self
    }

    pub fn heading_font_size<F>(mut self, f: F) -> Self
    where
        F: Fn(u8, Pixels) -> Pixels + Send + Sync + 'static,
    {
        self.heading_font_size = Some(Arc::new(f));
        self
    }

    /// Set style for code blocks.
    pub fn code_block(mut self, style: StyleRefinement) -> Self {
        self.code_block = style;
        self
    }

    pub fn markdown_palette(mut self, palette: MarkdownPalette) -> Self {
        self.is_dark = palette.is_dark;
        self.highlight_theme = if palette.is_dark {
            HighlightTheme::default_dark().clone()
        } else {
            HighlightTheme::default_light().clone()
        };
        self.foreground = Some(palette.foreground);
        self.muted_foreground = Some(palette.muted_foreground);
        self.border = Some(palette.border);
        self.code_background = Some(palette.code_background);
        self.code_foreground = Some(palette.code_foreground);
        self.table_header = Some(palette.table_header);
        self.table_row = Some(palette.table_row);
        self.table_row_alt = Some(palette.table_row_alt);
        self.quote_border = Some(palette.quote_border);
        self.link = Some(palette.link);
        self
    }
}

#[cfg(test)]
mod tests {
    use gpui::rgb;

    use super::*;

    fn color(hex: u32) -> Hsla {
        rgb(hex).into()
    }

    #[test]
    fn markdown_palette_sets_local_colors_without_global_theme() {
        let style = TextViewStyle::default().markdown_palette(MarkdownPalette {
            is_dark: true,
            foreground: color(0xf8fafc),
            muted_foreground: color(0x94a3b8),
            border: color(0x334155),
            code_background: color(0x0f172a),
            code_foreground: color(0xe2e8f0),
            table_header: color(0x1e293b),
            table_row: color(0x020617),
            table_row_alt: color(0x111827),
            quote_border: color(0x475569),
            link: color(0x38bdf8),
        });

        assert!(style.is_dark);
        assert_eq!(Some(color(0xf8fafc)), style.foreground);
        assert_eq!(Some(color(0x94a3b8)), style.muted_foreground);
        assert_eq!(Some(color(0x334155)), style.border);
        assert_eq!(Some(color(0x0f172a)), style.code_background);
        assert_eq!(Some(color(0xe2e8f0)), style.code_foreground);
        assert_eq!(Some(color(0x1e293b)), style.table_header);
        assert_eq!(Some(color(0x020617)), style.table_row);
        assert_eq!(Some(color(0x111827)), style.table_row_alt);
        assert_eq!(Some(color(0x475569)), style.quote_border);
        assert_eq!(Some(color(0x38bdf8)), style.link);
    }
}
