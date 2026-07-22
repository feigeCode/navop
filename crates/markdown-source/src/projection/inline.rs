use crate::{SourceBlock, SourceBlockKind, SourceEdit, SourceInlineKind, SourceInlineNode};
use std::ops::Range;

#[derive(Clone, Copy)]
pub(super) struct InlineProjection<'a> {
    pub old_source: &'a str,
    pub new_source: &'a str,
    pub old_block: &'a SourceBlock,
    pub new_block: &'a SourceBlock,
    pub revision: u64,
}

pub(super) fn reconcile_inline_edits(input: InlineProjection<'_>) -> Option<Vec<SourceEdit>> {
    if !same_block_shape(&input.old_block.kind, &input.new_block.kind) {
        return None;
    }
    let reconciler = InlineReconciler::new(input)?;
    let edits = reconciler.edits();
    (!edits.is_empty()).then_some(edits)
}

struct InlineReconciler<'a> {
    input: InlineProjection<'a>,
    old_nodes: Vec<&'a SourceInlineNode>,
    new_nodes: Vec<&'a SourceInlineNode>,
    matches: Vec<(usize, usize)>,
}

impl<'a> InlineReconciler<'a> {
    fn new(input: InlineProjection<'a>) -> Option<Self> {
        let old_nodes = projection_nodes(input.old_block);
        let new_nodes = projection_nodes(input.new_block);
        if old_nodes.is_empty() || new_nodes.is_empty() {
            return None;
        }
        let matches = matching_nodes(&old_nodes, &new_nodes);
        Some(Self {
            input,
            old_nodes,
            new_nodes,
            matches,
        })
    }

    fn edits(&self) -> Vec<SourceEdit> {
        let mut edits = Vec::new();
        let mut old_cursor = 0;
        let mut new_cursor = 0;
        let sentinel = (self.old_nodes.len(), self.new_nodes.len());
        for &(old_match, new_match) in self.matches.iter().chain(std::iter::once(&sentinel)) {
            self.append_gap(old_cursor..old_match, new_cursor..new_match, &mut edits);
            old_cursor = old_match.saturating_add(1);
            new_cursor = new_match.saturating_add(1);
        }
        edits
    }

    fn append_gap(
        &self,
        old_gap: Range<usize>,
        new_gap: Range<usize>,
        edits: &mut Vec<SourceEdit>,
    ) {
        if old_gap.len() == new_gap.len() {
            self.append_equal_count(old_gap, new_gap, edits);
            return;
        }
        let old_range = gap_range(self.input.old_block, &self.old_nodes, old_gap);
        let new_range = gap_range(self.input.new_block, &self.new_nodes, new_gap);
        let replacement = &self.input.new_source[new_range];
        if self.input.old_source[old_range.clone()] != *replacement {
            edits.push(SourceEdit::new(old_range, replacement, self.input.revision));
        }
    }

    fn append_equal_count(
        &self,
        old_gap: Range<usize>,
        new_gap: Range<usize>,
        edits: &mut Vec<SourceEdit>,
    ) {
        for (old_index, new_index) in old_gap.zip(new_gap) {
            let old = self.old_nodes[old_index];
            let new = self.new_nodes[new_index];
            let replacement = &self.input.new_source[new.source_range.clone()];
            if self.input.old_source[old.source_range.clone()] != *replacement {
                edits.push(SourceEdit::new(
                    old.source_range.clone(),
                    replacement,
                    self.input.revision,
                ));
            }
        }
    }
}

fn same_block_shape(old: &SourceBlockKind, new: &SourceBlockKind) -> bool {
    use SourceBlockKind as Kind;
    match (old, new) {
        (Kind::Heading { level: left, .. }, Kind::Heading { level: right, .. }) => left == right,
        (Kind::Paragraph, Kind::Paragraph)
        | (Kind::BlockQuote, Kind::BlockQuote)
        | (Kind::OrderedList { .. }, Kind::OrderedList { .. })
        | (Kind::UnorderedList, Kind::UnorderedList)
        | (Kind::Table(_), Kind::Table(_)) => true,
        _ => false,
    }
}

fn projection_nodes(block: &SourceBlock) -> Vec<&SourceInlineNode> {
    let mut selected: Vec<&SourceInlineNode> = Vec::new();
    for node in block
        .inline_nodes
        .iter()
        .filter(|node| is_projection_node(&node.kind))
    {
        if is_image_link(node, &block.inline_nodes) {
            continue;
        }
        let nested = selected
            .iter()
            .any(|parent| contains(&parent.source_range, &node.source_range));
        if !nested {
            selected.push(node);
        }
    }
    selected
}

fn is_projection_node(kind: &SourceInlineKind) -> bool {
    matches!(
        kind,
        SourceInlineKind::Text
            | SourceInlineKind::Emphasis { .. }
            | SourceInlineKind::Strong { .. }
            | SourceInlineKind::InlineCode { .. }
            | SourceInlineKind::Link(_)
            | SourceInlineKind::Image(_)
            | SourceInlineKind::Delete { .. }
            | SourceInlineKind::HardBreak
            | SourceInlineKind::Html
    )
}

fn is_image_link(node: &SourceInlineNode, nodes: &[SourceInlineNode]) -> bool {
    matches!(node.kind, SourceInlineKind::Link(_))
        && nodes.iter().any(|child| {
            matches!(child.kind, SourceInlineKind::Image(_))
                && contains(&node.source_range, &child.source_range)
        })
}

fn contains(parent: &Range<usize>, child: &Range<usize>) -> bool {
    parent.start <= child.start && child.end <= parent.end
}

fn matching_nodes(old: &[&SourceInlineNode], new: &[&SourceInlineNode]) -> Vec<(usize, usize)> {
    let width = new.len() + 1;
    let mut lengths = vec![0_usize; (old.len() + 1) * width];
    for old_index in (0..old.len()).rev() {
        for new_index in (0..new.len()).rev() {
            let index = old_index * width + new_index;
            lengths[index] = if old[old_index].fingerprint == new[new_index].fingerprint {
                lengths[(old_index + 1) * width + new_index + 1] + 1
            } else {
                lengths[(old_index + 1) * width + new_index]
                    .max(lengths[old_index * width + new_index + 1])
            };
        }
    }
    collect_matches(old, new, &lengths, width)
}

fn collect_matches(
    old: &[&SourceInlineNode],
    new: &[&SourceInlineNode],
    lengths: &[usize],
    width: usize,
) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    let (mut old_index, mut new_index) = (0, 0);
    while old_index < old.len() && new_index < new.len() {
        if old[old_index].fingerprint == new[new_index].fingerprint {
            result.push((old_index, new_index));
            old_index += 1;
            new_index += 1;
        } else if lengths[(old_index + 1) * width + new_index]
            >= lengths[old_index * width + new_index + 1]
        {
            old_index += 1;
        } else {
            new_index += 1;
        }
    }
    result
}

fn gap_range(block: &SourceBlock, nodes: &[&SourceInlineNode], gap: Range<usize>) -> Range<usize> {
    let bounds = content_bounds(block, nodes);
    let start = gap
        .start
        .checked_sub(1)
        .and_then(|index| nodes.get(index))
        .map_or(bounds.start, |node| node.source_range.end);
    let end = nodes
        .get(gap.end)
        .map_or(bounds.end, |node| node.source_range.start);
    start..end
}

fn content_bounds(block: &SourceBlock, nodes: &[&SourceInlineNode]) -> Range<usize> {
    block.content_range.clone().unwrap_or_else(|| {
        nodes
            .first()
            .map_or(block.source_range.start, |node| node.source_range.start)
            ..nodes
                .last()
                .map_or(block.source_range.end, |node| node.source_range.end)
    })
}
