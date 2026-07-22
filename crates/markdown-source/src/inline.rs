use crate::fingerprint::semantic_node;
use crate::{SourceImageMap, SourceInlineKind, SourceInlineNode, SourceLinkMap, SourceNodeId};
use markdown::mdast::Node;
use std::ops::Range;

pub(crate) struct InlineMapper<'a> {
    source: &'a str,
    offset_shift: usize,
    next_id: &'a mut u64,
}

impl<'a> InlineMapper<'a> {
    pub(crate) fn new(source: &'a str, offset_shift: usize, next_id: &'a mut u64) -> Self {
        Self {
            source,
            offset_shift,
            next_id,
        }
    }

    pub(crate) fn collect(&mut self, children: &[Node]) -> Vec<SourceInlineNode> {
        let mut result = Vec::new();
        for child in children {
            self.collect_node(child, None, &mut result);
        }
        result
    }

    fn collect_node(
        &mut self,
        node: &Node,
        outer_link: Option<SourceLinkMap>,
        result: &mut Vec<SourceInlineNode>,
    ) {
        let Some(range) = shifted_range(node, self.offset_shift) else {
            return;
        };
        let kind = self.inline_kind(node, range.clone(), outer_link.clone());
        let content_range = content_range(node, &range, self.offset_shift);
        let nested_outer = match &kind {
            SourceInlineKind::Link(link) => Some(link.clone()),
            _ => outer_link,
        };
        result.push(SourceInlineNode {
            id: self.allocate_id(),
            kind,
            source_range: range,
            content_range,
            fingerprint: semantic_node(node),
        });
        if let Some(children) = node.children() {
            for child in children {
                self.collect_node(child, nested_outer.clone(), result);
            }
        }
    }

    fn inline_kind(
        &self,
        node: &Node,
        range: Range<usize>,
        outer_link: Option<SourceLinkMap>,
    ) -> SourceInlineKind {
        match node {
            Node::Text(_) => SourceInlineKind::Text,
            Node::Emphasis(_) => delimiter_kind(node, range, 1, true, self.offset_shift),
            Node::Strong(_) => delimiter_kind(node, range, 2, false, self.offset_shift),
            Node::InlineCode(_) => code_kind(self.source, range),
            Node::Link(_) => link_map(self.source, range)
                .map_or(SourceInlineKind::RawMarkdown, SourceInlineKind::Link),
            Node::Image(_) => image_map(self.source, range, outer_link)
                .map_or(SourceInlineKind::RawMarkdown, SourceInlineKind::Image),
            Node::Delete(_) => delimiter_kind(node, range, 2, false, self.offset_shift),
            Node::Break(_) => SourceInlineKind::HardBreak,
            Node::Html(_) => SourceInlineKind::Html,
            _ => SourceInlineKind::RawMarkdown,
        }
    }

    fn allocate_id(&mut self) -> SourceNodeId {
        let id = SourceNodeId(*self.next_id);
        *self.next_id = self.next_id.saturating_add(1);
        id
    }
}

fn shifted_range(node: &Node, shift: usize) -> Option<Range<usize>> {
    let position = node.position()?;
    Some(position.start.offset + shift..position.end.offset + shift)
}

fn content_range(node: &Node, range: &Range<usize>, shift: usize) -> Option<Range<usize>> {
    let children = node.children()?;
    let first = children.first()?.position()?;
    let last = children.last()?.position()?;
    Some(first.start.offset + shift..last.end.offset + shift)
        .filter(|value| value.start >= range.start && value.end <= range.end)
}

fn delimiter_kind(
    node: &Node,
    range: Range<usize>,
    marker_len: usize,
    emphasis: bool,
    shift: usize,
) -> SourceInlineKind {
    let content = content_range(node, &range, shift).unwrap_or(range.clone());
    let opening = range.start..content.start.max(range.start + marker_len);
    let closing = content.end.min(range.end - marker_len)..range.end;
    if emphasis {
        SourceInlineKind::Emphasis {
            opening_marker: opening,
            closing_marker: closing,
        }
    } else if marker_len == 2 && self_marker_is_delete(node) {
        SourceInlineKind::Delete {
            opening_marker: opening,
            closing_marker: closing,
        }
    } else {
        SourceInlineKind::Strong {
            opening_marker: opening,
            closing_marker: closing,
        }
    }
}

fn self_marker_is_delete(node: &Node) -> bool {
    matches!(node, Node::Delete(_))
}

fn code_kind(source: &str, range: Range<usize>) -> SourceInlineKind {
    let raw = &source[range.clone()];
    let marker_len = raw.bytes().take_while(|byte| *byte == b'`').count();
    SourceInlineKind::InlineCode {
        opening_marker: range.start..range.start + marker_len,
        closing_marker: range.end - marker_len..range.end,
    }
}

pub(crate) fn link_map(source: &str, range: Range<usize>) -> Option<SourceLinkMap> {
    let (label_range, destination_range) = resource_ranges(source, range.clone(), false)?;
    Some(SourceLinkMap {
        full_range: range,
        label_range,
        destination_range,
    })
}

fn image_map(
    source: &str,
    range: Range<usize>,
    outer_link: Option<SourceLinkMap>,
) -> Option<SourceImageMap> {
    let (alt_range, destination_range) = resource_ranges(source, range.clone(), true)?;
    let full_range = outer_link
        .as_ref()
        .map_or_else(|| range.clone(), |link| link.full_range.clone());
    Some(SourceImageMap {
        full_range,
        alt_range,
        destination_range,
        outer_link,
    })
}

fn resource_ranges(
    source: &str,
    range: Range<usize>,
    image: bool,
) -> Option<(Range<usize>, Range<usize>)> {
    let raw = &source[range.clone()];
    let label_start = usize::from(image) + 1;
    let label_end = find_unescaped(raw, b']', label_start)?;
    let open = label_end + 1;
    if raw.as_bytes().get(open) != Some(&b'(') {
        return None;
    }
    let (destination_start, destination_end) = destination_bounds(raw, open + 1)?;
    Some((
        range.start + label_start..range.start + label_end,
        range.start + destination_start..range.start + destination_end,
    ))
}

fn destination_bounds(raw: &str, mut start: usize) -> Option<(usize, usize)> {
    while raw
        .as_bytes()
        .get(start)
        .is_some_and(u8::is_ascii_whitespace)
    {
        start += 1;
    }
    if raw.as_bytes().get(start) == Some(&b'<') {
        let end = find_unescaped(raw, b'>', start + 1)?;
        return Some((start + 1, end));
    }
    let mut depth = 0_u32;
    let bytes = raw.as_bytes();
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = index.saturating_add(2),
            b'(' => depth = depth.saturating_add(1),
            b')' if depth == 0 => return Some((start, index)),
            b')' => depth = depth.saturating_sub(1),
            byte if byte.is_ascii_whitespace() && depth == 0 => return Some((start, index)),
            _ => {}
        }
        index += 1;
    }
    None
}

fn find_unescaped(value: &str, needle: u8, start: usize) -> Option<usize> {
    let bytes = value.as_bytes();
    (start..bytes.len())
        .find(|index| bytes[*index] == needle && (*index == 0 || bytes[*index - 1] != b'\\'))
}
