//! Inline projection engine for editable Markdown delimiters.

use std::ops::Range;

use crate::components::InlineFootnoteReference;
use crate::components::markdown::inline::{
    InlineFragment, InlineLink, InlineRenderCache, InlineScript, InlineStyle, InlineTextTree,
    StyleFlag, can_use_markdown_script_delimiters,
};

use super::CollapsedCaretAffinity;

/// One displayed segment in an expanded inline projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ExpandedInlineSegment {
    pub(super) display_range: Range<usize>,
    pub(super) clean_range: Range<usize>,
    pub(super) fragment_index: usize,
    pub(super) link_group: Option<usize>,
    pub(super) kind: ExpandedInlineSegmentKind,
}

/// Inline construct whose Markdown delimiters can be projected for editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExpandedInlineKind {
    /// Link label and target syntax.
    Link,
    /// Bold Markdown delimiters.
    BoldMarkdown,
    /// Italic Markdown delimiters.
    ItalicMarkdown,
    /// Strikethrough delimiters.
    Strikethrough,
    /// Code span backtick delimiters.
    Code,
    /// Superscript Markdown delimiters.
    SuperscriptMarkdown,
    /// Superscript HTML delimiters.
    SuperscriptHtml,
    /// Subscript Markdown delimiters.
    SubscriptMarkdown,
    /// Subscript HTML delimiters.
    SubscriptHtml,
}

impl ExpandedInlineKind {
    fn applies_to(self, style: InlineStyle) -> bool {
        match self {
            Self::Link => false,
            Self::BoldMarkdown => style.bold,
            Self::ItalicMarkdown => style.italic,
            Self::Strikethrough => style.strikethrough,
            Self::Code => style.code,
            Self::SuperscriptMarkdown | Self::SuperscriptHtml => {
                style.script == InlineScript::Superscript
            }
            Self::SubscriptMarkdown | Self::SubscriptHtml => {
                style.script == InlineScript::Subscript
            }
        }
    }

    fn open_marker(self) -> &'static str {
        match self {
            Self::Link => "[",
            Self::BoldMarkdown => "**",
            Self::ItalicMarkdown => "*",
            Self::Strikethrough => "~~",
            Self::Code => "`",
            Self::SuperscriptMarkdown => "^",
            Self::SuperscriptHtml => "<sup>",
            Self::SubscriptMarkdown => "~",
            Self::SubscriptHtml => "<sub>",
        }
    }

    fn close_marker(self) -> &'static str {
        match self {
            Self::Link => ")",
            Self::SuperscriptHtml => "</sup>",
            Self::SubscriptHtml => "</sub>",
            _ => self.open_marker(),
        }
    }

    pub(super) fn style_flag(self) -> Option<StyleFlag> {
        match self {
            Self::Link => None,
            Self::BoldMarkdown => Some(StyleFlag::Bold),
            Self::ItalicMarkdown => Some(StyleFlag::Italic),
            Self::Strikethrough => Some(StyleFlag::Strikethrough),
            Self::Code => Some(StyleFlag::Code),
            Self::SuperscriptMarkdown | Self::SuperscriptHtml => Some(StyleFlag::Superscript),
            Self::SubscriptMarkdown | Self::SubscriptHtml => Some(StyleFlag::Subscript),
        }
    }

    fn projection_rank(self) -> u8 {
        match self {
            Self::Link => 0,
            Self::BoldMarkdown => 1,
            Self::Strikethrough => 2,
            Self::SuperscriptMarkdown
            | Self::SuperscriptHtml
            | Self::SubscriptMarkdown
            | Self::SubscriptHtml => 3,
            Self::ItalicMarkdown => 4,
            Self::Code => 5,
        }
    }
}

/// Display role of one projected inline segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExpandedInlineSegmentKind {
    /// Text with no projected inline syntax.
    PlainText,
    /// Text carrying projected style.
    StyledText,
    /// Opening delimiter such as `[` or backticks.
    OpeningDelimiter(ExpandedInlineKind),
    /// Middle delimiter such as `](` for links.
    MiddleDelimiter(ExpandedInlineKind),
    /// Editable link target text.
    LinkTargetText,
    /// Editable footnote id text.
    FootnoteIdText,
    /// Closing delimiter such as `)` or backticks.
    ClosingDelimiter(ExpandedInlineKind),
}

/// One projected link run spanning one or more inline fragments.
#[derive(Clone, Debug)]
pub(super) struct ExpandedLinkRun {
    pub(super) link: InlineLink,
    pub(super) start_fragment_index: usize,
    pub(super) end_fragment_index: usize,
    pub(super) clean_range: Range<usize>,
    pub(super) display_range: Range<usize>,
    pub(super) target_display_range: Range<usize>,
}

/// One projected footnote reference run.
#[derive(Clone, Debug)]
pub(super) struct ExpandedFootnoteRun {
    pub(super) footnote: InlineFootnoteReference,
    pub(super) clean_range: Range<usize>,
    pub(super) display_range: Range<usize>,
}

/// Selection snapshot translated into an expanded link display range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProjectedLinkSelectionSnapshot {
    pub(super) clean_range: Range<usize>,
    pub(super) display_relative_range: Range<usize>,
    pub(super) selection_reversed: bool,
}

/// Render cache and offset maps for an expanded inline projection.
#[derive(Clone, Debug)]
pub(crate) struct ExpandedInlineProjection {
    pub(super) cache: InlineRenderCache,
    pub(super) segments: Vec<ExpandedInlineSegment>,
    pub(super) clean_to_display_cursor: Vec<usize>,
    pub(super) display_to_clean: Vec<usize>,
    pub(super) link_runs: Vec<ExpandedLinkRun>,
    pub(super) footnote_runs: Vec<ExpandedFootnoteRun>,
}

#[cfg(test)]
pub(super) fn expanded_display_offset_for_clean(
    fragments: &[InlineFragment],
    clean: usize,
) -> usize {
    let mut display = 0usize;
    let mut clean_cursor = 0usize;
    for fragment in fragments {
        let clean_len = fragment.text.len();
        let clean_end = clean_cursor + clean_len;
        if clean <= clean_end {
            let off = clean.saturating_sub(clean_cursor);
            if fragment.style.code && clean_len > 0 {
                if off == clean_len {
                    return display + clean_len + 2;
                }
                return display + 1 + off;
            }
            if fragment.style.script != InlineScript::Normal && clean_len > 0 {
                if off == clean_len {
                    return display + clean_len + 2;
                }
                return display + 1 + off;
            }
            return display + off;
        }
        clean_cursor = clean_end;
        display += if fragment.style.code && clean_len > 0 {
            clean_len + 2
        } else if fragment.style.script != InlineScript::Normal && clean_len > 0 {
            clean_len + 2
        } else {
            clean_len
        };
    }
    display
}

#[cfg(test)]
pub(super) fn expanded_display_cursor_offset_for_clean(
    fragments: &[InlineFragment],
    clean: usize,
) -> usize {
    let mut display = 0usize;
    let mut clean_cursor = 0usize;
    for fragment in fragments {
        let clean_len = fragment.text.len();
        let clean_end = clean_cursor + clean_len;
        if clean <= clean_end {
            let off = clean.saturating_sub(clean_cursor);
            if fragment.style.code && clean_len > 0 {
                if off == 0 {
                    return display + 1;
                }
                if off >= clean_len {
                    return display + clean_len + 1;
                }
                return display + 1 + off;
            }
            if fragment.style.script != InlineScript::Normal && clean_len > 0 {
                if off == 0 {
                    return display + 1;
                }
                if off >= clean_len {
                    return display + clean_len + 1;
                }
                return display + 1 + off;
            }
            return display + off;
        }
        clean_cursor = clean_end;
        display += if fragment.style.code && clean_len > 0 {
            clean_len + 2
        } else if fragment.style.script != InlineScript::Normal && clean_len > 0 {
            clean_len + 2
        } else {
            clean_len
        };
    }
    display
}

impl ExpandedInlineProjection {
    // Projection is a temporary editing view over clean inline fragments. It
    // exposes delimiters only for the fragment touched by the caret, selection,
    // or IME marked range, while preserving maps back to clean text offsets.
    pub(super) fn build(
        fragments: &[InlineFragment],
        clean_selected: Range<usize>,
        clean_marked: Option<Range<usize>>,
    ) -> Option<Self> {
        let clean_len = fragments
            .iter()
            .map(|fragment| fragment.text.len())
            .sum::<usize>();
        let mut projected_fragments = Vec::new();
        let mut segments = Vec::new();
        let mut clean_to_display_cursor = vec![0; clean_len + 1];
        let mut display_to_clean = vec![0];
        let mut link_runs = Vec::new();
        let mut footnote_runs = Vec::new();
        let mut clean_cursor = 0usize;
        let mut display_cursor = 0usize;
        let mut any_expanded = false;
        let mut fragment_index = 0usize;

        while fragment_index < fragments.len() {
            let fragment = &fragments[fragment_index];
            let fragment_len = fragment.text.len();
            if fragment_len == 0 {
                fragment_index += 1;
                continue;
            }

            if let Some(footnote) = fragment.footnote.as_ref() {
                let clean_range = clean_cursor..clean_cursor + fragment_len;
                let expand_footnote = Self::fragment_is_touched(
                    clean_range.clone(),
                    &clean_selected,
                    clean_marked.as_ref(),
                );
                let run_display_start = display_cursor;
                if expand_footnote {
                    any_expanded = true;
                    let open_marker = "[^".to_string();
                    let open_len = open_marker.len();
                    projected_fragments.push(InlineFragment {
                        text: open_marker,
                        style: InlineStyle::default(),
                        html_style: None,
                        link: None,
                        footnote: None,
                        math: None,
                    });
                    segments.push(ExpandedInlineSegment {
                        display_range: display_cursor..display_cursor + open_len,
                        clean_range: clean_range.start..clean_range.start,
                        fragment_index,
                        link_group: None,
                        kind: ExpandedInlineSegmentKind::OpeningDelimiter(ExpandedInlineKind::Link),
                    });
                    for _ in 0..open_len {
                        display_to_clean.push(clean_range.start);
                    }
                    display_cursor += open_len;

                    let id_text = footnote.id.clone();
                    let id_len = id_text.len();
                    projected_fragments.push(InlineFragment {
                        text: id_text,
                        style: fragment.style,
                        html_style: fragment.html_style,
                        link: None,
                        footnote: Some(footnote.clone()),
                        math: None,
                    });
                    segments.push(ExpandedInlineSegment {
                        display_range: display_cursor..display_cursor + id_len,
                        clean_range: clean_range.clone(),
                        fragment_index,
                        link_group: None,
                        kind: ExpandedInlineSegmentKind::FootnoteIdText,
                    });
                    for offset in 0..=fragment_len {
                        let mapped = if fragment_len == 0 {
                            0
                        } else {
                            (id_len * offset) / fragment_len
                        };
                        clean_to_display_cursor[clean_range.start + offset] =
                            display_cursor + mapped;
                    }
                    for offset in 1..=id_len {
                        let mapped = if id_len == 0 {
                            0
                        } else {
                            (fragment_len * offset) / id_len
                        };
                        display_to_clean.push(clean_range.start + mapped);
                    }
                    display_cursor += id_len;
                    let close_marker = "]".to_string();
                    let close_len = close_marker.len();
                    projected_fragments.push(InlineFragment {
                        text: close_marker,
                        style: InlineStyle::default(),
                        html_style: None,
                        link: None,
                        footnote: None,
                        math: None,
                    });
                    segments.push(ExpandedInlineSegment {
                        display_range: display_cursor..display_cursor + close_len,
                        clean_range: clean_range.end..clean_range.end,
                        fragment_index,
                        link_group: None,
                        kind: ExpandedInlineSegmentKind::ClosingDelimiter(ExpandedInlineKind::Link),
                    });
                    for _ in 0..close_len {
                        display_to_clean.push(clean_range.end);
                    }
                    display_cursor += close_len;

                    footnote_runs.push(ExpandedFootnoteRun {
                        footnote: footnote.clone(),
                        clean_range: clean_range.clone(),
                        display_range: run_display_start..display_cursor,
                    });
                } else {
                    projected_fragments.push(fragment.clone());
                    segments.push(ExpandedInlineSegment {
                        display_range: display_cursor..display_cursor + fragment_len,
                        clean_range: clean_range.clone(),
                        fragment_index,
                        link_group: None,
                        kind: ExpandedInlineSegmentKind::PlainText,
                    });
                    for offset in 0..=fragment_len {
                        clean_to_display_cursor[clean_range.start + offset] =
                            display_cursor + offset;
                    }
                    for offset in 1..=fragment_len {
                        display_to_clean.push(clean_range.start + offset);
                    }
                    display_cursor += fragment_len;
                }

                clean_cursor = clean_range.end;
                fragment_index += 1;
                continue;
            }

            if let Some(link) = fragment.link.as_ref() {
                let run_start = fragment_index;
                let run_clean_start = clean_cursor;
                let mut run_end = fragment_index;
                let mut run_clean_end = clean_cursor;
                while run_end < fragments.len() {
                    let run_fragment = &fragments[run_end];
                    if run_fragment.link.as_ref() != Some(link) {
                        break;
                    }
                    run_clean_end += run_fragment.text.len();
                    run_end += 1;
                }

                let run_clean_range = run_clean_start..run_clean_end;
                let expand_link = Self::fragment_is_touched(
                    run_clean_range.clone(),
                    &clean_selected,
                    clean_marked.as_ref(),
                );
                let link_group = expand_link.then_some(link_runs.len());
                let run_display_start = display_cursor;
                if expand_link {
                    any_expanded = true;
                    let open_marker = link.open_marker().to_string();
                    let open_len = open_marker.len();
                    projected_fragments.push(InlineFragment {
                        text: open_marker,
                        style: InlineStyle::default(),
                        html_style: None,
                        link: None,
                        footnote: None,
                        math: None,
                    });
                    segments.push(ExpandedInlineSegment {
                        display_range: display_cursor..display_cursor + open_len,
                        clean_range: run_clean_start..run_clean_start,
                        fragment_index: run_start,
                        link_group,
                        kind: ExpandedInlineSegmentKind::OpeningDelimiter(ExpandedInlineKind::Link),
                    });
                    for _ in 0..open_len {
                        display_to_clean.push(run_clean_start);
                    }
                    display_cursor += open_len;
                }

                let mut local_clean_cursor = run_clean_start;
                for current_index in run_start..run_end {
                    let current_fragment = &fragments[current_index];
                    let current_len = current_fragment.text.len();
                    let current_clean_range = local_clean_cursor..local_clean_cursor + current_len;
                    // While the link is expanded, reveal each label fragment's
                    // own emphasis markers so anchor text edits like ordinary text.
                    let label_kinds = if expand_link {
                        Self::expanded_kinds_for_fragment(
                            fragments,
                            current_index,
                            current_fragment.style,
                            current_clean_range.clone(),
                            &clean_selected,
                            clean_marked.as_ref(),
                        )
                    } else {
                        Vec::new()
                    };
                    push_projected_fragment(
                        current_fragment,
                        current_index,
                        current_clean_range.clone(),
                        &label_kinds,
                        link_group,
                        expand_link,
                        &mut projected_fragments,
                        &mut segments,
                        &mut clean_to_display_cursor,
                        &mut display_to_clean,
                        &mut display_cursor,
                        &mut any_expanded,
                    );
                    local_clean_cursor = current_clean_range.end;
                }
                if expand_link {
                    if let Some(middle_marker) = link.middle_marker() {
                        let middle_len = middle_marker.len();
                        projected_fragments.push(InlineFragment {
                            text: middle_marker.to_string(),
                            style: InlineStyle::default(),
                            html_style: None,
                            link: None,
                            footnote: None,
                            math: None,
                        });
                        segments.push(ExpandedInlineSegment {
                            display_range: display_cursor..display_cursor + middle_len,
                            clean_range: run_clean_end..run_clean_end,
                            fragment_index: run_start,
                            link_group,
                            kind: ExpandedInlineSegmentKind::MiddleDelimiter(
                                ExpandedInlineKind::Link,
                            ),
                        });
                        for _ in 0..middle_len {
                            display_to_clean.push(run_clean_end);
                        }
                        display_cursor += middle_len;
                    }

                    let target_display_start = display_cursor;
                    if let Some(link_target) = link.editable_text() {
                        let target_len = link_target.len();
                        if target_len > 0 {
                            projected_fragments.push(InlineFragment {
                                text: link_target,
                                style: InlineStyle::default(),
                                html_style: None,
                                link: Some(link.clone()),
                                footnote: None,
                                math: None,
                            });
                            segments.push(ExpandedInlineSegment {
                                display_range: display_cursor..display_cursor + target_len,
                                clean_range: run_clean_end..run_clean_end,
                                fragment_index: run_start,
                                link_group,
                                kind: ExpandedInlineSegmentKind::LinkTargetText,
                            });
                            for _ in 0..target_len {
                                display_to_clean.push(run_clean_end);
                            }
                            display_cursor += target_len;
                        }
                    }
                    let target_display_end = display_cursor;

                    let close_marker = link.close_marker().to_string();
                    let close_len = close_marker.len();
                    projected_fragments.push(InlineFragment {
                        text: close_marker,
                        style: InlineStyle::default(),
                        html_style: None,
                        link: None,
                        footnote: None,
                        math: None,
                    });
                    segments.push(ExpandedInlineSegment {
                        display_range: display_cursor..display_cursor + close_len,
                        clean_range: run_clean_end..run_clean_end,
                        fragment_index: run_start,
                        link_group,
                        kind: ExpandedInlineSegmentKind::ClosingDelimiter(ExpandedInlineKind::Link),
                    });
                    for _ in 0..close_len {
                        display_to_clean.push(run_clean_end);
                    }
                    display_cursor += close_len;

                    link_runs.push(ExpandedLinkRun {
                        link: link.clone(),
                        start_fragment_index: run_start,
                        end_fragment_index: run_end,
                        clean_range: run_clean_range.clone(),
                        display_range: run_display_start..display_cursor,
                        target_display_range: target_display_start..target_display_end,
                    });
                }

                clean_cursor = run_clean_end;
                fragment_index = run_end;
                continue;
            }

            let clean_range = clean_cursor..clean_cursor + fragment_len;
            let expanded_kinds = Self::expanded_kinds_for_fragment(
                fragments,
                fragment_index,
                fragment.style,
                clean_range.clone(),
                &clean_selected,
                clean_marked.as_ref(),
            );

            push_projected_fragment(
                fragment,
                fragment_index,
                clean_range.clone(),
                &expanded_kinds,
                None,
                false,
                &mut projected_fragments,
                &mut segments,
                &mut clean_to_display_cursor,
                &mut display_to_clean,
                &mut display_cursor,
                &mut any_expanded,
            );

            clean_cursor = clean_range.end;
            fragment_index += 1;
        }

        if any_expanded {
            for segment in &segments {
                match segment.kind {
                    ExpandedInlineSegmentKind::OpeningDelimiter(
                        ExpandedInlineKind::BoldMarkdown,
                    )
                    | ExpandedInlineSegmentKind::OpeningDelimiter(
                        ExpandedInlineKind::ItalicMarkdown,
                    )
                    | ExpandedInlineSegmentKind::OpeningDelimiter(ExpandedInlineKind::Code)
                    | ExpandedInlineSegmentKind::OpeningDelimiter(
                        ExpandedInlineKind::Strikethrough,
                    )
                    | ExpandedInlineSegmentKind::OpeningDelimiter(
                        ExpandedInlineKind::SuperscriptMarkdown,
                    )
                    | ExpandedInlineSegmentKind::OpeningDelimiter(
                        ExpandedInlineKind::SubscriptMarkdown,
                    ) => {
                        clean_to_display_cursor[segment.clean_range.start] =
                            segment.display_range.end;
                    }
                    ExpandedInlineSegmentKind::ClosingDelimiter(
                        ExpandedInlineKind::BoldMarkdown,
                    )
                    | ExpandedInlineSegmentKind::ClosingDelimiter(
                        ExpandedInlineKind::ItalicMarkdown,
                    )
                    | ExpandedInlineSegmentKind::ClosingDelimiter(ExpandedInlineKind::Code)
                    | ExpandedInlineSegmentKind::ClosingDelimiter(
                        ExpandedInlineKind::Strikethrough,
                    )
                    | ExpandedInlineSegmentKind::ClosingDelimiter(
                        ExpandedInlineKind::SuperscriptMarkdown,
                    )
                    | ExpandedInlineSegmentKind::ClosingDelimiter(
                        ExpandedInlineKind::SubscriptMarkdown,
                    ) => {
                        clean_to_display_cursor[segment.clean_range.start] =
                            segment.display_range.start;
                    }
                    _ => {}
                }
            }
        }

        any_expanded.then(|| Self {
            cache: InlineTextTree::from_fragments(projected_fragments).render_cache(),
            segments,
            clean_to_display_cursor,
            display_to_clean,
            link_runs,
            footnote_runs,
        })
    }

    #[allow(dead_code)]
    pub(super) fn pointer_target_offset(&self, offset: usize) -> usize {
        for segment in &self.segments {
            match segment.kind {
                ExpandedInlineSegmentKind::OpeningDelimiter(_)
                    if offset >= segment.display_range.start
                        && offset <= segment.display_range.end =>
                {
                    return segment.display_range.end;
                }
                ExpandedInlineSegmentKind::ClosingDelimiter(_)
                    if offset >= segment.display_range.start
                        && offset <= segment.display_range.end =>
                {
                    return segment.display_range.start;
                }
                ExpandedInlineSegmentKind::MiddleDelimiter(ExpandedInlineKind::Link)
                    if offset >= segment.display_range.start
                        && offset <= segment.display_range.end =>
                {
                    if let Some(link_group) = segment.link_group
                        && let Some(run) = self.link_runs.get(link_group)
                    {
                        return run.target_display_range.start;
                    }
                    return segment.display_range.end;
                }
                _ => {}
            }
        }
        offset
    }

    pub(super) fn collapsed_affinity_for_display_offset(
        &self,
        offset: usize,
    ) -> CollapsedCaretAffinity {
        for segment in &self.segments {
            match segment.kind {
                ExpandedInlineSegmentKind::OpeningDelimiter(_)
                    if offset == segment.display_range.start =>
                {
                    return CollapsedCaretAffinity::OuterStart;
                }
                ExpandedInlineSegmentKind::ClosingDelimiter(_)
                    if offset == segment.display_range.end =>
                {
                    return CollapsedCaretAffinity::OuterEnd;
                }
                _ => {}
            }
        }
        CollapsedCaretAffinity::Default
    }

    /// Whether `clean` sits at the start of a projected closing delimiter (the
    /// end boundary of a styled span). Used to place the caret after a
    /// just-typed closing marker.
    pub(super) fn caret_closes_span_at_clean(&self, clean: usize) -> bool {
        self.segments.iter().any(|segment| {
            matches!(segment.kind, ExpandedInlineSegmentKind::ClosingDelimiter(_))
                && segment.clean_range.start == clean
        })
    }

    pub(super) fn display_offset_for_clean_cursor(
        &self,
        clean: usize,
        affinity: CollapsedCaretAffinity,
    ) -> Option<usize> {
        match affinity {
            CollapsedCaretAffinity::Default => self
                .clean_to_display_cursor
                .get(clean.min(self.clean_to_display_cursor.len().saturating_sub(1)))
                .copied(),
            CollapsedCaretAffinity::OuterStart => self
                .segments
                .iter()
                .find_map(|segment| match segment.kind {
                    ExpandedInlineSegmentKind::OpeningDelimiter(_)
                        if segment.clean_range.start == clean =>
                    {
                        Some(segment.display_range.start)
                    }
                    _ => None,
                })
                .or_else(|| {
                    self.clean_to_display_cursor
                        .get(clean.min(self.clean_to_display_cursor.len().saturating_sub(1)))
                        .copied()
                }),
            CollapsedCaretAffinity::OuterEnd => self
                .segments
                .iter()
                .find_map(|segment| match segment.kind {
                    ExpandedInlineSegmentKind::ClosingDelimiter(_)
                        if segment.clean_range.start == clean =>
                    {
                        Some(segment.display_range.end)
                    }
                    _ => None,
                })
                .or_else(|| {
                    self.clean_to_display_cursor
                        .get(clean.min(self.clean_to_display_cursor.len().saturating_sub(1)))
                        .copied()
                }),
        }
    }

    pub(super) fn move_left_target(
        &self,
        offset: usize,
    ) -> Option<(usize, CollapsedCaretAffinity)> {
        for segment in &self.segments {
            match segment.kind {
                ExpandedInlineSegmentKind::OpeningDelimiter(_)
                    if offset == segment.display_range.end =>
                {
                    return Some((
                        segment.display_range.start,
                        CollapsedCaretAffinity::OuterStart,
                    ));
                }
                ExpandedInlineSegmentKind::ClosingDelimiter(_)
                    if offset == segment.display_range.end =>
                {
                    return Some((segment.display_range.start, CollapsedCaretAffinity::Default));
                }
                _ => {}
            }
        }
        None
    }

    pub(super) fn move_right_target(
        &self,
        offset: usize,
    ) -> Option<(usize, CollapsedCaretAffinity)> {
        for segment in &self.segments {
            match segment.kind {
                ExpandedInlineSegmentKind::OpeningDelimiter(_)
                    if offset == segment.display_range.start =>
                {
                    return Some((segment.display_range.end, CollapsedCaretAffinity::Default));
                }
                ExpandedInlineSegmentKind::ClosingDelimiter(_)
                    if offset == segment.display_range.start =>
                {
                    return Some((segment.display_range.end, CollapsedCaretAffinity::OuterEnd));
                }
                _ => {}
            }
        }
        None
    }

    fn expanded_kinds_for_fragment(
        fragments: &[InlineFragment],
        fragment_index: usize,
        style: InlineStyle,
        fragment_range: Range<usize>,
        clean_selected: &Range<usize>,
        clean_marked: Option<&Range<usize>>,
    ) -> Vec<ExpandedInlineKind> {
        let mut kinds = Vec::new();
        let script_kind = Self::script_projection_kind(fragments, fragment_index);
        for kind in [
            Some(ExpandedInlineKind::BoldMarkdown),
            Some(ExpandedInlineKind::ItalicMarkdown),
            Some(ExpandedInlineKind::Strikethrough),
            script_kind,
            Some(ExpandedInlineKind::Code),
        ]
        .into_iter()
        .flatten()
        {
            if kind.applies_to(style)
                && Self::fragment_is_touched(fragment_range.clone(), clean_selected, clean_marked)
            {
                kinds.push(kind);
            }
        }
        kinds.sort_by_key(|kind| kind.projection_rank());
        kinds
    }

    fn script_projection_kind(
        fragments: &[InlineFragment],
        fragment_index: usize,
    ) -> Option<ExpandedInlineKind> {
        let fragment = fragments.get(fragment_index)?;
        match fragment.style.script {
            InlineScript::Normal => None,
            InlineScript::Superscript => {
                // Prefer compact Markdown markers only when serialization can
                // round-trip them safely; standalone script spans use HTML.
                if can_use_markdown_script_delimiters(
                    fragment_index
                        .checked_sub(1)
                        .and_then(|index| fragments.get(index)),
                    fragment,
                ) {
                    Some(ExpandedInlineKind::SuperscriptMarkdown)
                } else {
                    Some(ExpandedInlineKind::SuperscriptHtml)
                }
            }
            InlineScript::Subscript => {
                // A strikethrough subscript would serialize ambiguously around
                // `~`, so it also uses the HTML marker projection.
                if !fragment.style.strikethrough
                    && can_use_markdown_script_delimiters(
                        fragment_index
                            .checked_sub(1)
                            .and_then(|index| fragments.get(index)),
                        fragment,
                    )
                {
                    Some(ExpandedInlineKind::SubscriptMarkdown)
                } else {
                    Some(ExpandedInlineKind::SubscriptHtml)
                }
            }
        }
    }

    fn fragment_is_touched(
        fragment_range: Range<usize>,
        clean_selected: &Range<usize>,
        clean_marked: Option<&Range<usize>>,
    ) -> bool {
        if let Some(marked_range) = clean_marked
            && !marked_range.is_empty()
            && Self::ranges_overlap(&fragment_range, marked_range)
        {
            return true;
        }

        if !clean_selected.is_empty() {
            return Self::ranges_overlap(&fragment_range, clean_selected);
        }

        let cursor = clean_selected.start;
        fragment_range.start <= cursor && cursor <= fragment_range.end
    }

    fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
        left.start < right.end && right.start < left.end
    }

    pub(super) fn link_run_fully_covering_range(
        &self,
        range: &Range<usize>,
    ) -> Option<&ExpandedLinkRun> {
        self.link_runs.iter().find(|run| {
            run.display_range.start <= range.start && range.end <= run.display_range.end
        })
    }

    pub(super) fn link_run_for_clean_range(
        &self,
        clean_range: &Range<usize>,
    ) -> Option<&ExpandedLinkRun> {
        self.link_runs
            .iter()
            .find(|run| run.clean_range == *clean_range)
    }

    pub(super) fn footnote_run_fully_covering_range(
        &self,
        range: &Range<usize>,
    ) -> Option<&ExpandedFootnoteRun> {
        self.footnote_runs.iter().find(|run| {
            run.display_range.start <= range.start && range.end <= run.display_range.end
        })
    }
}

fn marker_style_for_projection(mut style: InlineStyle, kind: ExpandedInlineKind) -> InlineStyle {
    if matches!(
        kind,
        ExpandedInlineKind::SuperscriptMarkdown
            | ExpandedInlineKind::SuperscriptHtml
            | ExpandedInlineKind::SubscriptMarkdown
            | ExpandedInlineKind::SubscriptHtml
    ) {
        style.script = InlineScript::Normal;
    }
    style
}

/// Emit one inline fragment, wrapped in the projected emphasis delimiters for
/// `kinds`. Shared by standalone and link-label fragments so anchor text reveals
/// its bold/italic/code markers like ordinary text. `force_styled` keeps a
/// marker-less fragment styled (link labels while a link run is expanded).
#[allow(clippy::too_many_arguments)]
fn push_projected_fragment(
    fragment: &InlineFragment,
    fragment_index: usize,
    clean_range: Range<usize>,
    kinds: &[ExpandedInlineKind],
    link_group: Option<usize>,
    force_styled: bool,
    projected_fragments: &mut Vec<InlineFragment>,
    segments: &mut Vec<ExpandedInlineSegment>,
    clean_to_display_cursor: &mut [usize],
    display_to_clean: &mut Vec<usize>,
    display_cursor: &mut usize,
    any_expanded: &mut bool,
) {
    let fragment_len = fragment.text.len();

    for kind in kinds {
        *any_expanded = true;
        let marker = kind.open_marker().to_string();
        let marker_len = marker.len();
        let marker_style = marker_style_for_projection(fragment.style, *kind);
        projected_fragments.push(InlineFragment {
            text: marker,
            style: marker_style,
            html_style: fragment.html_style,
            link: None,
            footnote: None,
            math: None,
        });
        segments.push(ExpandedInlineSegment {
            display_range: *display_cursor..*display_cursor + marker_len,
            clean_range: clean_range.start..clean_range.start,
            fragment_index,
            link_group,
            kind: ExpandedInlineSegmentKind::OpeningDelimiter(*kind),
        });
        for _ in 0..marker_len {
            display_to_clean.push(clean_range.start);
        }
        *display_cursor += marker_len;
    }

    let text_segment_kind = if kinds.is_empty() && !force_styled {
        ExpandedInlineSegmentKind::PlainText
    } else {
        ExpandedInlineSegmentKind::StyledText
    };
    projected_fragments.push(fragment.clone());
    segments.push(ExpandedInlineSegment {
        display_range: *display_cursor..*display_cursor + fragment_len,
        clean_range: clean_range.clone(),
        fragment_index,
        link_group,
        kind: text_segment_kind,
    });
    for offset in 0..=fragment_len {
        clean_to_display_cursor[clean_range.start + offset] = *display_cursor + offset;
    }
    for offset in 1..=fragment_len {
        display_to_clean.push(clean_range.start + offset);
    }
    *display_cursor += fragment_len;

    for kind in kinds.iter().rev() {
        let marker = kind.close_marker().to_string();
        let marker_len = marker.len();
        let marker_style = marker_style_for_projection(fragment.style, *kind);
        projected_fragments.push(InlineFragment {
            text: marker,
            style: marker_style,
            html_style: fragment.html_style,
            link: None,
            footnote: None,
            math: None,
        });
        segments.push(ExpandedInlineSegment {
            display_range: *display_cursor..*display_cursor + marker_len,
            clean_range: clean_range.end..clean_range.end,
            fragment_index,
            link_group,
            kind: ExpandedInlineSegmentKind::ClosingDelimiter(*kind),
        });
        for _ in 0..marker_len {
            display_to_clean.push(clean_range.end);
        }
        *display_cursor += marker_len;
    }
}
