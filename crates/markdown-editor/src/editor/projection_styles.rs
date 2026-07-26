use crate::{MarkdownEditorTheme, MarkdownProjection, ProjectionStyle};
use gpui::{FontStyle, FontWeight, HighlightStyle, StrikethroughStyle, UnderlineStyle, px};
use gpui_component::input::InputTextHighlight;

pub(super) fn projection_highlights(
    projection: &MarkdownProjection,
    theme: &MarkdownEditorTheme,
) -> Vec<InputTextHighlight> {
    projection
        .styles
        .iter()
        .map(|span| {
            let reserved_marker = span.style == ProjectionStyle::Marker
                && projection.active_inline != Some(span.node_id);
            (
                span.range.clone(),
                projection_style(span.style, reserved_marker, theme),
            )
        })
        .collect()
}

fn projection_style(
    style: ProjectionStyle,
    reserved_marker: bool,
    theme: &MarkdownEditorTheme,
) -> HighlightStyle {
    match style {
        ProjectionStyle::Marker if reserved_marker => HighlightStyle {
            color: Some(theme.background),
            ..Default::default()
        },
        ProjectionStyle::Marker => HighlightStyle {
            color: Some(theme.muted_foreground),
            ..Default::default()
        },
        ProjectionStyle::Emphasis => HighlightStyle {
            font_style: Some(FontStyle::Italic),
            ..Default::default()
        },
        ProjectionStyle::Strong => HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        },
        ProjectionStyle::InlineCode | ProjectionStyle::InlineMath => HighlightStyle {
            color: theme
                .highlight_theme
                .style
                .syntax
                .string_special
                .map(Into::into)
                .and_then(|style: HighlightStyle| style.color),
            background_color: Some(theme.border.opacity(0.22)),
            ..Default::default()
        },
        ProjectionStyle::Link | ProjectionStyle::Image => HighlightStyle {
            color: Some(theme.primary),
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(theme.primary),
                wavy: false,
            }),
            ..Default::default()
        },
        ProjectionStyle::Delete => HighlightStyle {
            strikethrough: Some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(theme.muted_foreground),
            }),
            ..Default::default()
        },
    }
}
