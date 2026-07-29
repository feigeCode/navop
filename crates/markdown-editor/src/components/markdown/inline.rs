//! Attribute-based inline Markdown tree for block titles and table cells.
//!
//! The runtime model stores only text fragments and formatting attributes.
//! Markdown markers are parsed at the I/O boundary and regenerated on save,
//! which keeps editing operations focused on text ranges instead of raw
//! delimiter strings.

use std::ops::Range;

use super::footnote::{
    InlineFootnoteHit, InlineFootnoteReference, parse_inline_footnote_reference,
    superscript_ordinal,
};
use super::html::{
    HtmlAttr, HtmlInlineStyle, HtmlNode, HtmlNodeKind, has_dangerous_attrs, is_inline_tag,
    parse_html_attrs, style_for_node,
};
use super::link::{LinkReferenceDefinition, LinkReferenceDefinitions, parse_link_target};

/// Bitfield of active inline formatting flags for a span of text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub script: InlineScript,
}

/// Vertical script style for simple Markdown extension syntax.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InlineScript {
    #[default]
    Normal,
    Superscript,
    Subscript,
}

impl InlineStyle {
    pub fn with_bold(self) -> Self {
        Self { bold: true, ..self }
    }

    pub fn with_italic(self) -> Self {
        Self {
            italic: true,
            ..self
        }
    }

    pub fn with_underline(self) -> Self {
        Self {
            underline: true,
            ..self
        }
    }

    pub fn with_strikethrough(self) -> Self {
        Self {
            strikethrough: true,
            ..self
        }
    }

    pub fn with_code(self) -> Self {
        Self { code: true, ..self }
    }

    pub fn with_superscript(self) -> Self {
        Self {
            script: InlineScript::Superscript,
            ..self
        }
    }

    pub fn with_subscript(self) -> Self {
        Self {
            script: InlineScript::Subscript,
            ..self
        }
    }

    pub fn has_script(self) -> bool {
        self.script != InlineScript::Normal
    }

    fn apply(self, delimiter: Delimiter) -> Self {
        match delimiter {
            Delimiter::BoldMarkdown { .. } | Delimiter::BoldHtml => self.with_bold(),
            Delimiter::ItalicMarkdown { .. } | Delimiter::ItalicHtml => self.with_italic(),
            Delimiter::Underline => self.with_underline(),
            Delimiter::StrikethroughMarkdown => self.with_strikethrough(),
            Delimiter::CodeMarkdown { .. } => self.with_code(),
            Delimiter::SuperscriptMarkdown | Delimiter::SuperscriptHtml => self.with_superscript(),
            Delimiter::SubscriptMarkdown | Delimiter::SubscriptHtml => self.with_subscript(),
        }
    }
}

/// A contiguous run of text with a uniform [`InlineStyle`].
///
/// The [`InlineTextTree`] is simply a `Vec<InlineFragment>` with
/// adjacent fragments of equal style merged during normalization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineFragment {
    pub text: String,
    pub style: InlineStyle,
    pub(crate) html_style: Option<HtmlInlineStyle>,
    pub link: Option<InlineLink>,
    pub footnote: Option<InlineFootnoteReference>,
    pub math: Option<InlineMath>,
}

/// Source-preserving inline LaTeX math metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineMath {
    /// Full Markdown source, including `$...$` or `\(...\)` delimiters.
    pub source: String,
    /// LaTeX body between the inline math delimiters.
    pub body: String,
    /// Delimiter form used by the source.
    pub delimiter: InlineMathDelimiter,
}

/// Supported inline math delimiter syntaxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineMathDelimiter {
    /// Dollar-delimited inline math: `$...$`.
    Dollar,
    /// Parenthesis-delimited inline math: `\(...\)`.
    Paren,
}

/// Link metadata attached to a formatted inline text fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineLink {
    /// Inline destination and optional title from `[label](destination "title")`.
    Inline {
        destination: String,
        title: Option<String>,
    },
    /// Reference-style link resolved from `[label][ref]`-style syntax.
    Reference { label: String, destination: String },
    /// Autolink target from `<scheme:target>` or email-like syntax.
    Autolink { target: String },
}

/// Link target pair used by hit-testing and open-link prompts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineLinkHit {
    pub prompt_target: String,
    pub open_target: String,
}

impl InlineLink {
    pub fn open_target(&self) -> &str {
        match self {
            Self::Inline { destination, .. } | Self::Reference { destination, .. } => destination,
            Self::Autolink { target } => target,
        }
    }

    pub fn raw_target(&self) -> &str {
        match self {
            Self::Inline { destination, .. } => destination,
            Self::Reference { label, .. } => label,
            Self::Autolink { target } => target,
        }
    }

    pub(crate) fn hit(&self) -> InlineLinkHit {
        InlineLinkHit {
            prompt_target: self.raw_target().to_string(),
            open_target: self.open_target().to_string(),
        }
    }

    pub(crate) fn is_source_preserving(&self) -> bool {
        matches!(self, Self::Reference { .. } | Self::Autolink { .. })
    }

    pub(crate) fn open_marker(&self) -> &'static str {
        match self {
            Self::Autolink { .. } => "<",
            Self::Inline { .. } | Self::Reference { .. } => "[",
        }
    }

    pub(crate) fn middle_marker(&self) -> Option<&'static str> {
        match self {
            Self::Inline { .. } => Some("]("),
            Self::Reference { .. } => Some("]["),
            Self::Autolink { .. } => None,
        }
    }

    pub(crate) fn editable_text(&self) -> Option<String> {
        match self {
            Self::Inline { destination, title } => {
                Some(format_inline_link_target(destination, title.as_deref()))
            }
            Self::Reference { label, .. } => Some(label.clone()),
            Self::Autolink { .. } => None,
        }
    }

    pub(crate) fn close_marker(&self) -> &'static str {
        match self {
            Self::Inline { .. } => ")",
            Self::Reference { .. } => "]",
            Self::Autolink { .. } => ">",
        }
    }
}

fn format_inline_link_target(destination: &str, title: Option<&str>) -> String {
    match title {
        Some(title) => format!("{destination} \"{}\"", escape_link_title(title)),
        None => destination.to_string(),
    }
}

fn escape_link_title(title: &str) -> String {
    let mut escaped = String::with_capacity(title.len());
    for ch in title.chars() {
        if matches!(ch, '\\' | '"') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// A cursor inside the inline text tree.
///
/// `fragment_index` identifies the fragment and `byte_offset` addresses a byte
/// boundary inside that fragment's text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextCursor {
    pub fragment_index: usize,
    pub byte_offset: usize,
}

/// A visible-text range with its associated [`InlineStyle`], used by
/// the render cache to build styled text runs for the text system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineSpan {
    pub range: Range<usize>,
    pub style: InlineStyle,
    pub(crate) html_style: Option<HtmlInlineStyle>,
    pub link: Option<InlineLinkHit>,
    pub footnote: Option<InlineFootnoteHit>,
    pub math: Option<InlineMath>,
}

/// Fragment attributes inherited by inserted text at a caret position.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InlineInsertionAttributes {
    pub style: InlineStyle,
    pub(crate) html_style: Option<HtmlInlineStyle>,
    pub link: Option<InlineLink>,
    pub footnote: Option<InlineFootnoteReference>,
    pub math: Option<InlineMath>,
}

/// Pre-computed view of an [`InlineTextTree`] optimized for rendering.
///
/// Flattens the fragment tree into a visible text string plus a list of
/// [`InlineSpan`]s.  Also maintains bidirectional mapping tables between
/// visible offsets and fragment positions, used by the IME subsystem.
#[derive(Clone, Debug, Default)]
pub struct InlineRenderCache {
    visible_text: String,
    spans: Vec<InlineSpan>,
    #[allow(dead_code)]
    visible_to_tree: Vec<TextCursor>,
    #[allow(dead_code)]
    tree_to_visible: Vec<usize>,
}

/// Bidirectional offset map between source Markdown and visible inline text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InlineMarkdownOffsetMap {
    markdown: String,
    visible_to_markdown: Vec<usize>,
    markdown_to_visible: Vec<usize>,
}

impl InlineMarkdownOffsetMap {
    pub(crate) fn markdown(&self) -> &str {
        &self.markdown
    }

    pub(crate) fn visible_to_markdown_offset(&self, offset: usize) -> usize {
        self.visible_to_markdown
            .get(offset.min(self.visible_to_markdown.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn visible_to_markdown_range(&self, range: Range<usize>) -> Range<usize> {
        self.visible_to_markdown_offset(range.start)..self.visible_to_markdown_offset(range.end)
    }

    pub(crate) fn markdown_to_visible_offset(&self, offset: usize) -> usize {
        self.markdown_to_visible
            .get(offset.min(self.markdown_to_visible.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn markdown_to_visible_range(&self, range: Range<usize>) -> Range<usize> {
        self.markdown_to_visible_offset(range.start)..self.markdown_to_visible_offset(range.end)
    }
}

impl InlineRenderCache {
    pub fn from_tree(tree: &InlineTextTree) -> Self {
        let mut visible_text = String::new();
        let mut spans = Vec::new();
        let mut visible_to_tree = vec![TextCursor::default(); tree.visible_len() + 1];
        let mut tree_to_visible = Vec::with_capacity(tree.fragments.len() + 1);
        let mut visible_offset = 0;

        for (fragment_index, fragment) in tree.fragments.iter().enumerate() {
            tree_to_visible.push(visible_offset);
            let fragment_start = visible_offset;
            visible_text.push_str(&fragment.text);
            let fragment_len = fragment.text.len();
            if fragment_len > 0 {
                spans.push(InlineSpan {
                    range: fragment_start..fragment_start + fragment_len,
                    style: fragment.style,
                    html_style: fragment.html_style,
                    link: fragment.link.as_ref().map(InlineLink::hit),
                    footnote: fragment
                        .footnote
                        .as_ref()
                        .and_then(InlineFootnoteReference::hit),
                    math: fragment.math.clone(),
                });
            }

            for byte_offset in 0..=fragment_len {
                visible_to_tree[fragment_start + byte_offset] = TextCursor {
                    fragment_index,
                    byte_offset,
                };
            }

            visible_offset += fragment_len;
        }

        tree_to_visible.push(visible_offset);
        if tree.fragments.is_empty() {
            visible_to_tree[0] = TextCursor::default();
        }

        Self {
            visible_text,
            spans,
            visible_to_tree,
            tree_to_visible,
        }
    }

    pub fn visible_text(&self) -> &str {
        &self.visible_text
    }

    pub fn spans(&self) -> &[InlineSpan] {
        &self.spans
    }

    pub fn visible_len(&self) -> usize {
        self.visible_text.len()
    }

    pub fn style_at(&self, offset: usize) -> InlineStyle {
        self.spans
            .iter()
            .find(|span| span.range.start <= offset && offset < span.range.end)
            .map(|span| span.style)
            .unwrap_or_default()
    }

    #[allow(dead_code)]
    pub(crate) fn html_style_at(&self, offset: usize) -> Option<HtmlInlineStyle> {
        self.spans
            .iter()
            .find(|span| span.range.start <= offset && offset < span.range.end)
            .and_then(|span| span.html_style)
    }

    #[allow(dead_code)]
    pub fn link_at(&self, offset: usize) -> Option<&str> {
        self.link_hit_at(offset).map(|hit| hit.open_target.as_str())
    }

    pub fn link_hit_at(&self, offset: usize) -> Option<&InlineLinkHit> {
        self.spans
            .iter()
            .find(|span| span.range.start <= offset && offset < span.range.end)
            .and_then(|span| span.link.as_ref())
    }

    #[allow(dead_code)]
    pub fn footnote_hit_at(&self, offset: usize) -> Option<&InlineFootnoteHit> {
        self.spans
            .iter()
            .find(|span| span.range.start <= offset && offset < span.range.end)
            .and_then(|span| span.footnote.as_ref())
    }

    #[allow(dead_code)]
    pub fn inline_math_at(&self, offset: usize) -> Option<&InlineMath> {
        self.spans
            .iter()
            .find(|span| span.range.start <= offset && offset < span.range.end)
            .and_then(|span| span.math.as_ref())
    }
}

/// A sequence of [`InlineFragment`]s representing inline-formatted text.
///
/// This is the core data structure for block titles.  It supports:
/// - Building from raw Markdown (auto-parsing bold/italic/underline markers)
/// - Bidirectional Markdown serialization with optimal delimiter choice
/// - Splitting at arbitrary byte offsets (used for Enter key, paste)
/// - Toggling inline styles on arbitrary ranges
///
/// The serialization uses a Viterbi-like DP optimization to choose between
/// Markdown and HTML delimiter variants, avoiding ambiguous `****` runs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InlineTextTree {
    pub(crate) fragments: Vec<InlineFragment>,
}

impl InlineTextTree {
    pub fn plain(text: impl Into<String>) -> Self {
        Self::from_fragments(vec![InlineFragment {
            text: text.into(),
            style: InlineStyle::default(),
            html_style: None,
            link: None,
            footnote: None,
            math: None,
        }])
    }

    /// Parse marker-based Markdown into the internal fragment representation.
    ///
    /// Markers (`**`, `*`, `<u>`, `<strong>`, `<em>`) are consumed and
    /// converted to [`InlineStyle`] flags on adjacent fragments.  The
    /// markers themselves are never stored — the tree holds only text
    /// content and style attributes.
    pub fn from_markdown(markdown: &str) -> Self {
        Self::from_markdown_with_link_references(markdown, &LinkReferenceDefinitions::default())
    }

    pub(crate) fn from_markdown_with_link_references(
        markdown: &str,
        reference_definitions: &LinkReferenceDefinitions,
    ) -> Self {
        let mut tree = Self::plain(markdown)
            .normalize_inline_syntax_with_link_references(reference_definitions)
            .tree;
        tree.normalize_code_spans();
        tree
    }

    /// Code-span content normalization:
    /// - CRLF/CR line endings are normalized to LF so inline code can render
    ///   across hard lines in the editor.
    /// - If the content is not entirely spaces and both starts AND ends with
    ///   a single space, those two spaces are stripped.
    fn normalize_code_spans(&mut self) {
        for fragment in &mut self.fragments {
            if fragment.style.code && !fragment.text.is_empty() {
                let mut s = fragment.text.replace("\r\n", "\n").replace('\r', "\n");
                let all_space = s.chars().all(|c| c == ' ');
                if !all_space && s.starts_with(' ') && s.ends_with(' ') {
                    s.remove(0);
                    s.pop();
                }
                fragment.text = s;
            }
        }
        self.normalize_fragments();
    }

    pub fn from_fragments(fragments: Vec<InlineFragment>) -> Self {
        let mut tree = Self { fragments };
        tree.normalize_fragments();
        tree
    }

    pub fn visible_text(&self) -> String {
        let mut text = String::new();
        for fragment in &self.fragments {
            text.push_str(&fragment.text);
        }
        text
    }

    pub fn visible_len(&self) -> usize {
        self.fragments
            .iter()
            .map(|fragment| fragment.text.len())
            .sum()
    }

    pub(crate) fn has_source_preserving_links(&self) -> bool {
        self.fragments.iter().any(|fragment| {
            fragment
                .link
                .as_ref()
                .is_some_and(InlineLink::is_source_preserving)
                || fragment.footnote.is_some()
                || fragment.math.is_some()
        })
    }

    /// Whether any fragment carries an inline `[label](url)` link. Unlike
    /// reference/autolink links these are not "source preserving", but their
    /// `[...](...)` markers are still stripped from the fragment text, so an
    /// edit that re-derives the tree from visible text alone would drop them.
    pub(crate) fn has_inline_links(&self) -> bool {
        self.fragments
            .iter()
            .any(|fragment| matches!(fragment.link, Some(InlineLink::Inline { .. })))
    }

    pub(crate) fn has_mixed_inline_visuals(&self) -> bool {
        self.fragments
            .iter()
            .any(|fragment| fragment.math.is_some() || fragment.style.has_script())
    }

    pub(crate) fn has_footnote_references(&self) -> bool {
        self.fragments
            .iter()
            .any(|fragment| fragment.footnote.is_some())
    }

    pub(crate) fn apply_footnote_reference_state(
        &mut self,
        mut resolve: impl FnMut(&str) -> Option<(usize, usize)>,
    ) {
        for fragment in &mut self.fragments {
            let Some(footnote) = fragment.footnote.as_mut() else {
                continue;
            };
            if let Some((ordinal, occurrence_index)) = resolve(&footnote.id) {
                footnote.ordinal = Some(ordinal);
                footnote.occurrence_index = occurrence_index;
                fragment.text = superscript_ordinal(ordinal);
            } else {
                footnote.ordinal = None;
                footnote.occurrence_index = 0;
                fragment.text = footnote.raw_markdown();
            }
        }
        self.normalize_fragments();
    }

    pub fn render_cache(&self) -> InlineRenderCache {
        InlineRenderCache::from_tree(self)
    }

    /// Serialize fragments back to Markdown text with optimal delimiter choices.
    ///
    /// Each fragment's style flags determine which markers surround its text.
    /// This is the export side of the I/O boundary; the internal fragment
    /// representation never stores raw marker characters.
    pub fn serialize_markdown(&self) -> String {
        self.markdown_offset_map().markdown
    }

    pub(crate) fn markdown_offset_map(&self) -> InlineMarkdownOffsetMap {
        if self.fragments.is_empty() {
            return InlineMarkdownOffsetMap {
                markdown: String::new(),
                visible_to_markdown: vec![0],
                markdown_to_visible: vec![0],
            };
        }

        let mut output = String::new();
        let mut visible_to_markdown = vec![0; self.visible_len() + 1];
        let mut markdown_to_visible = vec![0];
        let mut visible_cursor = 0usize;
        let mut index = 0usize;
        while index < self.fragments.len() {
            if let Some(footnote) = self.fragments[index].footnote.clone() {
                let raw_markdown = footnote.raw_markdown();
                let raw_len = raw_markdown.len();
                let run_visible_len = self.fragments[index].text.len();
                let run_start = output.len();
                output.push_str(&raw_markdown);
                let run_end = output.len();

                for local_visible in 0..=run_visible_len {
                    let mapped = if run_visible_len == 0 {
                        0
                    } else {
                        (raw_len * local_visible) / run_visible_len
                    };
                    visible_to_markdown[visible_cursor + local_visible] = run_start + mapped;
                }

                markdown_to_visible.resize(run_end + 1, visible_cursor);
                for local_markdown in 0..=raw_len {
                    let mapped = if raw_len == 0 {
                        0
                    } else {
                        (run_visible_len * local_markdown) / raw_len
                    };
                    markdown_to_visible[run_start + local_markdown] = visible_cursor + mapped;
                }

                visible_cursor += run_visible_len;
                index += 1;
                continue;
            }

            if let Some(math) = self.fragments[index].math.clone() {
                let raw_markdown = math.source;
                let raw_len = raw_markdown.len();
                let run_visible_len = self.fragments[index].text.len();
                let run_start = output.len();
                output.push_str(&raw_markdown);
                let run_end = output.len();

                for local_visible in 0..=run_visible_len {
                    visible_to_markdown[visible_cursor + local_visible] =
                        run_start + local_visible.min(raw_len);
                }

                markdown_to_visible.resize(run_end + 1, visible_cursor);
                for local_markdown in 0..=raw_len {
                    markdown_to_visible[run_start + local_markdown] =
                        visible_cursor + local_markdown.min(run_visible_len);
                }

                visible_cursor += run_visible_len;
                index += 1;
                continue;
            }

            let link = self.fragments[index].link.clone();
            let mut end = index + 1;
            while end < self.fragments.len()
                && self.fragments[end].link == link
                && self.fragments[end].footnote.is_none()
                && self.fragments[end].math.is_none()
            {
                end += 1;
            }

            let run_map =
                serialize_fragment_run_markdown_with_offset_map(&self.fragments[index..end]);
            if let Some(link) = link {
                let run_visible_len = run_map.visible_to_markdown.len().saturating_sub(1);
                let link_start = output.len();
                let editable_text = link.editable_text();
                output.push_str(link.open_marker());
                output.push_str(run_map.markdown());
                if let Some(middle_marker) = link.middle_marker() {
                    output.push_str(middle_marker);
                }
                if let Some(editable_text) = editable_text.as_deref() {
                    output.push_str(editable_text);
                }
                output.push_str(link.close_marker());
                let link_end = output.len();
                let label_markdown_start = link_start + link.open_marker().len();

                for local_visible in 0..=run_visible_len {
                    visible_to_markdown[visible_cursor + local_visible] =
                        label_markdown_start + run_map.visible_to_markdown_offset(local_visible);
                }

                markdown_to_visible.resize(link_end + 1, visible_cursor);
                for local in 0..=link.open_marker().len() {
                    markdown_to_visible[link_start + local] = visible_cursor;
                }
                for local_markdown in 0..run_map.markdown().len() {
                    markdown_to_visible[label_markdown_start + local_markdown] =
                        visible_cursor + run_map.markdown_to_visible_offset(local_markdown);
                }

                let label_markdown_end = label_markdown_start + run_map.markdown().len();
                markdown_to_visible[label_markdown_end] = visible_cursor + run_visible_len;

                let suffix_start = label_markdown_end;
                let suffix_len = link.middle_marker().map(str::len).unwrap_or(0)
                    + editable_text.as_ref().map(String::len).unwrap_or(0)
                    + link.close_marker().len();
                for local in 0..=suffix_len {
                    markdown_to_visible[suffix_start + local] = visible_cursor + run_visible_len;
                }
                visible_cursor += run_visible_len;
            } else {
                let run_start = output.len();
                output.push_str(run_map.markdown());
                let run_end = output.len();

                let run_visible_len = run_map.visible_to_markdown.len().saturating_sub(1);
                for local_visible in 0..=run_visible_len {
                    visible_to_markdown[visible_cursor + local_visible] =
                        run_start + run_map.visible_to_markdown_offset(local_visible);
                }

                markdown_to_visible.resize(run_end + 1, visible_cursor);
                for local_markdown in 0..=run_map.markdown().len() {
                    markdown_to_visible[run_start + local_markdown] =
                        visible_cursor + run_map.markdown_to_visible_offset(local_markdown);
                }
                visible_cursor += run_visible_len;
            }

            index = end;
        }

        InlineMarkdownOffsetMap {
            markdown: output,
            visible_to_markdown,
            markdown_to_visible,
        }
    }
}

fn serialize_fragment_run_markdown_with_offset_map(
    fragments: &[InlineFragment],
) -> InlineMarkdownOffsetMap {
    if fragments.is_empty() {
        return InlineMarkdownOffsetMap {
            markdown: String::new(),
            visible_to_markdown: vec![0],
            markdown_to_visible: vec![0],
        };
    }

    let stacks = choose_fragment_stacks(fragments);
    let mut output = String::new();
    let total_visible_len = fragments
        .iter()
        .map(|fragment| fragment.text.len())
        .sum::<usize>();
    let mut visible_to_markdown = vec![0; total_visible_len + 1];
    let mut markdown_to_visible = vec![0];
    let mut current_stack: Vec<Delimiter> = Vec::new();
    let mut current_html_style: Option<HtmlInlineStyle> = None;
    let mut visible_cursor = 0usize;

    for (fragment, next_stack) in fragments.iter().zip(stacks.iter()) {
        if current_html_style != fragment.html_style {
            let transition = stack_transition_string(&current_stack, &[]);
            push_markdown_marker(
                &mut output,
                &mut markdown_to_visible,
                visible_cursor,
                &transition,
            );
            current_stack.clear();

            if current_html_style.is_some() {
                push_markdown_marker(
                    &mut output,
                    &mut markdown_to_visible,
                    visible_cursor,
                    "</span>",
                );
            }
            if let Some(style) = fragment.html_style
                && let Some(marker) = html_style_open_marker(style)
            {
                push_markdown_marker(
                    &mut output,
                    &mut markdown_to_visible,
                    visible_cursor,
                    &marker,
                );
            }
            current_html_style = fragment.html_style;
        }

        let transition = stack_transition_string(&current_stack, next_stack);
        let transition_start = output.len();
        output.push_str(&transition);
        markdown_to_visible.resize(output.len() + 1, visible_cursor);
        for local in 0..=transition.len() {
            markdown_to_visible[transition_start + local] = visible_cursor;
        }

        let escaped = if let Some(math) = fragment.math.as_ref() {
            identity_text_with_offset_map(&math.source)
        } else if fragment.style.code {
            escape_code_span_text_with_offset_map(&fragment.text)
        } else {
            escape_literal_text_with_offset_map(&fragment.text)
        };
        let escaped_start = output.len();
        output.push_str(escaped.markdown());
        for local_visible in 0..=fragment.text.len() {
            visible_to_markdown[visible_cursor + local_visible] =
                escaped_start + escaped.visible_to_markdown_offset(local_visible);
        }
        markdown_to_visible.resize(output.len() + 1, visible_cursor);
        for local_markdown in 0..=escaped.markdown().len() {
            markdown_to_visible[escaped_start + local_markdown] =
                visible_cursor + escaped.markdown_to_visible_offset(local_markdown);
        }
        visible_cursor += fragment.text.len();
        current_stack = next_stack.clone();
    }

    let transition = stack_transition_string(&current_stack, &[]);
    push_markdown_marker(
        &mut output,
        &mut markdown_to_visible,
        visible_cursor,
        &transition,
    );
    if current_html_style.is_some() {
        push_markdown_marker(
            &mut output,
            &mut markdown_to_visible,
            visible_cursor,
            "</span>",
        );
    }

    InlineMarkdownOffsetMap {
        markdown: output,
        visible_to_markdown,
        markdown_to_visible,
    }
}

fn push_markdown_marker(
    output: &mut String,
    markdown_to_visible: &mut Vec<usize>,
    visible_cursor: usize,
    marker: &str,
) {
    if marker.is_empty() {
        return;
    }
    let marker_start = output.len();
    output.push_str(marker);
    markdown_to_visible.resize(output.len() + 1, visible_cursor);
    for local in 0..=marker.len() {
        markdown_to_visible[marker_start + local] = visible_cursor;
    }
}

fn identity_text_with_offset_map(text: &str) -> InlineMarkdownOffsetMap {
    InlineMarkdownOffsetMap {
        markdown: text.to_string(),
        visible_to_markdown: (0..=text.len()).collect(),
        markdown_to_visible: (0..=text.len()).collect(),
    }
}

fn html_style_open_marker(style: HtmlInlineStyle) -> Option<String> {
    style
        .to_css()
        .map(|css| format!("<span style=\"{}\">", escape_html_attr(&css)))
}

fn escape_html_attr(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

impl InlineTextTree {
    pub fn split_at(&self, offset: usize) -> (Self, Self) {
        let clamped = offset.min(self.visible_len());
        let mut left = Vec::new();
        let mut right = Vec::new();
        let mut consumed = 0;

        for fragment in &self.fragments {
            let fragment_len = fragment.text.len();
            let fragment_start = consumed;
            let fragment_end = fragment_start + fragment_len;

            if clamped <= fragment_start {
                right.push(fragment.clone());
            } else if clamped >= fragment_end {
                left.push(fragment.clone());
            } else {
                let split_offset = clamp_to_char_boundary(&fragment.text, clamped - fragment_start);
                if split_offset > 0 {
                    left.push(InlineFragment {
                        text: fragment.text[..split_offset].to_string(),
                        style: fragment.style,
                        html_style: fragment.html_style,
                        link: fragment.link.clone(),
                        footnote: fragment.footnote.clone(),
                        math: None,
                    });
                }
                if split_offset < fragment_len {
                    right.push(InlineFragment {
                        text: fragment.text[split_offset..].to_string(),
                        style: fragment.style,
                        html_style: fragment.html_style,
                        link: fragment.link.clone(),
                        footnote: fragment.footnote.clone(),
                        math: None,
                    });
                }
            }

            consumed = fragment_end;
        }

        (Self::from_fragments(left), Self::from_fragments(right))
    }

    pub fn append_tree(&mut self, other: Self) {
        self.fragments.extend(other.fragments);
        self.normalize_fragments();
    }

    pub(crate) fn replace_fragment_range(
        &mut self,
        range: Range<usize>,
        replacement: Vec<InlineFragment>,
    ) {
        self.fragments.splice(range, replacement);
        self.normalize_fragments();
    }

    pub fn remove_visible_prefix(&mut self, prefix_len: usize) {
        let (_, tail) = self.split_at(prefix_len);
        *self = tail;
    }

    pub fn attributes_for_insertion_at(&self, offset: usize) -> InlineInsertionAttributes {
        if self.fragments.is_empty() {
            return InlineInsertionAttributes::default();
        }

        let clamped = offset.min(self.visible_len());
        let mut consumed = 0;

        for (index, fragment) in self.fragments.iter().enumerate() {
            let fragment_len = fragment.text.len();
            let fragment_start = consumed;
            let fragment_end = fragment_start + fragment_len;

            if fragment_start < clamped && clamped < fragment_end {
                return InlineInsertionAttributes {
                    style: fragment.style,
                    html_style: fragment.html_style,
                    link: fragment.link.clone(),
                    footnote: fragment.footnote.clone(),
                    math: None,
                };
            }

            // Typing at a delimited-fragment boundary should produce plain
            // text, not extend the span past its visible closing/opening
            // marker when the caret is outside.
            if clamped == fragment_end && index + 1 == self.fragments.len() {
                return if fragment.style.code || fragment.style.strikethrough {
                    InlineInsertionAttributes::default()
                } else {
                    InlineInsertionAttributes {
                        style: fragment.style,
                        html_style: fragment.html_style,
                        link: fragment.link.clone(),
                        footnote: fragment.footnote.clone(),
                        math: None,
                    }
                };
            }

            if clamped == fragment_start && index == 0 {
                return if fragment.style.code || fragment.style.strikethrough {
                    InlineInsertionAttributes::default()
                } else {
                    InlineInsertionAttributes {
                        style: fragment.style,
                        html_style: fragment.html_style,
                        link: fragment.link.clone(),
                        footnote: fragment.footnote.clone(),
                        math: None,
                    }
                };
            }

            consumed = fragment_end;
        }

        InlineInsertionAttributes::default()
    }

    pub fn toggle_bold(&mut self, range: Range<usize>) -> bool {
        self.toggle_style(range, StyleFlag::Bold)
    }

    pub fn toggle_italic(&mut self, range: Range<usize>) -> bool {
        self.toggle_style(range, StyleFlag::Italic)
    }

    pub fn toggle_underline(&mut self, range: Range<usize>) -> bool {
        self.toggle_style(range, StyleFlag::Underline)
    }

    #[allow(dead_code)]
    pub fn toggle_strikethrough(&mut self, range: Range<usize>) -> bool {
        self.toggle_style(range, StyleFlag::Strikethrough)
    }

    pub fn toggle_code(&mut self, range: Range<usize>) -> bool {
        self.toggle_style(range, StyleFlag::Code)
    }

    pub(crate) fn unwrap_styles_on_fragments(&mut self, targets: &[(usize, StyleFlag)]) {
        if targets.is_empty() {
            return;
        }

        for (fragment_index, flag) in targets {
            if let Some(fragment) = self.fragments.get_mut(*fragment_index) {
                fragment.style = set_style_flag(fragment.style, *flag, false);
            }
        }
        self.normalize_fragments();
    }

    #[allow(dead_code)]
    pub fn replace_visible_range(
        &self,
        range: Range<usize>,
        new_text: &str,
        inserted_attributes: InlineInsertionAttributes,
    ) -> InlineEditResult {
        self.replace_visible_range_with_link_references(
            range,
            new_text,
            inserted_attributes,
            &LinkReferenceDefinitions::default(),
        )
    }

    pub(crate) fn replace_visible_range_with_link_references(
        &self,
        range: Range<usize>,
        new_text: &str,
        inserted_attributes: InlineInsertionAttributes,
        reference_definitions: &LinkReferenceDefinitions,
    ) -> InlineEditResult {
        let clamped_start = range.start.min(self.visible_len());
        let clamped_end = range.end.min(self.visible_len());
        let (before, tail) = self.split_at(clamped_start);
        let (_, after) = tail.split_at(clamped_end.saturating_sub(clamped_start));

        let mut temp = before;
        if !new_text.is_empty() {
            temp.fragments.push(InlineFragment {
                text: new_text.to_string(),
                style: inserted_attributes.style,
                html_style: inserted_attributes.html_style,
                link: inserted_attributes.link,
                footnote: inserted_attributes.footnote,
                math: inserted_attributes.math,
            });
        }
        temp.append_tree(after);
        temp.normalize_fragments();
        temp.normalize_inline_syntax_with_link_references(reference_definitions)
    }

    /// Like `replace_visible_range` but skips marker normalization so
    /// that backticks, stars, and other delimiters are stored as-is.
    /// Used for source-mode editing where the text must remain raw.
    pub fn replace_visible_range_raw(
        &self,
        range: Range<usize>,
        new_text: &str,
        inserted_attributes: InlineInsertionAttributes,
    ) -> InlineEditResult {
        let clamped_start = range.start.min(self.visible_len());
        let clamped_end = range.end.min(self.visible_len());
        let (before, tail) = self.split_at(clamped_start);
        let (_, after) = tail.split_at(clamped_end.saturating_sub(clamped_start));

        let mut temp = before;
        if !new_text.is_empty() {
            temp.fragments.push(InlineFragment {
                text: new_text.to_string(),
                style: inserted_attributes.style,
                html_style: inserted_attributes.html_style,
                link: inserted_attributes.link,
                footnote: inserted_attributes.footnote,
                math: inserted_attributes.math,
            });
        }
        temp.append_tree(after);
        temp.normalize_fragments();
        let len = temp.visible_len();
        InlineEditResult {
            tree: InlineTextTree::from_fragments(temp.fragments),
            visible_to_normalized: (0..=len).collect(),
        }
    }

    /// Core marker-to-style normalizer: scans the fragment text for
    /// delimiter sequences (`**`, `*`, `<u>`, etc.), removes them, and
    /// applies the corresponding [`InlineStyle`] to the text between
    /// matching pairs.  Unmatched delimiters are emitted as literal text.
    #[allow(dead_code)]
    pub fn normalize_inline_syntax(&self) -> InlineEditResult {
        self.normalize_inline_syntax_with_link_references(&LinkReferenceDefinitions::default())
    }

    pub(crate) fn normalize_inline_syntax_with_link_references(
        &self,
        reference_definitions: &LinkReferenceDefinitions,
    ) -> InlineEditResult {
        let visible_text = self.visible_text();
        let tokens = flatten_tokens(&self.fragments);
        let mut builder = NormalizeBuilder::new(visible_text.len());
        let _ = parse_until(
            &tokens,
            0,
            None,
            InlineStyle::default(),
            None,
            &mut builder,
            false,
            reference_definitions,
        );
        InlineEditResult {
            tree: InlineTextTree::from_fragments(builder.fragments),
            visible_to_normalized: builder.visible_to_normalized,
        }
    }

    fn toggle_style(&mut self, range: Range<usize>, flag: StyleFlag) -> bool {
        if range.is_empty() {
            return false;
        }

        let clamped_start = range.start.min(self.visible_len());
        let clamped_end = range.end.min(self.visible_len());
        if clamped_start >= clamped_end {
            return false;
        }

        let (before, tail) = self.split_at(clamped_start);
        let (mut middle, after) = tail.split_at(clamped_end - clamped_start);
        let should_remove = middle
            .fragments
            .iter()
            .all(|fragment| style_flag_enabled(fragment.style, flag));

        for fragment in &mut middle.fragments {
            fragment.style = set_style_flag(fragment.style, flag, !should_remove);
        }
        middle.normalize_fragments();

        let mut next = before;
        next.append_tree(middle);
        next.append_tree(after);
        *self = next;
        true
    }

    fn normalize_fragments(&mut self) {
        let mut normalized: Vec<InlineFragment> = Vec::new();
        for fragment in self.fragments.drain(..) {
            if fragment.text.is_empty() {
                continue;
            }

            if let Some(last) = normalized.last_mut()
                && last.style == fragment.style
                && last.html_style == fragment.html_style
                && last.link == fragment.link
                && last.footnote == fragment.footnote
                && last.math.is_none()
                && fragment.math.is_none()
            {
                last.text.push_str(&fragment.text);
                continue;
            }

            normalized.push(fragment);
        }
        self.fragments = normalized;
    }
}

/// Result of a visible-text replacement operation, containing the
/// normalized tree and a mapping from pre-edit visible offsets to
/// post-edit tree offsets.
#[derive(Clone, Debug)]
pub struct InlineEditResult {
    pub tree: InlineTextTree,
    visible_to_normalized: Vec<usize>,
}

impl InlineEditResult {
    pub fn map_offset(&self, offset: usize) -> usize {
        self.visible_to_normalized
            .get(offset.min(self.visible_to_normalized.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0)
    }

    pub fn map_range(&self, range: &Range<usize>) -> Range<usize> {
        self.map_offset(range.start)..self.map_offset(range.end)
    }
}

/// Ordered preference of delimiter variants used by the DP serializer.
/// Lower rank = more preferred.  Markdown delimiters are preferred over HTML
/// because they are shorter and more idiomatic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Delimiter {
    /// Markdown bold marker using either `*` or `_`.
    BoldMarkdown { marker: char },
    /// Markdown italic marker using either `*` or `_`.
    ItalicMarkdown { marker: char },
    /// Markdown strikethrough marker `~~`.
    StrikethroughMarkdown,
    /// Markdown superscript marker `^`.
    SuperscriptMarkdown,
    /// Markdown subscript marker `~`.
    SubscriptMarkdown,
    /// HTML underline marker `<u>`.
    Underline,
    /// HTML superscript marker `<sup>`.
    SuperscriptHtml,
    /// HTML subscript marker `<sub>`.
    SubscriptHtml,
    /// HTML bold marker `<strong>`.
    BoldHtml,
    /// HTML italic marker `<em>`.
    ItalicHtml,
    /// Markdown code span marker using a selected backtick run length.
    CodeMarkdown { run_len: usize },
}

impl Delimiter {
    /// Returns the opening marker string.  For code spans this is `run_len`
    /// backticks; for emphasis it's `**`, `*`, `<u>`, etc.
    fn open(self) -> String {
        match self {
            Self::BoldMarkdown { marker } => marker.to_string().repeat(2),
            Self::ItalicMarkdown { marker } => marker.to_string(),
            Self::StrikethroughMarkdown => "~~".into(),
            Self::SuperscriptMarkdown => "^".into(),
            Self::SubscriptMarkdown => "~".into(),
            Self::Underline => "<u>".into(),
            Self::SuperscriptHtml => "<sup>".into(),
            Self::SubscriptHtml => "<sub>".into(),
            Self::BoldHtml => "<strong>".into(),
            Self::ItalicHtml => "<em>".into(),
            Self::CodeMarkdown { run_len } => "`".repeat(run_len),
        }
    }

    fn close(self) -> String {
        match self {
            Self::BoldMarkdown { marker } => marker.to_string().repeat(2),
            Self::ItalicMarkdown { marker } => marker.to_string(),
            Self::StrikethroughMarkdown => "~~".into(),
            Self::SuperscriptMarkdown => "^".into(),
            Self::SubscriptMarkdown => "~".into(),
            Self::Underline => "</u>".into(),
            Self::SuperscriptHtml => "</sup>".into(),
            Self::SubscriptHtml => "</sub>".into(),
            Self::BoldHtml => "</strong>".into(),
            Self::ItalicHtml => "</em>".into(),
            Self::CodeMarkdown { run_len } => "`".repeat(run_len),
        }
    }

    fn token_len(self) -> usize {
        match self {
            Self::CodeMarkdown { run_len } => run_len,
            other => other.open().chars().count(),
        }
    }

    fn preference_rank(self) -> u8 {
        match self {
            Self::BoldMarkdown { .. } => 0,
            Self::Underline => 1,
            Self::StrikethroughMarkdown => 2,
            Self::SuperscriptMarkdown | Self::SubscriptMarkdown => 3,
            Self::ItalicMarkdown { .. } => 4,
            Self::SuperscriptHtml | Self::SubscriptHtml => 5,
            Self::BoldHtml => 6,
            Self::ItalicHtml => 7,
            Self::CodeMarkdown { .. } => 8,
        }
    }

    fn is_html(self) -> bool {
        matches!(
            self,
            Self::BoldHtml | Self::ItalicHtml | Self::SuperscriptHtml | Self::SubscriptHtml
        )
    }
}

/// Inline style flag addressable by editing commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StyleFlag {
    /// Bold text.
    Bold,
    /// Italic text.
    Italic,
    /// Underlined text.
    Underline,
    /// Strikethrough text.
    Strikethrough,
    /// Inline code text.
    Code,
    /// Superscript text.
    Superscript,
    /// Subscript text.
    Subscript,
}

/// Source character plus style and byte range used by inline parsing.
#[derive(Clone)]
struct CharToken {
    ch: char,
    style: InlineStyle,
    html_style: Option<HtmlInlineStyle>,
    source_range: Range<usize>,
}

/// Result of parsing a delimited inline region.
struct ParseResult {
    next_index: usize,
    closed: bool,
}

/// Builds the output fragments during normalization (marker parsing).
/// Keeps track of the visible-to-normalized offset mapping so that
/// selections and cursors can be mapped to the normalized tree.
struct NormalizeBuilder {
    fragments: Vec<InlineFragment>,
    visible_to_normalized: Vec<usize>,
    normalized_len: usize,
}

impl NormalizeBuilder {
    fn new(input_len: usize) -> Self {
        Self {
            fragments: Vec::new(),
            visible_to_normalized: vec![0; input_len + 1],
            normalized_len: 0,
        }
    }

    fn drop_token(&mut self, token: &CharToken) {
        for boundary in token.source_range.start..=token.source_range.end {
            self.visible_to_normalized[boundary] = self.normalized_len;
        }
    }

    fn emit_token(
        &mut self,
        token: &CharToken,
        extra_style: InlineStyle,
        html_style: Option<HtmlInlineStyle>,
    ) {
        let mut style = token.style;
        if extra_style.bold {
            style.bold = true;
        }
        if extra_style.italic {
            style.italic = true;
        }
        if extra_style.underline {
            style.underline = true;
        }
        if extra_style.strikethrough {
            style.strikethrough = true;
        }
        if extra_style.code {
            style.code = true;
        }
        if extra_style.has_script() {
            style.script = extra_style.script;
        }
        let html_style = merge_html_styles(html_style, token.html_style);

        let text = token.ch.to_string();
        let start = self.normalized_len;
        for boundary in token.source_range.start..=token.source_range.end {
            self.visible_to_normalized[boundary] = start + (boundary - token.source_range.start);
        }
        self.normalized_len += text.len();

        if let Some(last) = self.fragments.last_mut()
            && last.style == style
            && last.html_style == html_style
            && last.link.is_none()
            && last.footnote.is_none()
            && last.math.is_none()
        {
            last.text.push_str(&text);
            return;
        }

        self.fragments.push(InlineFragment {
            text,
            style,
            html_style,
            link: None,
            footnote: None,
            math: None,
        });
    }

    fn emit_inline_math(
        &mut self,
        tokens: &[CharToken],
        math: InlineMath,
        extra_style: InlineStyle,
        extra_html_style: Option<HtmlInlineStyle>,
    ) {
        let source_start = tokens
            .first()
            .map(|token| token.source_range.start)
            .unwrap_or(0);
        let normalized_start = self.normalized_len;
        let source = math.source.clone();
        let visible_len = source.len();

        for token in tokens {
            let token_len = token.source_range.len();
            for delta in 0..=token_len {
                self.visible_to_normalized[token.source_range.start + delta] =
                    normalized_start + (token.source_range.start + delta - source_start);
            }
        }

        self.normalized_len += visible_len;
        self.fragments.push(InlineFragment {
            text: source,
            style: extra_style,
            html_style: extra_html_style,
            link: None,
            footnote: None,
            math: Some(math),
        });
    }
}

fn flatten_tokens(fragments: &[InlineFragment]) -> Vec<CharToken> {
    let mut tokens = Vec::new();
    let mut visible_offset = 0;

    for fragment in fragments {
        for ch in fragment.text.chars() {
            let len = ch.len_utf8();
            tokens.push(CharToken {
                ch,
                style: fragment.style,
                html_style: fragment.html_style,
                source_range: visible_offset..visible_offset + len,
            });
            visible_offset += len;
        }
    }

    tokens
}

/// Recursive-descent parser that consumes [`CharToken`]s and reconstructs
/// the normalized inline tree.  Matching delimiters are consumed (dropped);
/// unmatched ones are emitted as literal text.  Nested styles are handled by
/// recursive calls that accumulate `extra_style`.
fn parse_until(
    tokens: &[CharToken],
    mut index: usize,
    end_delimiter: Option<Delimiter>,
    extra_style: InlineStyle,
    extra_html_style: Option<HtmlInlineStyle>,
    builder: &mut NormalizeBuilder,
    inside_code: bool,
    reference_definitions: &LinkReferenceDefinitions,
) -> ParseResult {
    let body_start = index;
    while index < tokens.len() {
        // Check for closing delimiter.
        if let Some(ref end_delim) = end_delimiter {
            let mut closed = match end_delim {
                Delimiter::CodeMarkdown { run_len } => {
                    tokens[index].ch == '`' && backtick_run_len(tokens, index) == *run_len
                }
                Delimiter::SuperscriptMarkdown => {
                    tokens[index].ch == '^' && can_close_emphasis(tokens, index)
                }
                Delimiter::SubscriptMarkdown => {
                    is_single_tilde_delimiter(tokens, index) && can_close_emphasis(tokens, index)
                }
                _ => {
                    matches_sequence(tokens, index, &end_delim.close())
                        && can_close_emphasis(tokens, index)
                }
            };

            // Emphasis spans must enclose at least one character; reject a
            // close at the very start of the body so empty spans stay literal.
            if closed && index == body_start && emphasis_requires_body(*end_delim) {
                closed = false;
            }

            if closed {
                let close_len = end_delim.close().chars().count();
                for token in &tokens[index..index + close_len] {
                    builder.drop_token(token);
                }
                return ParseResult {
                    next_index: index + close_len,
                    closed: true,
                };
            }
        }

        if !inside_code
            && let Some(next_index) =
                parse_inline_math(tokens, index, extra_style, extra_html_style, builder)
        {
            index = next_index;
            continue;
        }

        if !inside_code
            && tokens[index].ch == '\\'
            && let Some(escaped_len) = escaped_sequence_token_len(tokens, index)
        {
            builder.drop_token(&tokens[index]);
            let escaped_start = index + 1;
            let escaped_end = escaped_start + escaped_len;
            for token in &tokens[escaped_start..escaped_end] {
                builder.emit_token(token, extra_style, extra_html_style);
            }
            index = escaped_end;
            continue;
        }

        // Inside a code span, all text (including markers) is literal.
        if !inside_code {
            if tokens[index].ch == '['
                && let Some(next_index) =
                    parse_footnote_reference(tokens, index, extra_style, extra_html_style, builder)
            {
                index = next_index;
                continue;
            }

            if let Some(next_index) = parse_inline_link(
                tokens,
                index,
                extra_style,
                extra_html_style,
                builder,
                reference_definitions,
            ) {
                index = next_index;
                continue;
            }

            if tokens[index].ch == '<'
                && let Some(next_index) = parse_inline_html_container(
                    tokens,
                    index,
                    extra_style,
                    extra_html_style,
                    builder,
                    reference_definitions,
                )
            {
                index = next_index;
                continue;
            }

            if tokens[index].ch == '<'
                && let Some(next_index) = parse_autolink(
                    tokens,
                    index,
                    extra_style,
                    extra_html_style,
                    builder,
                    reference_definitions,
                )
            {
                index = next_index;
                continue;
            }

            if let Some(delimiter) = match_open_delimiter(tokens, index) {
                if has_closing_delimiter(tokens, index, delimiter) {
                    for token in &tokens[index..index + delimiter.token_len()] {
                        builder.drop_token(token);
                    }
                    let inner_start = index + delimiter.token_len();
                    let is_code_delim = matches!(delimiter, Delimiter::CodeMarkdown { .. });
                    let parsed = parse_until(
                        tokens,
                        inner_start,
                        Some(delimiter),
                        extra_style.apply(delimiter),
                        extra_html_style,
                        builder,
                        is_code_delim,
                        reference_definitions,
                    );
                    if parsed.closed {
                        index = parsed.next_index;
                        continue;
                    }
                } else if delimiter.token_len() > 1 {
                    // Keep an unclosed multi-character opener (`**`, `__`, `~~`,
                    // backtick run) literal as one unit. Emitting just its first
                    // char would let the rest open a shorter span (e.g. `**bold*`
                    // -> `*` + italic `bold`), which is committed on every
                    // keystroke and loses the intended bold.
                    for token in &tokens[index..index + delimiter.token_len()] {
                        builder.emit_token(token, extra_style, extra_html_style);
                    }
                    index += delimiter.token_len();
                    continue;
                }
            }
        }

        builder.emit_token(&tokens[index], extra_style, extra_html_style);
        index += 1;
    }

    ParseResult {
        next_index: tokens.len(),
        closed: false,
    }
}

fn parse_inline_math(
    tokens: &[CharToken],
    index: usize,
    extra_style: InlineStyle,
    extra_html_style: Option<HtmlInlineStyle>,
    builder: &mut NormalizeBuilder,
) -> Option<usize> {
    let (body_start, close_start, close_end, delimiter) = if tokens.get(index)?.ch == '$' {
        if matches_sequence(tokens, index, "$$") || token_is_backslash_escaped(tokens, index) {
            return None;
        }
        let close = locate_inline_dollar_math_close(tokens, index + 1)?;
        (index + 1, close, close, InlineMathDelimiter::Dollar)
    } else if matches_sequence(tokens, index, "\\(") {
        let close = locate_inline_paren_math_close(tokens, index + 2)?;
        (index + 2, close, close + 1, InlineMathDelimiter::Paren)
    } else {
        return None;
    };

    if body_start >= close_start {
        return None;
    }
    if tokens[body_start..close_start]
        .iter()
        .any(|token| token.ch == '\n' || token.ch == '\r')
    {
        return None;
    }
    if tokens[body_start].ch.is_whitespace() || tokens[close_start - 1].ch.is_whitespace() {
        return None;
    }

    let source = tokens_to_string(&tokens[index..=close_end]);
    let body = tokens_to_string(&tokens[body_start..close_start]);
    if looks_like_obvious_currency(tokens, index, close_end, &body) {
        return None;
    }

    let math = InlineMath {
        source,
        body,
        delimiter,
    };
    builder.emit_inline_math(
        &tokens[index..=close_end],
        math,
        extra_style,
        extra_html_style,
    );
    Some(close_end + 1)
}

fn locate_inline_dollar_math_close(tokens: &[CharToken], mut cursor: usize) -> Option<usize> {
    while cursor < tokens.len() {
        let token = &tokens[cursor];
        if token.ch == '\n' || token.ch == '\r' {
            return None;
        }
        if token.ch == '$'
            && !token_is_backslash_escaped(tokens, cursor)
            && !matches_sequence(tokens, cursor, "$$")
        {
            return Some(cursor);
        }
        cursor += 1;
    }
    None
}

fn locate_inline_paren_math_close(tokens: &[CharToken], mut cursor: usize) -> Option<usize> {
    while cursor + 1 < tokens.len() {
        if tokens[cursor].ch == '\n' || tokens[cursor].ch == '\r' {
            return None;
        }
        if matches_sequence(tokens, cursor, "\\)") {
            return Some(cursor);
        }
        cursor += 1;
    }
    None
}

fn token_is_backslash_escaped(tokens: &[CharToken], index: usize) -> bool {
    if index == 0 {
        return false;
    }
    let mut cursor = index;
    let mut slash_count = 0usize;
    while cursor > 0 && tokens[cursor - 1].ch == '\\' {
        slash_count += 1;
        cursor -= 1;
    }
    slash_count % 2 == 1
}

fn looks_like_obvious_currency(
    tokens: &[CharToken],
    open_index: usize,
    close_index: usize,
    body: &str,
) -> bool {
    let prev_is_digit = open_index
        .checked_sub(1)
        .and_then(|idx| tokens.get(idx))
        .is_some_and(|token| token.ch.is_ascii_digit());
    let next_is_digit = tokens
        .get(close_index + 1)
        .is_some_and(|token| token.ch.is_ascii_digit());
    if prev_is_digit || next_is_digit {
        return true;
    }

    body.chars()
        .all(|ch| ch.is_ascii_digit() || matches!(ch, '.' | ',' | '_'))
        && body.chars().any(|ch| ch.is_ascii_digit())
        && body.len() > 1
}

fn parse_footnote_reference(
    tokens: &[CharToken],
    index: usize,
    extra_style: InlineStyle,
    extra_html_style: Option<HtmlInlineStyle>,
    builder: &mut NormalizeBuilder,
) -> Option<usize> {
    if tokens.get(index)?.ch != '[' || tokens.get(index + 1)?.ch != '^' {
        return None;
    }

    let mut cursor = index + 2;
    let end_index = loop {
        let token = tokens.get(cursor)?;
        if token.ch == '\\' {
            cursor += 2;
            continue;
        }
        if token.ch == ']' {
            break cursor;
        }
        cursor += 1;
    };

    let raw_markdown = tokens_to_string(&tokens[index..=end_index]);
    let id = parse_inline_footnote_reference(&raw_markdown)?;
    let fragments = vec![InlineFragment {
        text: raw_markdown.clone(),
        style: extra_style,
        html_style: extra_html_style,
        link: None,
        footnote: Some(InlineFootnoteReference {
            id,
            ordinal: None,
            occurrence_index: 0,
        }),
        math: None,
    }];

    let normalized_start = builder.normalized_len;
    let visible_len = raw_markdown.len();
    let normalized_end = normalized_start + visible_len;
    for token in &tokens[index..=end_index] {
        let token_len = token.source_range.len();
        for delta in 0..=token_len {
            builder.visible_to_normalized[token.source_range.start + delta] = normalized_start
                + (token.source_range.start + delta - tokens[index].source_range.start);
        }
    }

    for fragment in fragments {
        builder.normalized_len += fragment.text.len();
        if let Some(last) = builder.fragments.last_mut()
            && last.style == fragment.style
            && last.html_style == fragment.html_style
            && last.link == fragment.link
            && last.footnote == fragment.footnote
            && last.math.is_none()
            && fragment.math.is_none()
        {
            last.text.push_str(&fragment.text);
        } else {
            builder.fragments.push(fragment);
        }
    }

    for boundary in tokens[end_index].source_range.end..=tokens[end_index].source_range.end {
        builder.visible_to_normalized[boundary] = normalized_end;
    }

    Some(end_index + 1)
}

fn parse_inline_link(
    tokens: &[CharToken],
    index: usize,
    extra_style: InlineStyle,
    extra_html_style: Option<HtmlInlineStyle>,
    builder: &mut NormalizeBuilder,
    reference_definitions: &LinkReferenceDefinitions,
) -> Option<usize> {
    let located = locate_inline_link(tokens, index, reference_definitions)?;
    let label_end = located.label_end;
    let label_tokens = &tokens[index + 1..label_end];
    let label_markdown = tokens_to_string(label_tokens);
    let mut label_result = InlineTextTree::plain(label_markdown)
        .normalize_inline_syntax_with_link_references(reference_definitions);
    apply_extra_style_to_fragments(
        &mut label_result.tree.fragments,
        extra_style,
        extra_html_style,
    );
    let link = located.link;

    let normalized_start = builder.normalized_len;
    let label_len = label_result.tree.visible_len();

    for boundary in tokens[index].source_range.start..=tokens[index].source_range.end {
        builder.visible_to_normalized[boundary] = normalized_start;
    }

    let mut local_boundary = 0usize;
    for token in label_tokens {
        let token_len = token.source_range.len();
        for delta in 0..=token_len {
            builder.visible_to_normalized[token.source_range.start + delta] =
                normalized_start + label_result.visible_to_normalized[local_boundary + delta];
        }
        local_boundary += token_len;
    }

    let normalized_end = normalized_start + label_len;
    for token in &tokens[label_end..=located.end_index] {
        for boundary in token.source_range.start..=token.source_range.end {
            builder.visible_to_normalized[boundary] = normalized_end;
        }
    }

    for mut fragment in label_result.tree.fragments {
        fragment.link = Some(link.clone());
        fragment.footnote = None;
        fragment.math = None;
        builder.normalized_len += fragment.text.len();
        if let Some(last) = builder.fragments.last_mut()
            && last.style == fragment.style
            && last.html_style == fragment.html_style
            && last.link == fragment.link
            && last.footnote == fragment.footnote
            && last.math.is_none()
            && fragment.math.is_none()
        {
            last.text.push_str(&fragment.text);
        } else {
            builder.fragments.push(fragment);
        }
    }

    Some(located.end_index + 1)
}

fn parse_autolink(
    tokens: &[CharToken],
    index: usize,
    extra_style: InlineStyle,
    extra_html_style: Option<HtmlInlineStyle>,
    builder: &mut NormalizeBuilder,
    _reference_definitions: &LinkReferenceDefinitions,
) -> Option<usize> {
    let end_index = locate_autolink(tokens, index)?;
    let target_tokens = &tokens[index + 1..end_index];
    let target = tokens_to_string(target_tokens);
    let fragments = vec![InlineFragment {
        text: target.clone(),
        style: extra_style,
        html_style: extra_html_style,
        link: Some(InlineLink::Autolink {
            target: target.clone(),
        }),
        footnote: None,
        math: None,
    }];

    let normalized_start = builder.normalized_len;
    let target_len = target.len();

    for boundary in tokens[index].source_range.start..=tokens[index].source_range.end {
        builder.visible_to_normalized[boundary] = normalized_start;
    }

    let mut local_boundary = 0usize;
    for token in target_tokens {
        let token_len = token.source_range.len();
        for delta in 0..=token_len {
            builder.visible_to_normalized[token.source_range.start + delta] =
                normalized_start + local_boundary + delta;
        }
        local_boundary += token_len;
    }

    let normalized_end = normalized_start + target_len;
    for boundary in tokens[end_index].source_range.start..=tokens[end_index].source_range.end {
        builder.visible_to_normalized[boundary] = normalized_end;
    }

    for fragment in fragments {
        builder.normalized_len += fragment.text.len();
        if let Some(last) = builder.fragments.last_mut()
            && last.style == fragment.style
            && last.html_style == fragment.html_style
            && last.link == fragment.link
            && last.footnote == fragment.footnote
            && last.math.is_none()
            && fragment.math.is_none()
        {
            last.text.push_str(&fragment.text);
        } else {
            builder.fragments.push(fragment);
        }
    }

    Some(end_index + 1)
}

#[derive(Clone, Debug)]
struct InlineHtmlTag {
    name: String,
    attrs: Vec<HtmlAttr>,
    end_index: usize,
    self_closing: bool,
}

fn parse_inline_html_container(
    tokens: &[CharToken],
    index: usize,
    extra_style: InlineStyle,
    extra_html_style: Option<HtmlInlineStyle>,
    builder: &mut NormalizeBuilder,
    reference_definitions: &LinkReferenceDefinitions,
) -> Option<usize> {
    let tag = locate_inline_html_open_tag(tokens, index)?;
    if tag.self_closing || !is_inline_tag(&tag.name) || has_dangerous_attrs(&tag.attrs) {
        return None;
    }

    let (close_start, close_end) =
        locate_matching_inline_html_close(tokens, tag.end_index + 1, &tag.name)?;
    let tag_style = inline_html_semantic_style(&tag.name, extra_style);
    let html_style = merge_html_styles(extra_html_style, inline_html_style(&tag));
    if tag_style == extra_style && html_style == extra_html_style {
        return None;
    }

    for token in &tokens[index..=tag.end_index] {
        builder.drop_token(token);
    }
    let _ = parse_until(
        &tokens[tag.end_index + 1..close_start],
        0,
        None,
        tag_style,
        html_style,
        builder,
        false,
        reference_definitions,
    );
    for token in &tokens[close_start..=close_end] {
        builder.drop_token(token);
    }

    Some(close_end + 1)
}

fn inline_html_semantic_style(name: &str, style: InlineStyle) -> InlineStyle {
    match name {
        "strong" | "b" => style.with_bold(),
        "em" | "i" => style.with_italic(),
        "u" | "ins" => style.with_underline(),
        "del" => style.with_strikethrough(),
        "code" | "kbd" => style.with_code(),
        "sup" => style.with_superscript(),
        "sub" => style.with_subscript(),
        _ => style,
    }
}

fn inline_html_style(tag: &InlineHtmlTag) -> Option<HtmlInlineStyle> {
    let node = HtmlNode {
        kind: HtmlNodeKind::InlineSemantic,
        tag_name: tag.name.clone(),
        attrs: tag.attrs.clone(),
        children: Vec::new(),
        raw_source: String::new(),
        source_range: 0..0,
    };
    let style = style_for_node(&node);
    (!style.is_empty()).then_some(style)
}

fn merge_html_styles(
    parent: Option<HtmlInlineStyle>,
    child: Option<HtmlInlineStyle>,
) -> Option<HtmlInlineStyle> {
    let mut merged = parent.unwrap_or_default();
    if let Some(child) = child {
        if child.color.is_some() {
            merged.color = child.color;
        }
        if child.background_color.is_some() {
            merged.background_color = child.background_color;
        }
        if child.font_size.is_some() {
            merged.font_size = child.font_size;
        }
    }

    (!merged.is_empty()).then_some(merged)
}

/// Located inline link syntax inside the token stream.
#[derive(Clone)]
struct LocatedInlineLink {
    label_end: usize,
    end_index: usize,
    link: InlineLink,
}

fn locate_inline_link(
    tokens: &[CharToken],
    index: usize,
    reference_definitions: &LinkReferenceDefinitions,
) -> Option<LocatedInlineLink> {
    if tokens.get(index)?.ch != '[' {
        return None;
    }
    if index > 0 && matches!(tokens[index - 1].ch, '!' | ']') {
        return None;
    }

    let mut label_depth = 0usize;
    let mut cursor = index + 1;
    let label_end = loop {
        let token = tokens.get(cursor)?;
        if token.ch == '\\' {
            cursor += 2;
            continue;
        }

        match token.ch {
            '[' => label_depth += 1,
            ']' if label_depth == 0 => break cursor,
            ']' => label_depth = label_depth.saturating_sub(1),
            _ => {}
        }
        cursor += 1;
    };

    match tokens.get(label_end + 1).map(|token| token.ch) {
        Some('(') => {
            let url_start = label_end + 2;
            let mut paren_depth = 0usize;
            cursor = url_start;
            let url_end = loop {
                let token = tokens.get(cursor)?;
                if token.ch == '\\' {
                    cursor += 2;
                    continue;
                }

                match token.ch {
                    '(' => paren_depth += 1,
                    ')' if paren_depth == 0 => break cursor,
                    ')' => paren_depth = paren_depth.saturating_sub(1),
                    _ => {}
                }
                cursor += 1;
            };

            // An empty destination such as in `[label]()` is a valid link, but the
            // target parser rejects an empty string. Recognizing it keeps the caret
            // inside the projected link while the destination is filled in.
            let (destination, title) = if url_start == url_end {
                (String::new(), None)
            } else {
                parse_link_target(&tokens_to_string(&tokens[url_start..url_end]))?
            };
            Some(LocatedInlineLink {
                label_end,
                end_index: url_end,
                link: InlineLink::Inline { destination, title },
            })
        }
        Some('[') => {
            let reference_start = label_end + 2;
            cursor = reference_start;
            let reference_end = loop {
                let token = tokens.get(cursor)?;
                if token.ch == '\\' {
                    cursor += 2;
                    continue;
                }
                if token.ch == ']' {
                    break cursor;
                }
                cursor += 1;
            };

            let raw_label = tokens_to_string(&tokens[reference_start..reference_end]);
            let link_label = if raw_label.is_empty() {
                tokens_to_string(&tokens[index + 1..label_end])
            } else {
                raw_label
            };
            let normalized_label = super::image::normalize_reference_label(&link_label)?;
            let LinkReferenceDefinition { destination, .. } =
                reference_definitions.get(&normalized_label)?.clone();
            Some(LocatedInlineLink {
                label_end,
                end_index: reference_end,
                link: InlineLink::Reference {
                    label: link_label,
                    destination,
                },
            })
        }
        _ => {
            let raw_label = tokens_to_string(&tokens[index + 1..label_end]);
            let normalized_label = super::image::normalize_reference_label(&raw_label)?;
            let LinkReferenceDefinition { destination, .. } =
                reference_definitions.get(&normalized_label)?.clone();
            Some(LocatedInlineLink {
                label_end,
                end_index: label_end,
                link: InlineLink::Reference {
                    label: raw_label,
                    destination,
                },
            })
        }
    }
}

fn locate_autolink(tokens: &[CharToken], index: usize) -> Option<usize> {
    if tokens.get(index)?.ch != '<' {
        return None;
    }

    let mut cursor = index + 1;
    let end_index = loop {
        let token = tokens.get(cursor)?;
        if token.ch == '\\' {
            cursor += 2;
            continue;
        }
        if token.ch == '>' {
            break cursor;
        }
        cursor += 1;
    };

    let target = tokens_to_string(&tokens[index + 1..end_index]);
    (!target.is_empty() && !looks_like_non_autolink_html_tag(tokens, end_index, &target))
        .then_some(end_index)
}

fn tokens_to_string(tokens: &[CharToken]) -> String {
    tokens.iter().map(|token| token.ch).collect()
}

fn locate_inline_html_open_tag(tokens: &[CharToken], index: usize) -> Option<InlineHtmlTag> {
    if tokens.get(index)?.ch != '<' {
        return None;
    }

    let mut cursor = index + 1;
    if !tokens.get(cursor)?.ch.is_ascii_alphabetic() {
        return None;
    }
    let name_start = cursor;
    while cursor < tokens.len() && is_html_tag_name_char(tokens[cursor].ch) {
        cursor += 1;
    }
    let name = tokens_to_string(&tokens[name_start..cursor]).to_ascii_lowercase();

    match tokens.get(cursor).map(|token| token.ch) {
        Some(ch) if ch.is_whitespace() || ch == '>' || ch == '/' => {}
        _ => return None,
    }

    let attrs_start = cursor;
    let mut quote = None;
    while cursor < tokens.len() {
        let ch = tokens[cursor].ch;
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            cursor += 1;
            continue;
        }

        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            cursor += 1;
            continue;
        }

        if ch == '>' {
            let attrs_source = tokens_to_string(&tokens[attrs_start..cursor]);
            let self_closing = attrs_source.trim_end().ends_with('/');
            return Some(InlineHtmlTag {
                name,
                attrs: parse_html_attrs(&attrs_source),
                end_index: cursor,
                self_closing,
            });
        }

        cursor += 1;
    }

    None
}

fn locate_inline_html_close_tag(
    tokens: &[CharToken],
    index: usize,
    expected_name: &str,
) -> Option<usize> {
    if tokens.get(index)?.ch != '<' || tokens.get(index + 1)?.ch != '/' {
        return None;
    }

    let mut cursor = index + 2;
    while tokens
        .get(cursor)
        .is_some_and(|token| token.ch.is_whitespace())
    {
        cursor += 1;
    }
    let name_start = cursor;
    while cursor < tokens.len() && is_html_tag_name_char(tokens[cursor].ch) {
        cursor += 1;
    }
    if name_start == cursor {
        return None;
    }
    let name = tokens_to_string(&tokens[name_start..cursor]).to_ascii_lowercase();
    if name != expected_name {
        return None;
    }
    while tokens
        .get(cursor)
        .is_some_and(|token| token.ch.is_whitespace())
    {
        cursor += 1;
    }
    (tokens.get(cursor)?.ch == '>').then_some(cursor)
}

fn locate_matching_inline_html_close(
    tokens: &[CharToken],
    mut cursor: usize,
    name: &str,
) -> Option<(usize, usize)> {
    let mut depth = 1usize;
    while cursor < tokens.len() {
        if tokens[cursor].ch != '<' {
            cursor += 1;
            continue;
        }

        if let Some(close_end) = locate_inline_html_close_tag(tokens, cursor, name) {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some((cursor, close_end));
            }
            cursor = close_end + 1;
            continue;
        }

        if let Some(open) = locate_inline_html_open_tag(tokens, cursor) {
            if open.name == name && !open.self_closing {
                depth += 1;
            }
            cursor = open.end_index + 1;
            continue;
        }

        cursor += 1;
    }

    None
}

fn looks_like_non_autolink_html_tag(tokens: &[CharToken], end_index: usize, target: &str) -> bool {
    let target = target.trim();
    if target.starts_with('/') {
        let rest = target.trim_start_matches('/').trim();
        return html_tag_name_with_attrs(rest).is_some();
    }

    if let Some((_tag_name, has_attrs_or_slash)) = html_tag_name_with_attrs(target)
        && has_attrs_or_slash
    {
        return true;
    }

    let Some((tag_name, _)) = html_tag_name_with_attrs(target) else {
        return false;
    };
    let rest = tokens_to_string(&tokens[end_index + 1..]).to_ascii_lowercase();
    let tag_name = tag_name.to_ascii_lowercase();
    rest.contains(&format!("</{tag_name}>"))
}

fn html_tag_name_with_attrs(target: &str) -> Option<(&str, bool)> {
    if target.is_empty() {
        return None;
    }

    let first = target.as_bytes()[0];
    if !first.is_ascii_alphabetic() {
        return None;
    }

    let mut end = 0usize;
    for (index, ch) in target.char_indices() {
        if is_html_tag_name_char(ch) {
            end = index + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }

    let raw_rest = &target[end..];
    let rest = raw_rest.trim();
    if rest.is_empty() {
        return Some((&target[..end], false));
    }
    (raw_rest.chars().next().is_some_and(|ch| ch.is_whitespace())
        || rest == "/"
        || rest.starts_with('/'))
    .then_some((&target[..end], true))
}

fn is_html_tag_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')
}

fn apply_extra_style_to_fragments(
    fragments: &mut [InlineFragment],
    extra_style: InlineStyle,
    extra_html_style: Option<HtmlInlineStyle>,
) {
    for fragment in fragments {
        if extra_style.bold {
            fragment.style.bold = true;
        }
        if extra_style.italic {
            fragment.style.italic = true;
        }
        if extra_style.underline {
            fragment.style.underline = true;
        }
        if extra_style.strikethrough {
            fragment.style.strikethrough = true;
        }
        if extra_style.code {
            fragment.style.code = true;
        }
        if extra_style.has_script() {
            fragment.style.script = extra_style.script;
        }
        fragment.html_style = merge_html_styles(extra_html_style, fragment.html_style);
    }
}

fn match_open_delimiter(tokens: &[CharToken], index: usize) -> Option<Delimiter> {
    if matches_sequence(tokens, index, "<strong>") {
        Some(Delimiter::BoldHtml)
    } else if matches_sequence(tokens, index, "<em>") {
        Some(Delimiter::ItalicHtml)
    } else if matches_sequence(tokens, index, "<u>") {
        Some(Delimiter::Underline)
    } else if matches_sequence(tokens, index, "~~") {
        Some(Delimiter::StrikethroughMarkdown)
    } else if matches_sequence(tokens, index, "^") && can_open_script(tokens, index, '^') {
        Some(Delimiter::SuperscriptMarkdown)
    } else if is_single_tilde_delimiter(tokens, index) && can_open_script(tokens, index, '~') {
        Some(Delimiter::SubscriptMarkdown)
    } else if matches_sequence(tokens, index, "**") && can_open_emphasis(tokens, index, 2) {
        Some(Delimiter::BoldMarkdown { marker: '*' })
    } else if matches_sequence(tokens, index, "__") && can_open_emphasis(tokens, index, 2) {
        Some(Delimiter::BoldMarkdown { marker: '_' })
    } else if matches_sequence(tokens, index, "*") && can_open_emphasis(tokens, index, 1) {
        Some(Delimiter::ItalicMarkdown { marker: '*' })
    } else if matches_sequence(tokens, index, "_") && can_open_emphasis(tokens, index, 1) {
        Some(Delimiter::ItalicMarkdown { marker: '_' })
    } else if tokens[index].ch == '`' {
        // Count the run of consecutive backticks.
        let run_len = backtick_run_len(tokens, index);
        // A backtick run is only a valid opener if it is NOT immediately
        // followed by another backtick (no double-counting).
        if run_len > 0 {
            Some(Delimiter::CodeMarkdown { run_len })
        } else {
            None
        }
    } else {
        None
    }
}

/// Returns the length of the consecutive backtick run starting at `index`.
fn backtick_run_len(tokens: &[CharToken], index: usize) -> usize {
    let mut len = 0;
    while index + len < tokens.len() && tokens[index + len].ch == '`' {
        len += 1;
    }
    // A backtick run is only valid if it's not immediately preceded by an
    // additional backtick (the run must start at `index`).
    if index > 0 && tokens[index - 1].ch == '`' {
        return 0;
    }
    len
}

fn has_closing_delimiter(tokens: &[CharToken], index: usize, delimiter: Delimiter) -> bool {
    let skip = delimiter.token_len();
    let close_str = delimiter.close();

    // For code spans we look for a matching-length backtick run;
    // for emphasis we just scan for the close string.
    if let Delimiter::CodeMarkdown { .. } = delimiter {
        let mut cursor = index + skip;
        while cursor < tokens.len() {
            if tokens[cursor].ch == '\\'
                && let Some(escaped_len) = escaped_sequence_token_len(tokens, cursor)
            {
                cursor += 1 + escaped_len;
                continue;
            }

            if tokens[cursor].ch == '`' && backtick_run_len(tokens, cursor) == skip {
                return true;
            }

            cursor += 1;
        }
        return false;
    }

    if matches!(
        delimiter,
        Delimiter::SuperscriptMarkdown | Delimiter::SubscriptMarkdown
    ) {
        let marker = match delimiter {
            Delimiter::SuperscriptMarkdown => '^',
            Delimiter::SubscriptMarkdown => '~',
            _ => unreachable!(),
        };
        return locate_script_close(tokens, index + skip, marker).is_some();
    }

    let body_start = index + skip;
    let requires_body = emphasis_requires_body(delimiter);
    let mut cursor = body_start;
    while cursor < tokens.len() {
        if tokens[cursor].ch == '\\'
            && let Some(escaped_len) = escaped_sequence_token_len(tokens, cursor)
        {
            cursor += 1 + escaped_len;
            continue;
        }

        if matches_sequence(tokens, cursor, &close_str) {
            // Emphasis spans must enclose at least one character; a close
            // sitting immediately after the open (e.g. `**` or `*` `*`) is an
            // empty span and is treated as literal text instead.
            if requires_body && cursor == body_start {
                cursor += 1;
                continue;
            }
            return true;
        }

        cursor += 1;
    }

    false
}

/// Whether `delimiter` requires a non-empty body. Emphasis and strikethrough
/// markers must enclose at least one character; code spans may be empty and
/// script markers already constrain their bodies elsewhere.
fn emphasis_requires_body(delimiter: Delimiter) -> bool {
    matches!(
        delimiter,
        Delimiter::BoldMarkdown { .. }
            | Delimiter::ItalicMarkdown { .. }
            | Delimiter::StrikethroughMarkdown
            | Delimiter::BoldHtml
            | Delimiter::ItalicHtml
            | Delimiter::Underline
    )
}

fn locate_script_close(tokens: &[CharToken], mut cursor: usize, marker: char) -> Option<usize> {
    let body_start = cursor;
    while cursor < tokens.len() {
        if tokens[cursor].ch == '\\'
            && let Some(escaped_len) = escaped_sequence_token_len(tokens, cursor)
        {
            cursor += 1 + escaped_len;
            continue;
        }

        let is_close = if marker == '~' {
            is_single_tilde_delimiter(tokens, cursor)
        } else {
            tokens[cursor].ch == marker
        };
        if is_close {
            return valid_script_body(tokens, body_start, cursor).then_some(cursor);
        }

        cursor += 1;
    }

    None
}

fn valid_script_body(tokens: &[CharToken], start: usize, end: usize) -> bool {
    start < end
        && tokens[start..end]
            .iter()
            .all(|token| token.ch.is_ascii_alphanumeric())
}

fn is_single_tilde_delimiter(tokens: &[CharToken], index: usize) -> bool {
    tokens.get(index).is_some_and(|token| token.ch == '~')
        && index
            .checked_sub(1)
            .and_then(|prev| tokens.get(prev))
            .is_none_or(|token| token.ch != '~')
        && tokens.get(index + 1).is_none_or(|token| token.ch != '~')
}

fn matches_sequence(tokens: &[CharToken], index: usize, sequence: &str) -> bool {
    sequence
        .chars()
        .enumerate()
        .all(|(offset, ch)| tokens.get(index + offset).is_some_and(|t| t.ch == ch))
}

fn escaped_sequence_token_len(tokens: &[CharToken], index: usize) -> Option<usize> {
    let next_index = index + 1;
    if next_index >= tokens.len() {
        return None;
    }

    if matches_sequence(tokens, next_index, "</strong>") {
        Some(9)
    } else if matches_sequence(tokens, next_index, "<strong>") {
        Some(8)
    } else if matches_sequence(tokens, next_index, "</em>") {
        Some(5)
    } else if matches_sequence(tokens, next_index, "<em>") {
        Some(4)
    } else if matches_sequence(tokens, next_index, "</u>") {
        Some(4)
    } else if matches_sequence(tokens, next_index, "<u>") {
        Some(3)
    } else if matches_sequence(tokens, next_index, "\\")
        || matches_sequence(tokens, next_index, "*")
        || matches_sequence(tokens, next_index, "_")
        || matches_sequence(tokens, next_index, "~")
        || matches_sequence(tokens, next_index, "[")
        || matches_sequence(tokens, next_index, "]")
        || matches_sequence(tokens, next_index, "`")
        || matches_sequence(tokens, next_index, "^")
    {
        Some(1)
    } else {
        None
    }
}

fn escape_literal_text_with_offset_map(text: &str) -> InlineMarkdownOffsetMap {
    let mut escaped = String::new();
    let mut visible_to_markdown = vec![0; text.len() + 1];
    let mut markdown_to_visible = vec![0];
    let mut index = 0;

    while index < text.len() {
        visible_to_markdown[index] = escaped.len();
        if text[index..].starts_with("</strong>") {
            let start = escaped.len();
            escaped.push('\\');
            escaped.push_str("</strong>");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 9;
            continue;
        }

        if text[index..].starts_with("<strong>") {
            let start = escaped.len();
            escaped.push('\\');
            escaped.push_str("<strong>");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 8;
            continue;
        }

        if text[index..].starts_with("</em>") {
            let start = escaped.len();
            escaped.push('\\');
            escaped.push_str("</em>");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 5;
            continue;
        }

        if text[index..].starts_with("<em>") {
            let start = escaped.len();
            escaped.push('\\');
            escaped.push_str("<em>");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 4;
            continue;
        }

        if text[index..].starts_with("</u>") {
            let start = escaped.len();
            escaped.push('\\');
            escaped.push_str("</u>");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 4;
            continue;
        }

        if text[index..].starts_with("<u>") {
            let start = escaped.len();
            escaped.push('\\');
            escaped.push_str("<u>");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 3;
            continue;
        }

        if text[index..].starts_with('\\') {
            let start = escaped.len();
            escaped.push_str("\\\\");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 1;
            continue;
        }

        if text[index..].starts_with('*') {
            let start = escaped.len();
            escaped.push_str("\\*");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 1;
            continue;
        }

        if text[index..].starts_with('_') {
            let start = escaped.len();
            escaped.push_str("\\_");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 1;
            continue;
        }

        if text[index..].starts_with('~') {
            let start = escaped.len();
            escaped.push_str("\\~");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 1;
            continue;
        }

        if text[index..].starts_with('^') {
            let start = escaped.len();
            escaped.push_str("\\^");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 1;
            continue;
        }

        if text[index..].starts_with('`') {
            let start = escaped.len();
            escaped.push_str("\\`");
            markdown_to_visible.resize(escaped.len() + 1, index);
            for local in 0..=escaped.len() - start {
                markdown_to_visible[start + local] = index;
            }
            index += 1;
            continue;
        }

        let ch = text[index..].chars().next().unwrap();
        let start = escaped.len();
        escaped.push(ch);
        markdown_to_visible.resize(escaped.len() + 1, index);
        for local in 0..=escaped.len() - start {
            markdown_to_visible[start + local] = index;
        }
        index += ch.len_utf8();
    }
    visible_to_markdown[text.len()] = escaped.len();
    markdown_to_visible[escaped.len()] = text.len();

    InlineMarkdownOffsetMap {
        markdown: escaped,
        visible_to_markdown,
        markdown_to_visible,
    }
}

fn escape_code_span_text_with_offset_map(text: &str) -> InlineMarkdownOffsetMap {
    let needs_padding = !text.is_empty()
        && !text.chars().all(|ch| ch == ' ')
        && (text.starts_with([' ', '`']) || text.ends_with([' ', '`']));
    let leading_padding = usize::from(needs_padding);

    let mut markdown = String::new();
    if needs_padding {
        markdown.push(' ');
    }
    markdown.push_str(text);
    if needs_padding {
        markdown.push(' ');
    }

    let mut visible_to_markdown = vec![0; text.len() + 1];
    for (visible, markdown_offset) in visible_to_markdown.iter_mut().enumerate() {
        *markdown_offset = leading_padding + visible;
    }

    let content_start = leading_padding;
    let content_end = leading_padding + text.len();
    let mut markdown_to_visible = vec![0; markdown.len() + 1];
    for (markdown_offset, visible) in markdown_to_visible.iter_mut().enumerate() {
        *visible = if markdown_offset <= content_start {
            0
        } else if markdown_offset >= content_end {
            text.len()
        } else {
            markdown_offset - content_start
        };
    }

    InlineMarkdownOffsetMap {
        markdown,
        visible_to_markdown,
        markdown_to_visible,
    }
}

/// Viterbi-like DP that picks the optimal delimiter stack for each fragment.
///
/// Each fragment's style can be expressed with either Markdown or HTML
/// delimiters.  We minimize the total number of delimiter characters written
/// plus a penalty for HTML variants.  A large penalty is added when a
/// transition would produce 4+ consecutive `*` characters (Markdown ambiguity).
fn choose_fragment_stacks(fragments: &[InlineFragment]) -> Vec<Vec<Delimiter>> {
    // Enumerate the 1-2 possible delimiter stacks for each fragment's style.
    let variants = fragments
        .iter()
        .enumerate()
        .map(|(index, fragment)| {
            stack_variants(
                fragment,
                index.checked_sub(1).and_then(|i| fragments.get(i)),
            )
        })
        .collect::<Vec<_>>();

    // DP table: costs[fragment_index][choice_index]
    let mut costs: Vec<Vec<usize>> = variants
        .iter()
        .map(|choices| vec![usize::MAX; choices.len()])
        .collect();
    let mut previous_choice: Vec<Vec<Option<usize>>> = variants
        .iter()
        .map(|choices| vec![None; choices.len()])
        .collect();

    // Initial fragment: cost from empty stack to each variant.
    for (choice_index, stack) in variants[0].iter().enumerate() {
        costs[0][choice_index] = stack_transition_cost(&[], stack) + stack_variant_penalty(stack);
    }

    // Forward pass: compute minimum cost for each fragment's choices.
    for fragment_index in 1..variants.len() {
        for (choice_index, stack) in variants[fragment_index].iter().enumerate() {
            for (prev_index, prev_stack) in variants[fragment_index - 1].iter().enumerate() {
                let prev_cost = costs[fragment_index - 1][prev_index];
                if prev_cost == usize::MAX {
                    continue;
                }

                let cost = prev_cost
                    + stack_transition_cost(prev_stack, stack)
                    + stack_variant_penalty(stack);
                if cost < costs[fragment_index][choice_index] {
                    costs[fragment_index][choice_index] = cost;
                    previous_choice[fragment_index][choice_index] = Some(prev_index);
                }
            }
        }
    }

    // Backtrack: choose the best final stack and trace back through the DP.
    let last_fragment_index = variants.len() - 1;
    let (mut best_choice, _) = variants[last_fragment_index]
        .iter()
        .enumerate()
        .map(|(choice_index, stack)| {
            (
                choice_index,
                costs[last_fragment_index][choice_index] + stack_transition_cost(stack, &[]),
            )
        })
        .min_by(|(left_index, left_cost), (right_index, right_cost)| {
            left_cost.cmp(right_cost).then_with(|| {
                stack_preference_key(&variants[last_fragment_index][*left_index]).cmp(
                    &stack_preference_key(&variants[last_fragment_index][*right_index]),
                )
            })
        })
        .unwrap_or((0, 0));

    let mut chosen = vec![Vec::new(); variants.len()];
    for fragment_index in (0..variants.len()).rev() {
        chosen[fragment_index] = variants[fragment_index][best_choice].clone();
        if let Some(prev_index) = previous_choice[fragment_index][best_choice] {
            best_choice = prev_index;
        }
    }

    chosen
}

fn stack_variants(
    fragment: &InlineFragment,
    previous_fragment: Option<&InlineFragment>,
) -> Vec<Vec<Delimiter>> {
    let style = fragment.style;
    let code_run_len = style.code.then(|| code_delimiter_run_len(&fragment.text));
    let mut markdown_stack = Vec::new();
    if style.bold {
        markdown_stack.push(Delimiter::BoldMarkdown { marker: '*' });
    }
    if style.underline {
        markdown_stack.push(Delimiter::Underline);
    }
    if style.strikethrough {
        markdown_stack.push(Delimiter::StrikethroughMarkdown);
    }
    match style.script {
        InlineScript::Normal => {}
        InlineScript::Superscript
            if can_use_markdown_script_delimiters(previous_fragment, fragment) =>
        {
            markdown_stack.push(Delimiter::SuperscriptMarkdown)
        }
        InlineScript::Superscript => markdown_stack.push(Delimiter::SuperscriptHtml),
        InlineScript::Subscript
            if style.strikethrough
                || !can_use_markdown_script_delimiters(previous_fragment, fragment) =>
        {
            markdown_stack.push(Delimiter::SubscriptHtml)
        }
        InlineScript::Subscript => markdown_stack.push(Delimiter::SubscriptMarkdown),
    }
    if style.italic {
        markdown_stack.push(Delimiter::ItalicMarkdown { marker: '*' });
    }
    // Code is always the innermost delimiter so it nests inside emphasis.
    if let Some(run_len) = code_run_len {
        markdown_stack.push(Delimiter::CodeMarkdown { run_len });
    }

    let has_emphasis = style.bold || style.italic;
    if !has_emphasis {
        return vec![markdown_stack];
    }

    let mut html_stack = Vec::new();
    if style.bold {
        html_stack.push(Delimiter::BoldHtml);
    }
    if style.underline {
        html_stack.push(Delimiter::Underline);
    }
    if style.strikethrough {
        html_stack.push(Delimiter::StrikethroughMarkdown);
    }
    match style.script {
        InlineScript::Normal => {}
        InlineScript::Superscript => html_stack.push(Delimiter::SuperscriptHtml),
        InlineScript::Subscript => html_stack.push(Delimiter::SubscriptHtml),
    }
    if style.italic {
        html_stack.push(Delimiter::ItalicHtml);
    }
    if let Some(run_len) = code_run_len {
        html_stack.push(Delimiter::CodeMarkdown { run_len });
    }

    vec![markdown_stack, html_stack]
}

pub(crate) fn can_use_markdown_script_delimiters(
    previous_fragment: Option<&InlineFragment>,
    fragment: &InlineFragment,
) -> bool {
    // This guard is shared by serialization and inline projection. Markdown
    // script markers need a plain ASCII owner immediately before the script
    // fragment; otherwise we fall back to <sup>/<sub> so the next parse sees
    // the same style boundary.
    let Some(previous) = previous_fragment else {
        return false;
    };
    if previous.style.has_script() {
        return false;
    }
    previous
        .text
        .chars()
        .next_back()
        .is_some_and(|ch| ch.is_ascii_alphanumeric())
        && previous.html_style == fragment.html_style
        && previous.link == fragment.link
        && previous.footnote.is_none()
        && fragment.footnote.is_none()
        && previous.math.is_none()
        && fragment.math.is_none()
        && styles_match_ignoring_script(previous.style, fragment.style)
}

fn styles_match_ignoring_script(left: InlineStyle, right: InlineStyle) -> bool {
    left.bold == right.bold
        && left.italic == right.italic
        && left.underline == right.underline
        && left.strikethrough == right.strikethrough
        && left.code == right.code
}

fn code_delimiter_run_len(text: &str) -> usize {
    let mut longest = 0usize;
    let mut current = 0usize;
    for ch in text.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest + 1
}

fn stack_transition_len(from: &[Delimiter], to: &[Delimiter]) -> usize {
    let common = common_prefix_len(from, to);
    let close_len = from[common..]
        .iter()
        .rev()
        .map(|delimiter| delimiter.close().len())
        .sum::<usize>();
    let open_len = to[common..]
        .iter()
        .map(|delimiter| delimiter.open().len())
        .sum::<usize>();
    close_len + open_len
}

/// Cost of closing `from` delimiters and opening `to` delimiters in sequence.
/// Adds a heavy penalty if the resulting string would contain 4+ consecutive
/// `*` characters, which Markdown parsers may interpret ambiguously.
fn stack_transition_cost(from: &[Delimiter], to: &[Delimiter]) -> usize {
    let marker_len = stack_transition_len(from, to);
    let marker_string = stack_transition_string(from, to);
    let ambiguity_penalty =
        if !from.is_empty() && !to.is_empty() && longest_star_run(&marker_string) >= 4 {
            1_000
        } else {
            0
        };
    marker_len + ambiguity_penalty
}

fn stack_variant_penalty(stack: &[Delimiter]) -> usize {
    if stack.iter().any(|delimiter| delimiter.is_html()) {
        64
    } else {
        0
    }
}

fn write_stack_transition(output: &mut String, from: &[Delimiter], to: &[Delimiter]) {
    let common = common_prefix_len(from, to);
    for delimiter in from[common..].iter().rev() {
        output.push_str(&delimiter.close());
    }
    for delimiter in &to[common..] {
        output.push_str(&delimiter.open());
    }
}

fn stack_transition_string(from: &[Delimiter], to: &[Delimiter]) -> String {
    let mut output = String::new();
    write_stack_transition(&mut output, from, to);
    output
}

fn common_prefix_len(left: &[Delimiter], right: &[Delimiter]) -> usize {
    let mut index = 0;
    while index < left.len() && index < right.len() && left[index] == right[index] {
        index += 1;
    }
    index
}

fn stack_preference_key(stack: &[Delimiter]) -> Vec<u8> {
    stack
        .iter()
        .map(|delimiter| delimiter.preference_rank())
        .collect()
}

fn longest_star_run(text: &str) -> usize {
    let mut max_run = 0;
    let mut current_run = 0;
    for ch in text.chars() {
        if ch == '*' {
            current_run += 1;
            max_run = max_run.max(current_run);
        } else {
            current_run = 0;
        }
    }
    max_run
}

fn style_flag_enabled(style: InlineStyle, flag: StyleFlag) -> bool {
    match flag {
        StyleFlag::Bold => style.bold,
        StyleFlag::Italic => style.italic,
        StyleFlag::Underline => style.underline,
        StyleFlag::Strikethrough => style.strikethrough,
        StyleFlag::Code => style.code,
        StyleFlag::Superscript => style.script == InlineScript::Superscript,
        StyleFlag::Subscript => style.script == InlineScript::Subscript,
    }
}

fn set_style_flag(mut style: InlineStyle, flag: StyleFlag, enabled: bool) -> InlineStyle {
    match flag {
        StyleFlag::Bold => style.bold = enabled,
        StyleFlag::Italic => style.italic = enabled,
        StyleFlag::Underline => style.underline = enabled,
        StyleFlag::Strikethrough => style.strikethrough = enabled,
        StyleFlag::Code => style.code = enabled,
        StyleFlag::Superscript => {
            style.script = if enabled {
                InlineScript::Superscript
            } else if style.script == InlineScript::Superscript {
                InlineScript::Normal
            } else {
                style.script
            }
        }
        StyleFlag::Subscript => {
            style.script = if enabled {
                InlineScript::Subscript
            } else if style.script == InlineScript::Subscript {
                InlineScript::Normal
            } else {
                style.script
            }
        }
    }
    style
}

fn clamp_to_char_boundary(text: &str, offset: usize) -> usize {
    let clamped = offset.min(text.len());
    if text.is_char_boundary(clamped) {
        return clamped;
    }

    let mut boundary = clamped;
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

fn can_open_emphasis(tokens: &[CharToken], index: usize, len: usize) -> bool {
    tokens
        .get(index + len)
        .map(|token| !token.ch.is_whitespace())
        .unwrap_or(false)
}

fn can_open_script(tokens: &[CharToken], index: usize, marker: char) -> bool {
    if token_is_backslash_escaped(tokens, index) {
        return false;
    }

    if marker == '~' && !is_single_tilde_delimiter(tokens, index) {
        return false;
    }

    index > 0
        && tokens[index - 1].ch.is_ascii_alphanumeric()
        && tokens
            .get(index + 1)
            .is_some_and(|token| token.ch.is_ascii_alphanumeric())
}

fn can_close_emphasis(tokens: &[CharToken], index: usize) -> bool {
    index > 0 && !tokens[index - 1].ch.is_whitespace()
}

#[cfg(test)]
mod tests {
    use super::{
        InlineFragment, InlineInsertionAttributes, InlineLinkHit, InlineMathDelimiter,
        InlineScript, InlineStyle, InlineTextTree, LinkReferenceDefinitions, StyleFlag,
    };
    use crate::components::HtmlCssColor;

    #[test]
    fn parses_supported_styles_and_serializes_canonically() {
        let tree = InlineTextTree::from_markdown("1**23**4*56*7<u>89</u>0***ab***<u>*cd*</u>");
        let serialized = tree.serialize_markdown();
        let reparsed = InlineTextTree::from_markdown(&serialized);

        assert_eq!(tree.visible_text(), "1234567890abcd");
        assert_eq!(reparsed.visible_text(), tree.visible_text());
        assert_eq!(reparsed.render_cache().spans(), tree.render_cache().spans());
    }

    #[test]
    fn parses_underscore_emphasis_and_canonicalizes_to_asterisks() {
        let tree = InlineTextTree::from_markdown("_a_ __b__");

        assert_eq!(tree.visible_text(), "a b");
        assert_eq!(tree.serialize_markdown(), "*a* **b**");
    }

    #[test]
    fn emphasis_delimiters_surrounded_by_spaces_stay_literal() {
        let tree = InlineTextTree::from_markdown("* a * _ b _");

        assert_eq!(tree.visible_text(), "* a * _ b _");
        assert_eq!(tree.serialize_markdown(), "\\* a \\* \\_ b \\_");
    }

    #[test]
    fn preserves_unclosed_markers_as_literal_text() {
        let tree = InlineTextTree::from_markdown("1**234");

        assert_eq!(tree.visible_text(), "1**234");
        assert_eq!(tree.serialize_markdown(), "1\\*\\*234");
    }

    #[test]
    fn empty_emphasis_spans_stay_literal() {
        // `**`, `* *`, or `**word` must not be swallowed as an empty emphasis
        // span; the markers stay literal until a non-empty body is closed.
        for input in ["*", "**", "***", "****", "~~~~", "__"] {
            let tree = InlineTextTree::from_markdown(input);
            assert_eq!(tree.visible_text(), input, "input {input:?} lost markers");
        }

        let leading = InlineTextTree::from_markdown("**word");
        assert_eq!(leading.visible_text(), "**word");
        assert_eq!(leading.serialize_markdown(), "\\*\\*word");

        let trailing = InlineTextTree::from_markdown("**word*");
        assert_eq!(trailing.visible_text(), "**word*");
    }

    #[test]
    fn non_empty_emphasis_still_parses_after_empty_guard() {
        let bold = InlineTextTree::from_markdown("**word**");
        assert_eq!(bold.visible_text(), "word");
        assert_eq!(bold.serialize_markdown(), "**word**");

        let italic = InlineTextTree::from_markdown("*a*");
        assert_eq!(italic.visible_text(), "a");
        assert_eq!(italic.serialize_markdown(), "*a*");

        let single_char_bold = InlineTextTree::from_markdown("**a**");
        assert_eq!(single_char_bold.visible_text(), "a");
        assert_eq!(single_char_bold.serialize_markdown(), "**a**");

        let bold_italic = InlineTextTree::from_markdown("***x***");
        assert_eq!(bold_italic.visible_text(), "x");
        let spans = bold_italic.render_cache();
        assert!(
            spans
                .spans()
                .iter()
                .all(|span| span.style.bold && span.style.italic)
        );
    }

    #[test]
    fn unclosed_multichar_opener_stays_fully_literal() {
        // While typing `**bold**`, the intermediate `**bold*` must stay literal;
        // otherwise the second `*` opens an italic span and the bold is lost.
        let partial = InlineTextTree::from_markdown("**bold*");
        assert_eq!(partial.visible_text(), "**bold*");
        assert!(
            partial
                .render_cache()
                .spans()
                .iter()
                .all(|span| !span.style.italic && !span.style.bold),
            "`**bold*` must be plain literal, not italic"
        );

        // The completed marker still resolves to bold (not italic).
        let complete = InlineTextTree::from_markdown("**bold**");
        assert_eq!(complete.visible_text(), "bold");
        assert!(
            complete
                .render_cache()
                .spans()
                .iter()
                .all(|span| span.style.bold && !span.style.italic),
            "`**bold**` must be bold, not italic"
        );

        // A genuine single-`*` italic opener is unaffected by the multi-char rule.
        let italic = InlineTextTree::from_markdown("*word*");
        assert_eq!(italic.visible_text(), "word");
        assert!(
            italic
                .render_cache()
                .spans()
                .iter()
                .all(|span| span.style.italic && !span.style.bold),
            "`*word*` must stay italic"
        );

        // Other unclosed multi-char openers stay literal as a unit too.
        for input in ["__bold_", "~~strike~"] {
            let tree = InlineTextTree::from_markdown(input);
            assert_eq!(tree.visible_text(), input, "input {input:?} lost markers");
            assert!(
                tree.render_cache()
                    .spans()
                    .iter()
                    .all(|span| !span.style.italic
                        && !span.style.bold
                        && !span.style.strikethrough),
                "input {input:?} should be plain literal"
            );
        }
    }

    #[test]
    fn empty_code_span_is_unaffected_by_emphasis_guard() {
        // The empty-emphasis guard must not touch code spans. `*` inside a code
        // span stays literal and the span round-trips.
        let tree = InlineTextTree::from_markdown("`*`");
        assert_eq!(tree.visible_text(), "*");
        assert_eq!(tree.serialize_markdown(), "`*`");
    }

    #[test]
    fn preserves_escaped_marker_sequences_as_literal_text() {
        let tree = InlineTextTree::from_markdown("\\*\\*\\<u>text\\</u>\\\\");

        assert_eq!(tree.visible_text(), "**<u>text</u>\\");
        assert_eq!(tree.serialize_markdown(), "\\*\\*\\<u>text\\</u>\\\\");
    }

    #[test]
    fn preserves_tibetan_spaces_through_inline_round_trip() {
        let markdown = "༄༅།།དཔལ་ལྡན་རྩ་བའི་བླ་མ་རིན་པོ་ཆེ།། བདག་གི་སྤྱི་བོར་པདྨའི་གདན་བཞུགས་ནས།། ";
        let tree = InlineTextTree::from_markdown(markdown);
        let serialized = tree.serialize_markdown();

        assert_eq!(tree.visible_text(), markdown);
        assert!(tree.visible_text().contains("།། བདག"));
        assert!(tree.visible_text().ends_with(' '));
        assert_eq!(serialized, markdown);
        assert_eq!(
            InlineTextTree::from_markdown(&serialized).visible_text(),
            markdown
        );
    }

    #[test]
    fn preserves_chinese_spaces_through_inline_round_trip() {
        let markdown = "中文 文本 ";
        let tree = InlineTextTree::from_markdown(markdown);

        assert_eq!(tree.visible_text(), markdown);
        assert_eq!(tree.serialize_markdown(), markdown);
    }

    #[test]
    fn toggle_style_operates_on_selected_slice_only() {
        let mut tree = InlineTextTree::plain("123");
        assert!(tree.toggle_bold(1..3));
        assert_eq!(tree.serialize_markdown(), "1**23**");

        assert!(tree.toggle_bold(2..3));
        assert_eq!(tree.serialize_markdown(), "1**2**3");
    }

    #[test]
    fn replaces_visible_range_and_normalizes_manual_markdown_input() {
        let tree = InlineTextTree::plain(String::new());
        let result =
            tree.replace_visible_range(0..0, "**bold**", InlineInsertionAttributes::default());

        assert_eq!(result.tree.visible_text(), "bold");
        assert_eq!(result.map_offset(8), 4);
        assert_eq!(result.tree.serialize_markdown(), "**bold**");
    }

    #[test]
    fn renders_nested_marks_without_storing_markers_in_text() {
        let tree = InlineTextTree::from_markdown("**<u>*TEST*</u>**");
        let cache = tree.render_cache();

        assert_eq!(cache.visible_text(), "TEST");
        assert_eq!(
            cache.style_at(0),
            InlineStyle {
                bold: true,
                italic: true,
                underline: true,
                strikethrough: false,
                code: false,
                script: InlineScript::Normal,
            }
        );
    }

    #[test]
    fn replace_visible_range_raw_preserves_markers_as_literal_text() {
        let tree = InlineTextTree::plain("alpha");
        let result = tree.replace_visible_range_raw(
            5..5,
            "**`<u>x</u>`**",
            InlineInsertionAttributes::default(),
        );

        assert_eq!(result.tree.visible_text(), "alpha**`<u>x</u>`**");
        assert_eq!(
            result.tree.serialize_markdown(),
            "alpha\\*\\*\\`\\<u>x\\</u>\\`\\*\\*"
        );
    }

    #[test]
    fn unwrap_code_fragments_keeps_text_and_removes_code_style() {
        let mut tree = InlineTextTree::from_markdown("before `code` after");
        tree.unwrap_styles_on_fragments(&[(1, StyleFlag::Code)]);

        assert_eq!(tree.visible_text(), "before code after");
        let cache = tree.render_cache();
        assert!(!cache.style_at(7).code);
        assert_eq!(tree.serialize_markdown(), "before code after");
    }

    #[test]
    fn parses_and_serializes_strikethrough() {
        let tree = InlineTextTree::from_markdown("~~text~~");
        let cache = tree.render_cache();

        assert_eq!(tree.visible_text(), "text");
        assert!(cache.style_at(0).strikethrough);
        assert_eq!(tree.serialize_markdown(), "~~text~~");
    }

    #[test]
    fn parses_and_serializes_superscript() {
        let tree = InlineTextTree::from_markdown("x^2^");
        let cache = tree.render_cache();

        assert_eq!(tree.visible_text(), "x2");
        assert_eq!(cache.style_at(1).script, InlineScript::Superscript);
        assert_eq!(tree.serialize_markdown(), "x^2^");
    }

    #[test]
    fn parses_and_serializes_subscript_without_conflicting_with_strikethrough() {
        let tree = InlineTextTree::from_markdown("H~2~O and ~~old~~");
        let cache = tree.render_cache();

        assert_eq!(tree.visible_text(), "H2O and old");
        assert_eq!(cache.style_at(1).script, InlineScript::Subscript);
        assert!(cache.style_at("H2O and ".len()).strikethrough);
        assert_eq!(tree.serialize_markdown(), "H~2~O and ~~old~~");
    }

    #[test]
    fn script_markers_require_ascii_context_and_ascii_body() {
        for markdown in ["\\^2^", "\\~2~", "汉^2^", "H~二~O", "`x^2^ H~2~O`"] {
            let tree = InlineTextTree::from_markdown(markdown);
            assert!(
                tree.render_cache()
                    .spans()
                    .iter()
                    .all(|span| span.style.script == InlineScript::Normal),
                "{markdown} should not produce script spans"
            );
        }
    }

    #[test]
    fn inline_html_sup_and_sub_map_to_script_style() {
        let tree = InlineTextTree::from_markdown("x<sup>2</sup> and H<sub>2</sub>O");
        let cache = tree.render_cache();

        assert_eq!(tree.visible_text(), "x2 and H2O");
        assert_eq!(cache.style_at(1).script, InlineScript::Superscript);
        assert_eq!(
            cache.style_at("x2 and H".len()).script,
            InlineScript::Subscript
        );
        assert_eq!(tree.serialize_markdown(), "x^2^ and H~2~O");

        let standalone = InlineTextTree::from_markdown("<sup>2</sup>");
        assert_eq!(standalone.serialize_markdown(), "<sup>2</sup>");
    }

    #[test]
    fn unmatched_strikethrough_markers_stay_literal() {
        let tree = InlineTextTree::from_markdown("~~text");
        assert_eq!(tree.visible_text(), "~~text");
        assert_eq!(tree.serialize_markdown(), "\\~\\~text");
    }

    #[test]
    fn toggle_strikethrough_operates_on_selected_slice_only() {
        let mut tree = InlineTextTree::plain("1234");
        assert!(tree.toggle_strikethrough(1..4));
        assert!(tree.toggle_strikethrough(2..4));

        let serialized = tree.serialize_markdown();
        let reparsed = InlineTextTree::from_markdown(&serialized);

        assert_eq!(serialized, "1~~2~~34");
        assert_eq!(tree, reparsed);
    }

    #[test]
    fn insertion_at_outer_end_of_terminal_strikethrough_is_plain_text() {
        let tree = InlineTextTree::from_markdown("~~123~~");
        let result = tree.replace_visible_range(
            tree.visible_len()..tree.visible_len(),
            "456",
            tree.attributes_for_insertion_at(tree.visible_len()),
        );
        assert_eq!(result.tree.serialize_markdown(), "~~123~~456");
    }

    #[test]
    fn insertion_at_outer_start_of_terminal_strikethrough_is_plain_text() {
        let tree = InlineTextTree::from_markdown("~~123~~");
        let result = tree.replace_visible_range(0..0, "0", tree.attributes_for_insertion_at(0));
        assert_eq!(result.tree.serialize_markdown(), "0~~123~~");
    }

    #[test]
    fn serializes_partial_underline_removal_without_ambiguous_star_runs() {
        let mut tree = InlineTextTree::plain("1234");
        assert!(tree.toggle_bold(1..4));
        assert!(tree.toggle_underline(1..4));
        assert!(tree.toggle_italic(1..4));
        assert!(tree.toggle_underline(2..4));

        let serialized = tree.serialize_markdown();
        let reparsed = InlineTextTree::from_markdown(&serialized);

        assert_eq!(serialized, "1**<u>*2*</u>*34***");
        assert!(!serialized.contains("*****34"));
        assert_eq!(reparsed.visible_text(), "1234");
        assert_eq!(reparsed.render_cache().spans(), tree.render_cache().spans());
    }

    #[test]
    fn parses_inline_links_autolinks_and_preserves_other_unsupported_inline_syntax() {
        let markdown =
            "[link](http://example.com) ![alt](/img.png) <http://example.com/> <span>x</span>";
        let tree = InlineTextTree::from_markdown(markdown);

        assert_eq!(
            tree.visible_text(),
            "link ![alt](/img.png) http://example.com/ <span>x</span>"
        );
        assert_eq!(tree.render_cache().link_at(0), Some("http://example.com"));
        assert_eq!(
            tree.render_cache().link_at("link ![alt](/img.png) ".len()),
            Some("http://example.com/")
        );
        assert_eq!(tree.serialize_markdown(), markdown);
    }

    #[test]
    fn parses_dollar_inline_math_as_source_preserving_fragment() {
        let markdown = "before $x^2$ after";
        let tree = InlineTextTree::from_markdown(markdown);
        let cache = tree.render_cache();
        let math_start = "before ".len();
        let math = cache
            .inline_math_at(math_start)
            .expect("inline math span should be recorded");

        assert_eq!(tree.visible_text(), markdown);
        assert_eq!(math.source, "$x^2$");
        assert_eq!(math.body, "x^2");
        assert_eq!(math.delimiter, InlineMathDelimiter::Dollar);
        assert_eq!(tree.serialize_markdown(), markdown);
    }

    #[test]
    fn parses_paren_inline_math_as_source_preserving_fragment() {
        let markdown = "before \\(\\frac{1}{2}\\) after";
        let tree = InlineTextTree::from_markdown(markdown);
        let cache = tree.render_cache();
        let math_start = "before ".len();
        let math = cache
            .inline_math_at(math_start)
            .expect("inline math span should be recorded");

        assert_eq!(tree.visible_text(), markdown);
        assert_eq!(math.source, "\\(\\frac{1}{2}\\)");
        assert_eq!(math.body, "\\frac{1}{2}");
        assert_eq!(math.delimiter, InlineMathDelimiter::Paren);
        assert_eq!(tree.serialize_markdown(), markdown);
    }

    #[test]
    fn rejects_conservative_inline_math_cases() {
        for markdown in ["\\$x$", "$ x $", "$", "$x\ny$", "cost $12$"] {
            let tree = InlineTextTree::from_markdown(markdown);
            assert!(
                tree.render_cache()
                    .spans()
                    .iter()
                    .all(|span| span.math.is_none()),
                "{markdown:?} should stay plain text"
            );
        }
    }

    #[test]
    fn inline_math_does_not_parse_inside_code_spans() {
        let tree = InlineTextTree::from_markdown("`$x$` and $y$");
        let cache = tree.render_cache();

        assert!(cache.style_at(0).code);
        assert!(cache.inline_math_at(0).is_none());
        assert!(cache.inline_math_at("$x$ and ".len()).is_some());
        assert_eq!(tree.serialize_markdown(), "`$x$` and $y$");
    }

    #[test]
    fn parses_inline_link_title_without_polluting_open_target() {
        let markdown = "[ABC](https://abc.com \"https://abc.com\")";
        let tree = InlineTextTree::from_markdown(markdown);

        assert_eq!(tree.visible_text(), "ABC");
        assert_eq!(
            tree.render_cache().link_hit_at(0),
            Some(&InlineLinkHit {
                prompt_target: "https://abc.com".to_string(),
                open_target: "https://abc.com".to_string(),
            })
        );
        assert_eq!(tree.serialize_markdown(), markdown);
    }

    #[test]
    fn parses_span_style_as_inline_html_not_link() {
        let markdown = "留意<span style='color:blue;'>磁盘预留空间、系统环境变量</span>等问题";
        let tree = InlineTextTree::from_markdown(markdown);
        let cache = tree.render_cache();
        let span_start = "留意".len();

        assert_eq!(tree.visible_text(), "留意磁盘预留空间、系统环境变量等问题");
        assert_eq!(cache.link_at(span_start), None);
        assert!(matches!(
            cache.html_style_at(span_start).and_then(|style| style.color),
            Some(HtmlCssColor::Rgba(color))
                if color.red == 0 && color.green == 0 && color.blue == 255
        ));
        assert_eq!(cache.html_style_at(0), None);
        assert_eq!(
            tree.serialize_markdown(),
            "留意<span style=\"color: rgba(0,0,255,1.000);\">磁盘预留空间、系统环境变量</span>等问题"
        );
    }

    #[test]
    fn inline_span_style_allows_nested_markdown_code() {
        let markdown = "<span style='color:blue;'>英伟达驱动`CUDA+cuDNN`</span>";
        let tree = InlineTextTree::from_markdown(markdown);
        let cache = tree.render_cache();
        let code_start = "英伟达驱动".len();

        assert_eq!(tree.visible_text(), "英伟达驱动CUDA+cuDNN");
        assert!(cache.style_at(code_start).code);
        assert!(matches!(
            cache.html_style_at(code_start).and_then(|style| style.color),
            Some(HtmlCssColor::Rgba(color))
                if color.red == 0 && color.green == 0 && color.blue == 255
        ));

        let reparsed = InlineTextTree::from_markdown(&tree.serialize_markdown());
        assert_eq!(reparsed.visible_text(), tree.visible_text());
        assert_eq!(reparsed.render_cache().spans(), tree.render_cache().spans());
    }

    #[test]
    fn html_like_tags_are_not_autolinks_when_unsafe_or_unclosed() {
        let unclosed = InlineTextTree::from_markdown("<span style='color:blue;'>x");
        assert_eq!(unclosed.visible_text(), "<span style='color:blue;'>x");
        assert_eq!(unclosed.render_cache().link_at(0), None);

        let script = InlineTextTree::from_markdown("<script>alert(1)</script>");
        assert_eq!(script.visible_text(), "<script>alert(1)</script>");
        assert_eq!(script.render_cache().link_at(0), None);
    }

    #[test]
    fn parses_reference_style_links_with_definitions_and_preserves_syntax() {
        let markdown = "[reference link][ref-link]";
        let definitions =
            super::super::link::parse_link_reference_definitions("[ref-link]: https://example.com");
        let tree = InlineTextTree::from_markdown_with_link_references(markdown, &definitions);

        assert_eq!(tree.visible_text(), "reference link");
        assert_eq!(tree.render_cache().link_at(0), Some("https://example.com"));
        assert_eq!(tree.serialize_markdown(), markdown);
    }

    #[test]
    fn parses_reference_style_links_with_generic_normalized_labels() {
        let markdown = "[reference link][Ref   Links]";
        let definitions = super::super::link::parse_link_reference_definitions(
            "[ref links]: https://example.com",
        );
        let tree = InlineTextTree::from_markdown_with_link_references(markdown, &definitions);

        assert_eq!(tree.visible_text(), "reference link");
        assert_eq!(tree.render_cache().link_at(0), Some("https://example.com"));
        assert_eq!(
            tree.render_cache().link_hit_at(0),
            Some(&InlineLinkHit {
                prompt_target: "Ref   Links".to_string(),
                open_target: "https://example.com".to_string(),
            })
        );
        assert_eq!(tree.serialize_markdown(), markdown);
    }

    #[test]
    fn parses_collapsed_reference_style_links_with_definitions() {
        let markdown = "[collapsed reference][]";
        let definitions = super::super::link::parse_link_reference_definitions(
            "[collapsed reference]: https://example.org",
        );
        let tree = InlineTextTree::from_markdown_with_link_references(markdown, &definitions);

        assert_eq!(tree.visible_text(), "collapsed reference");
        assert_eq!(tree.render_cache().link_at(0), Some("https://example.org"));
        assert_eq!(
            tree.serialize_markdown(),
            "[collapsed reference][collapsed reference]"
        );
    }

    #[test]
    fn parses_shortcut_reference_style_links_with_definitions() {
        let markdown = "[shortcut reference]";
        let definitions = super::super::link::parse_link_reference_definitions(
            "[shortcut reference]: https://example.net",
        );
        let tree = InlineTextTree::from_markdown_with_link_references(markdown, &definitions);

        assert_eq!(tree.visible_text(), "shortcut reference");
        assert_eq!(tree.render_cache().link_at(0), Some("https://example.net"));
        assert_eq!(
            tree.serialize_markdown(),
            "[shortcut reference][shortcut reference]"
        );
    }

    #[test]
    fn resolves_reference_link_examples_from_test_markdown() {
        let markdown = include_str!("../../../test.md");
        let definitions = super::super::link::parse_link_reference_definitions(markdown);
        let tree = InlineTextTree::from_markdown_with_link_references(
            "[reference link][ref-link] [collapsed reference][] [shortcut reference]",
            &definitions,
        );

        assert_eq!(
            tree.visible_text(),
            "reference link collapsed reference shortcut reference"
        );
        assert_eq!(tree.render_cache().link_at(0), Some("https://example.com"));
        assert_eq!(
            tree.render_cache().link_at("reference link ".len()),
            Some("https://example.org")
        );
        assert_eq!(
            tree.render_cache()
                .link_at("reference link collapsed reference ".len()),
            Some("https://example.net")
        );
    }

    #[test]
    fn unresolved_reference_style_links_remain_literal_text() {
        let markdown = "[reference link][missing]";
        let tree = InlineTextTree::from_markdown_with_link_references(
            markdown,
            &LinkReferenceDefinitions::default(),
        );

        assert_eq!(tree.visible_text(), markdown);
        assert_eq!(tree.render_cache().link_at(0), None);
        assert_eq!(tree.serialize_markdown(), markdown);
    }

    #[test]
    fn unresolved_shortcut_reference_links_remain_literal_text() {
        let markdown = "[shortcut reference]";
        let tree = InlineTextTree::from_markdown_with_link_references(
            markdown,
            &LinkReferenceDefinitions::default(),
        );

        assert_eq!(tree.visible_text(), markdown);
        assert_eq!(tree.render_cache().link_at(0), None);
        assert_eq!(tree.serialize_markdown(), markdown);
    }

    #[test]
    fn shortcut_reference_detection_does_not_consume_images_as_links() {
        let definitions = super::super::link::parse_link_reference_definitions(
            "[alt]: https://example.com/not-an-image-link",
        );
        let tree = InlineTextTree::from_markdown_with_link_references("![alt]", &definitions);

        assert_eq!(tree.visible_text(), "![alt]");
        assert_eq!(tree.render_cache().link_at(0), None);
        assert_eq!(tree.serialize_markdown(), "![alt]");
    }

    #[test]
    fn shortcut_reference_detection_does_not_rewrite_reference_images() {
        let definitions = super::super::link::parse_link_reference_definitions(
            "[img]: https://example.com/image.png",
        );
        let tree =
            InlineTextTree::from_markdown_with_link_references("![cover][img]", &definitions);

        assert_eq!(tree.visible_text(), "![cover][img]");
        assert_eq!(tree.render_cache().link_at(0), None);
        assert_eq!(tree.serialize_markdown(), "![cover][img]");
    }

    #[test]
    fn parses_mailto_autolinks_and_preserves_syntax() {
        let markdown = "<mailto:test@example.com>";
        let tree = InlineTextTree::from_markdown(markdown);

        assert_eq!(tree.visible_text(), "mailto:test@example.com");
        assert_eq!(
            tree.render_cache().link_at(0),
            Some("mailto:test@example.com")
        );
        assert_eq!(tree.serialize_markdown(), markdown);
    }

    #[test]
    fn parses_any_standalone_autolink_and_preserves_syntax() {
        let markdown = "<ref2>";
        let tree = InlineTextTree::from_markdown(markdown);

        assert_eq!(tree.visible_text(), "ref2");
        assert_eq!(tree.render_cache().link_at(0), Some("ref2"));
        assert_eq!(
            tree.render_cache().link_hit_at(0),
            Some(&InlineLinkHit {
                prompt_target: "ref2".to_string(),
                open_target: "ref2".to_string(),
            })
        );
        assert_eq!(tree.serialize_markdown(), markdown);
    }

    #[test]
    fn parses_nested_inline_marks_inside_link_label() {
        let tree = InlineTextTree::from_markdown("[**go** now](https://example.com)");
        let cache = tree.render_cache();

        assert_eq!(tree.visible_text(), "go now");
        assert_eq!(cache.link_at(0), Some("https://example.com"));
        assert!(cache.style_at(0).bold);
        assert_eq!(
            tree.serialize_markdown(),
            "[**go** now](https://example.com)"
        );
    }

    #[test]
    fn serializes_partial_bold_removal_without_ambiguous_star_runs() {
        let mut tree = InlineTextTree::plain("1234");
        assert!(tree.toggle_bold(1..4));
        assert!(tree.toggle_italic(1..4));
        assert!(tree.toggle_bold(2..4));

        let serialized = tree.serialize_markdown();
        let reparsed = InlineTextTree::from_markdown(&serialized);

        assert_eq!(serialized, "1***2***<em>34</em>");
        assert_eq!(reparsed.visible_text(), "1234");
        assert_eq!(reparsed.render_cache().spans(), tree.render_cache().spans());
    }

    // --- inline code tests ---

    #[test]
    fn parses_backtick_as_code_style() {
        let tree = InlineTextTree::from_markdown("a `code` b");
        let cache = tree.render_cache();

        assert_eq!(cache.visible_text(), "a code b");
        // "code" at offset 2 should have code style
        let style = cache.style_at(2);
        assert!(style.code, "expected code=true at offset 2");
        assert!(!style.bold);
    }

    #[test]
    fn backtick_content_preserves_markers_as_literal() {
        // Inside a code span, ** and * are literal, not parsed as bold/italic.
        let tree = InlineTextTree::from_markdown("`**not bold**`");
        let cache = tree.render_cache();

        assert_eq!(cache.visible_text(), "**not bold**");
        let style = cache.style_at(0);
        assert!(style.code);
        assert!(!style.bold);
        assert!(!style.italic);
    }

    #[test]
    fn unclosed_backtick_is_literal() {
        let tree = InlineTextTree::from_markdown("a `b");
        assert_eq!(tree.visible_text(), "a `b");
        assert_eq!(tree.serialize_markdown(), "a \\`b");
    }

    #[test]
    fn toggle_code_on_selection() {
        let mut tree = InlineTextTree::plain("hello world");
        assert!(tree.toggle_code(0..5)); // "hello"
        assert_eq!(tree.serialize_markdown(), "`hello` world");
    }

    #[test]
    fn toggle_code_twice_removes_code() {
        let mut tree = InlineTextTree::plain("hello world");
        assert!(tree.toggle_code(0..5));
        assert!(tree.toggle_code(0..5)); // toggle back
        assert_eq!(tree.serialize_markdown(), "hello world");
    }

    #[test]
    fn code_round_trips_through_serialization() {
        let tree = InlineTextTree::from_markdown("a `code` b");
        let serialized = tree.serialize_markdown();
        let reparsed = InlineTextTree::from_markdown(&serialized);

        assert_eq!(serialized, "a `code` b");
        assert_eq!(reparsed.visible_text(), "a code b");
        assert_eq!(reparsed.render_cache().spans(), tree.render_cache().spans());
    }

    #[test]
    fn code_inside_bold_text() {
        // `**bold `code` more**` — bold wraps around a code span.
        let tree = InlineTextTree::from_markdown("**bold `code` more**");
        let serialized = tree.serialize_markdown();
        let reparsed = InlineTextTree::from_markdown(&serialized);

        assert_eq!(tree.visible_text(), "bold code more");
        assert_eq!(reparsed.visible_text(), tree.visible_text());
        assert_eq!(reparsed.render_cache().spans(), tree.render_cache().spans());
    }

    #[test]
    fn consecutive_backticks_treated_as_literal() {
        // Per CommonMark: a backtick run that has no matching closing run
        // is treated as literal text.
        let tree = InlineTextTree::from_markdown("``");
        // Two backticks with no closing -> literal (run_len=2, no matching close).
        assert_eq!(tree.visible_text(), "``");
        assert!(!tree.render_cache().style_at(0).code);
    }

    #[test]
    fn variable_length_backtick_run() {
        // `` `` `x` ``` `` (run_len=1 with 'x', matching close of run_len=1)
        let tree = InlineTextTree::from_markdown("`x`");
        assert_eq!(tree.visible_text(), "x");
        assert!(tree.render_cache().style_at(0).code);

        // ``` `` `` `` `` (run_len=2, content "a", run_len=2 close)
        let tree2 = InlineTextTree::from_markdown("``a``");
        assert_eq!(tree2.visible_text(), "a");
        assert!(tree2.render_cache().style_at(0).code);
    }

    #[test]
    fn code_span_content_normalization() {
        // Leading/trailing single space is stripped.
        let tree = InlineTextTree::from_markdown("` hello `");
        assert_eq!(tree.visible_text(), "hello");
        assert!(tree.render_cache().style_at(0).code);

        // All-space content is preserved (no stripping per spec).
        let tree2 = InlineTextTree::from_markdown("`   `");
        assert_eq!(tree2.visible_text(), "   ");
    }

    #[test]
    fn code_span_newline_is_preserved_as_hard_line() {
        let tree = InlineTextTree::from_markdown("`a\nb`");
        assert_eq!(tree.visible_text(), "a\nb");

        let cache = tree.render_cache();
        assert_eq!(cache.spans().len(), 1);
        assert_eq!(cache.spans()[0].range, 0..3);
        assert!(cache.spans()[0].style.code);
        assert_eq!(tree.serialize_markdown(), "`a\nb`");
    }

    #[test]
    fn code_span_blank_line_stays_inside_single_code_span() {
        let tree = InlineTextTree::from_markdown("`line 1\n\nline 2`");
        assert_eq!(tree.visible_text(), "line 1\n\nline 2");

        let cache = tree.render_cache();
        assert_eq!(cache.spans().len(), 1);
        assert_eq!(cache.spans()[0].range, 0.."line 1\n\nline 2".len());
        assert!(cache.spans()[0].style.code);
        assert_eq!(tree.serialize_markdown(), "`line 1\n\nline 2`");
    }

    #[test]
    fn code_span_content_keeps_inline_markers_literal() {
        let tree = InlineTextTree::from_markdown("`*[x] [link](x) \\\\`");

        assert_eq!(tree.visible_text(), "*[x] [link](x) \\\\");
        let cache = tree.render_cache();
        assert_eq!(cache.spans().len(), 1);
        assert!(cache.spans()[0].style.code);
        assert!(cache.spans()[0].link.is_none());
        assert!(!cache.spans()[0].style.bold);
        assert!(!cache.spans()[0].style.italic);
    }

    #[test]
    fn parses_literal_backtick_runs_with_unambiguous_delimiters() {
        let markdown = "`` ` `` and ``` `` ``` and ```` ``` ````";
        let tree = InlineTextTree::from_markdown(markdown);
        let cache = tree.render_cache();
        let code_ranges = cache
            .spans()
            .iter()
            .filter(|span| span.style.code)
            .map(|span| span.range.clone())
            .collect::<Vec<_>>();

        assert_eq!(tree.visible_text(), "` and `` and ```");
        assert_eq!(code_ranges, vec![0..1, 6..8, 13..16]);
        assert!(!cache.style_at("` ".len()).code);
        assert!(!cache.style_at("` and `` ".len()).code);

        let serialized = tree.serialize_markdown();
        let reparsed = InlineTextTree::from_markdown(&serialized);
        assert_eq!(reparsed.visible_text(), tree.visible_text());
        assert_eq!(reparsed.render_cache().spans(), cache.spans());
    }

    #[test]
    fn serializes_code_spans_with_safe_backtick_delimiters_and_padding() {
        for text in [" leading", "trailing ", "`tick", "tick`", "`", "``", "   "] {
            let tree = InlineTextTree::from_fragments(vec![InlineFragment {
                text: text.to_string(),
                style: InlineStyle {
                    code: true,
                    ..InlineStyle::default()
                },
                html_style: None,
                link: None,
                footnote: None,
                math: None,
            }]);
            let serialized = tree.serialize_markdown();
            let reparsed = InlineTextTree::from_markdown(&serialized);

            assert_eq!(
                reparsed.visible_text(),
                text,
                "serialized as {serialized:?}"
            );
            assert_eq!(reparsed.render_cache().spans(), tree.render_cache().spans());
        }
    }

    #[test]
    fn source_to_rendered_round_trip_preserves_code_span() {
        // Simulate Source -> Rendered: raw markdown -> from_markdown parses it.
        let raw = "`123`";
        let tree = InlineTextTree::from_markdown(raw);
        assert_eq!(tree.visible_text(), "123");
        assert!(tree.render_cache().style_at(0).code);

        // Serialize back: must produce valid markdown.
        let serialized = tree.serialize_markdown();
        assert_eq!(serialized, "`123`");

        // Re-parse: must produce same result.
        let reparsed = InlineTextTree::from_markdown(&serialized);
        assert_eq!(reparsed.visible_text(), "123");
        assert!(reparsed.render_cache().style_at(0).code);
    }

    #[test]
    fn raw_text_with_backticks_not_double_escaped() {
        // Simulate the Source block's display_text() path.
        let raw = "`123`";
        // display_text() returns raw text as-is; from_markdown re-parses.
        let parsed = InlineTextTree::from_markdown(raw);
        assert_eq!(parsed.visible_text(), "123");

        // A second round-trip should NOT escape or double the backticks.
        let serialized = parsed.serialize_markdown();
        assert_eq!(serialized, "`123`");
        let reparsed = InlineTextTree::from_markdown(&serialized);
        assert_eq!(reparsed.visible_text(), "123");
    }

    #[test]
    fn escaped_backtick_in_code() {
        let tree = InlineTextTree::from_markdown("\\`not code\\`");
        assert_eq!(tree.visible_text(), "`not code`");
        // Escaped backticks are literal, not code delimiters.
        let cache = tree.render_cache();
        assert!(!cache.style_at(0).code);
        assert_eq!(tree.serialize_markdown(), "\\`not code\\`");
    }
}
