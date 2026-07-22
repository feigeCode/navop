use super::ParserContext;
use crate::table::build_table_map;
use crate::{SourceBlockKind, SourceNodeCompatibility};
use markdown::mdast::Node;
use std::ops::Range;

const MINIMUM_FENCE_LENGTH: usize = 3;

type BlockDetails = (
    SourceBlockKind,
    Option<Range<usize>>,
    SourceNodeCompatibility,
);

impl ParserContext<'_> {
    pub(super) fn block_details(&mut self, node: &Node, range: Range<usize>) -> BlockDetails {
        match node {
            Node::Heading(value) => heading_details(self.source, range, value.depth),
            Node::Paragraph(_) => (
                SourceBlockKind::Paragraph,
                Some(range),
                paragraph_mode(node),
            ),
            Node::Blockquote(_) => editable(SourceBlockKind::BlockQuote, None),
            Node::List(value) if value.ordered => editable(
                SourceBlockKind::OrderedList {
                    start: u64::from(value.start.unwrap_or(1)),
                },
                None,
            ),
            Node::List(_) => editable(SourceBlockKind::UnorderedList, None),
            Node::Code(_) => code_details(self.source, range),
            Node::Table(_) => editable(
                SourceBlockKind::Table(build_table_map(self.source, range, &mut self.next_id)),
                None,
            ),
            Node::Yaml(_) | Node::Toml(_) => source_editable(SourceBlockKind::FrontMatter, range),
            Node::Html(_) => source_editable(SourceBlockKind::Html, range),
            Node::ThematicBreak(_) => editable(SourceBlockKind::ThematicBreak, None),
            _ => (
                SourceBlockKind::RawMarkdown,
                Some(range),
                SourceNodeCompatibility::PreservedRaw,
            ),
        }
    }
}

fn editable(kind: SourceBlockKind, content: Option<Range<usize>>) -> BlockDetails {
    (kind, content, SourceNodeCompatibility::Editable)
}

fn source_editable(kind: SourceBlockKind, range: Range<usize>) -> BlockDetails {
    (kind, Some(range), SourceNodeCompatibility::SourceEditable)
}

fn heading_details(source: &str, range: Range<usize>, level: u8) -> BlockDetails {
    let raw = &source[range.clone()];
    let marker_len = raw.bytes().take_while(|byte| *byte == b'#').count();
    let whitespace = raw[marker_len..]
        .bytes()
        .take_while(u8::is_ascii_whitespace)
        .count();
    let content_start = range.start + marker_len + whitespace;
    editable(
        SourceBlockKind::Heading {
            level,
            marker_range: range.start..range.start + marker_len,
        },
        Some(content_start..range.end),
    )
}

fn paragraph_mode(node: &Node) -> SourceNodeCompatibility {
    let source_only = node.children().is_some_and(|children| {
        children.iter().any(|child| {
            matches!(
                child,
                Node::Html(_) | Node::MdxJsxTextElement(_) | Node::MdxTextExpression(_)
            )
        })
    });
    source_only
        .then_some(SourceNodeCompatibility::SourceEditable)
        .unwrap_or(SourceNodeCompatibility::Editable)
}

fn code_details(source: &str, range: Range<usize>) -> BlockDetails {
    let raw = &source[range.clone()];
    let first_end = raw.find('\n').unwrap_or(raw.len());
    let opening = &raw[..first_end];
    let trimmed = opening.trim_start();
    let fence_len = trimmed
        .bytes()
        .take_while(|byte| matches!(byte, b'`' | b'~'))
        .count();
    if fence_len < MINIMUM_FENCE_LENGTH {
        return source_editable(SourceBlockKind::RawMarkdown, range);
    }
    fenced_code_details(
        source,
        range,
        FenceStart {
            line_end: first_end,
            marker_len: fence_len,
        },
    )
}

struct FenceStart {
    line_end: usize,
    marker_len: usize,
}

fn fenced_code_details(source: &str, range: Range<usize>, fence: FenceStart) -> BlockDetails {
    let opening = &source[range.start..range.start + fence.line_end];
    let trimmed = opening.trim_start();
    let indent = opening.len() - trimmed.len();
    let fence_start = range.start + indent;
    let language_range = language_range(
        source,
        fence_start + fence.marker_len..range.start + fence.line_end,
    );
    let closing_fence = closing_fence_range(
        source,
        range.clone(),
        FenceMarker {
            byte: trimmed.as_bytes()[0],
            len: fence.marker_len,
        },
    );
    editable(
        SourceBlockKind::CodeFence {
            opening_fence: fence_start..fence_start + fence.marker_len,
            closing_fence,
            language_range,
        },
        code_content_range(source, range),
    )
}

fn language_range(source: &str, range: Range<usize>) -> Option<Range<usize>> {
    let raw = &source[range.clone()];
    let leading = raw.len() - raw.trim_start().len();
    let end = raw.find(char::is_whitespace).unwrap_or(raw.len());
    (leading < end).then_some(range.start + leading..range.start + end)
}

struct FenceMarker {
    byte: u8,
    len: usize,
}

fn closing_fence_range(
    source: &str,
    range: Range<usize>,
    marker: FenceMarker,
) -> Option<Range<usize>> {
    let raw = &source[range.clone()];
    let line_start = raw.rfind('\n').map_or(0, |index| index + 1);
    let trimmed = raw[line_start..].trim_start();
    let actual = trimmed
        .bytes()
        .take_while(|byte| *byte == marker.byte)
        .count();
    (actual >= marker.len).then(|| {
        let indent = raw[line_start..].len() - trimmed.len();
        let start = range.start + line_start + indent;
        start..start + actual
    })
}

fn code_content_range(source: &str, range: Range<usize>) -> Option<Range<usize>> {
    let raw = &source[range.clone()];
    let start = raw.find('\n')? + 1;
    let end = raw.rfind('\n').unwrap_or(raw.len());
    Some(range.start + start..range.start + end)
}
