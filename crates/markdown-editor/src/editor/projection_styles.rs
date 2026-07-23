use crate::{
    MarkdownBlockRenderArtifact, MarkdownEditorTheme, MarkdownProjection, ProjectionStyle,
};
use gpui::{FontStyle, FontWeight, HighlightStyle, StrikethroughStyle, UnderlineStyle, px};
use gpui_component::input::InputTextHighlight;
use std::collections::HashMap;

pub(super) fn projection_highlights(
    projection: &MarkdownProjection,
    theme: &MarkdownEditorTheme,
    inline_math_artifacts: &HashMap<String, MarkdownBlockRenderArtifact>,
) -> Vec<InputTextHighlight> {
    projection
        .styles
        .iter()
        .map(|span| {
            let mut style = projection_style(span.style, theme);
            if span.style == ProjectionStyle::InlineMath
                && projection.active_inline != Some(span.node_id)
                && inline_math_artifacts.contains_key(&projection.text[span.range.clone()])
            {
                style.color = Some(theme.foreground.opacity(0.));
                style.background_color = None;
            }
            (span.range.clone(), style)
        })
        .collect()
}

fn projection_style(style: ProjectionStyle, theme: &MarkdownEditorTheme) -> HighlightStyle {
    match style {
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
