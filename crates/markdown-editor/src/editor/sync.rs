use super::MarkdownEditor;
use crate::{MarkdownEditorEvent, MarkdownProjection, ProjectionStyle};
use gpui::{
    App, Context, FontStyle, FontWeight, HighlightStyle, StrikethroughStyle, UnderlineStyle,
    Window, px,
};
use gpui_component::input::Position;
use markdown_source::{SourceInlineKind, SourceNodeId, SourceSelection, TableCellAddress};

impl MarkdownEditor {
    pub(super) fn refresh_projection_highlights(&self, cx: &mut Context<Self>) {
        let highlights =
            projection_highlights(&self.projection, &self.theme, &self.inline_math_artifacts);
        self.input
            .update(cx, |input, cx| input.set_text_highlights(highlights, cx));
    }

    pub(super) fn input_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.syncing_input {
            return;
        }
        let value = self.input.read(cx).value().to_string();
        if value == self.projection.text {
            return;
        }
        let source_cursor = self
            .projection
            .display_to_source(self.input.read(cx).selected_range().end);
        if let Some(edit) = self.projection.edit_for_value(&value)
            && !self.active_block_is_source_code()
            && edit.source_range.is_empty()
            && edit.replacement == "\n"
        {
            self.pending_newline = Some(edit.source_range.start);
            cx.defer_in(window, |editor, window, cx| {
                editor.flush_pending_newline(window, cx);
            });
            return;
        }
        if !matches!(self.edit_projected_value(&value, window, cx), Ok(true)) {
            self.resync_active(source_cursor, window, cx);
        }
    }

    pub(super) fn input_entered(
        &mut self,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_block_is_source_code() {
            let value = self.input.read(cx).value().to_string();
            let cursor = self
                .projection
                .display_end_to_source(self.input.read(cx).selected_range().end);
            let edited = self.edit_projected_value(&value, window, cx);
            if !matches!(edited, Ok(true)) {
                self.resync_active(cursor, window, cx);
            }
            return;
        }
        let Some(source_offset) = self.pending_newline.take() else {
            return;
        };
        if !secondary && matches!(self.split_active_block(source_offset, window, cx), Ok(true)) {
            return;
        }
        let value = self.input.read(cx).value().to_string();
        if !matches!(self.edit_projected_value(&value, window, cx), Ok(true)) {
            self.resync_active(source_offset, window, cx);
        }
    }

    fn flush_pending_newline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source_offset) = self.pending_newline.take() else {
            return;
        };
        let value = self.input.read(cx).value().to_string();
        if !matches!(self.edit_projected_value(&value, window, cx), Ok(true)) {
            self.resync_active(source_offset, window, cx);
        }
    }

    pub(super) fn cursor_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.syncing_input {
            return;
        }
        let display_cursor = self.input.read(cx).selected_range().end;
        let selection = self.source_selection(cx);
        let active_inline = self.active_inline_at_display(display_cursor);
        if active_inline != self.projection.active_inline {
            self.sync_selection(selection, window, cx);
        }
    }

    fn active_inline_at_display(&self, display_offset: usize) -> Option<SourceNodeId> {
        let document = self.history.document();
        let direct = self.projection.display_to_source(display_offset);
        document
            .inline_node_at(direct)
            .or_else(|| {
                previous_char_offset(
                    &document.source,
                    self.projection.display_end_to_source(display_offset),
                )
                .and_then(|offset| document.inline_node_at(offset))
            })
            .filter(|node| !matches!(node.kind, SourceInlineKind::RawMarkdown))
            .filter(|node| {
                self.projection.source_range.start <= node.source_range.start
                    && node.source_range.end <= self.projection.source_range.end
            })
            .filter(|node| {
                self.projection.source_to_display(node.source_range.start)
                    < self.projection.source_to_display(node.source_range.end)
            })
            .map(|node| node.id)
    }

    pub(super) fn sync_projection(
        &mut self,
        source_cursor: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document = self.history.document();
        let active_block = document.block_at(source_cursor).or_else(|| {
            previous_char_offset(&document.source, source_cursor)
                .and_then(|offset| document.block_at(offset))
        });
        let active = document
            .inline_node_at(source_cursor)
            .or_else(|| {
                previous_char_offset(&document.source, source_cursor)
                    .and_then(|offset| document.inline_node_at(offset))
            })
            .filter(|node| !matches!(node.kind, SourceInlineKind::RawMarkdown))
            .map(|node| node.id);
        self.active_block = active_block.map(|block| block.id);
        self.active_table_cell = None;
        self.projection = active_block.map_or_else(
            || MarkdownProjection::build(document, active),
            |block| {
                MarkdownProjection::build_range_preserving_layout(
                    document,
                    active,
                    block.source_range.clone(),
                )
            },
        );
        self.sync_input_mode(window, cx);
        self.sync_image_property_inputs(window, cx);
        self.sync_input(source_cursor, window, cx);
    }

    pub(super) fn sync_table_cell(
        &mut self,
        address: TableCellAddress,
        source_cursor: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document = self.history.document();
        let Ok(cell) = document.table_cell(address) else {
            self.sync_projection(source_cursor, window, cx);
            return;
        };
        let active = document
            .inline_node_at(source_cursor)
            .filter(|node| !matches!(node.kind, SourceInlineKind::RawMarkdown))
            .map(|node| node.id);
        self.active_block = Some(address.block_id);
        self.active_table_cell = Some(address);
        self.projection = MarkdownProjection::build_range_preserving_layout(
            document,
            active,
            cell.content_range.clone(),
        );
        self.sync_input_mode(window, cx);
        self.sync_image_property_inputs(window, cx);
        self.sync_input(source_cursor, window, cx);
    }

    fn resync_active(&mut self, source_cursor: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(address) = self.active_table_cell {
            self.sync_table_cell(address, source_cursor, window, cx);
        } else {
            self.sync_projection(source_cursor, window, cx);
        }
    }

    fn sync_input(&mut self, source_cursor: usize, window: &mut Window, cx: &mut Context<Self>) {
        let display_cursor = self.projection.source_to_display(source_cursor);
        let position = position_for_offset(&self.projection.text, display_cursor);
        self.syncing_input = true;
        self.input.update(cx, |input, cx| {
            if input.value() != self.projection.text {
                input.set_value(self.projection.text.clone(), window, cx);
            }
            input.set_text_highlights(
                projection_highlights(&self.projection, &self.theme, &self.inline_math_artifacts),
                cx,
            );
            if input.selected_range() != (display_cursor..display_cursor) {
                input.set_cursor_position(position, window, cx);
            }
        });
        self.syncing_input = false;
        cx.notify();
    }

    fn active_block_is_source_code(&self) -> bool {
        self.active_block
            .and_then(|id| self.history.document().block_by_id(id))
            .is_some_and(|block| {
                matches!(
                    block.kind,
                    markdown_source::SourceBlockKind::CodeFence { .. }
                        | markdown_source::SourceBlockKind::MathBlock { .. }
                )
            })
    }

    fn sync_input_mode(&self, window: &mut Window, cx: &mut Context<Self>) {
        let language = self.active_block.and_then(|id| {
            let block = self.history.document().block_by_id(id)?;
            match &block.kind {
                markdown_source::SourceBlockKind::CodeFence { language_range, .. } => {
                    language_range
                        .as_ref()
                        .map(|range| self.history.document().source[range.clone()].to_owned())
                        .map(|language| {
                            if language.eq_ignore_ascii_case("mermaid") {
                                "text".to_owned()
                            } else {
                                language
                            }
                        })
                        .or_else(|| Some("text".to_owned()))
                }
                markdown_source::SourceBlockKind::MathBlock { .. } => Some("latex".to_owned()),
                _ => None,
            }
        });
        self.input.update(cx, |input, cx| {
            if let Some(language) = language {
                input.set_code_editor_mode(language, window, cx);
            } else {
                input.set_rich_text_mode(window, cx);
            }
        });
    }

    fn sync_image_property_inputs(&self, window: &mut Window, cx: &mut Context<Self>) {
        let (alt, destination) = self.active_image_properties().unwrap_or_default();
        self.image_alt_input
            .update(cx, |input, cx| input.set_value(alt, window, cx));
        self.image_destination_input.update(cx, |input, cx| {
            input.set_value(destination, window, cx);
        });
    }

    pub(super) fn source_selection(&self, cx: &App) -> SourceSelection {
        let range = self.input.read(cx).selected_range();
        SourceSelection {
            anchor: self.projection.display_to_source(range.start),
            head: self.projection.display_end_to_source(range.end),
        }
    }

    pub(super) fn sync_selection(
        &mut self,
        selection: SourceSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resync_active(selection.head, window, cx);
        let start = self.projection.source_to_display(selection.anchor);
        let end = self.projection.source_to_display(selection.head);
        self.input.update(cx, |input, cx| {
            input.set_selected_range(start.min(end)..start.max(end), start > end, window, cx);
        });
    }

    pub(super) fn emit_changed(&self, cx: &mut Context<Self>) {
        cx.emit(MarkdownEditorEvent::Changed {
            source: self.source().to_owned(),
            revision: self.revision(),
        });
    }
}

pub(super) fn projection_highlights(
    projection: &MarkdownProjection,
    theme: &crate::MarkdownEditorTheme,
    inline_math_artifacts: &std::collections::HashMap<String, crate::MarkdownBlockRenderArtifact>,
) -> Vec<gpui_component::input::InputTextHighlight> {
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

fn projection_style(style: ProjectionStyle, theme: &crate::MarkdownEditorTheme) -> HighlightStyle {
    match style {
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

fn position_for_offset(value: &str, offset: usize) -> Position {
    let prefix = &value[..offset.min(value.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    Position::new(line as u32, prefix[line_start..].chars().count() as u32)
}

fn previous_char_offset(value: &str, offset: usize) -> Option<usize> {
    value
        .get(..offset)?
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
}
