use crate::fingerprint::{SourceFingerprint, semantic_node};
use crate::inline::InlineMapper;
use crate::{
    DocumentCompatibility, SourceBlock, SourceBlockKind, SourceDiagnostic,
    SourceDiagnosticSeverity, SourceMarkdownDocument, SourceNodeCompatibility, SourceNodeId,
};
use markdown::ParseOptions;
use markdown::mdast::Node;
use std::ops::Range;

mod block;
mod directive;
mod incremental;

use directive::directive_ranges;

const BOM: &str = "\u{feff}";
const FIRST_NODE_ID: u64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum SourceParseError {
    #[error("Markdown parse failed: {0}")]
    Markdown(String),
}

pub(crate) fn parse_options() -> ParseOptions {
    let mut options = ParseOptions::gfm();
    options.constructs.frontmatter = true;
    options.constructs.math_flow = true;
    options.constructs.math_text = true;
    options
}

pub(crate) fn parse_document(
    source: String,
    revision: u64,
) -> Result<SourceMarkdownDocument, SourceParseError> {
    let body_start = source.starts_with(BOM).then_some(BOM.len()).unwrap_or(0);
    let tree = markdown::to_mdast(&source[body_start..], &parse_options())
        .map_err(|error| SourceParseError::Markdown(error.to_string()))?;
    let raw_ranges = directive_ranges(&source);
    let seeds = block_seeds(tree, &raw_ranges, body_start);
    let mut context = ParserContext::new(&source, body_start);
    let blocks = seeds
        .into_iter()
        .map(|seed| context.build(seed))
        .collect::<Vec<_>>();
    let compatibility = context.compatibility(blocks.len());
    Ok(SourceMarkdownDocument {
        source,
        revision,
        blocks,
        diagnostics: compatibility.diagnostics.clone(),
        compatibility,
    })
}

pub(crate) fn reparse_after_edits(
    document: &SourceMarkdownDocument,
    source: String,
    edits: &[crate::SourceEdit],
    revision: u64,
) -> Result<(SourceMarkdownDocument, crate::SourceParseScope), SourceParseError> {
    if let Some(incremental) =
        incremental::reparse_single_block(document, &source, edits, revision)?
    {
        return Ok((incremental, crate::SourceParseScope::SingleBlock));
    }
    parse_document(source, revision)
        .map(|document| (document, crate::SourceParseScope::FullDocument))
}

struct ParserContext<'a> {
    source: &'a str,
    offset_shift: usize,
    next_id: u64,
    source_only_nodes: Vec<SourceNodeId>,
    diagnostics: Vec<SourceDiagnostic>,
}

impl<'a> ParserContext<'a> {
    fn new(source: &'a str, offset_shift: usize) -> Self {
        Self {
            source,
            offset_shift,
            next_id: FIRST_NODE_ID,
            source_only_nodes: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn build(&mut self, seed: BlockSeed) -> SourceBlock {
        match seed.node {
            Some(node) => self.build_node(node, seed.range),
            None => self.build_raw(seed.range),
        }
    }

    fn build_node(&mut self, node: Node, range: Range<usize>) -> SourceBlock {
        let id = self.allocate_id();
        let children = node.children().cloned().unwrap_or_default();
        let mut inline_mapper =
            InlineMapper::new(self.source, self.offset_shift, &mut self.next_id);
        let inline_nodes = inline_mapper.collect(&children);
        let (kind, content_range, compatibility) = self.block_details(&node, range.clone());
        self.record_compatibility(id, compatibility, range.clone());
        SourceBlock {
            id,
            kind,
            source_range: range.clone(),
            content_range,
            original_source: self.source[range].to_owned(),
            fingerprint: semantic_node(&node),
            inline_nodes,
            compatibility,
        }
    }

    fn build_raw(&mut self, range: Range<usize>) -> SourceBlock {
        let id = self.allocate_id();
        self.record_compatibility(id, SourceNodeCompatibility::PreservedRaw, range.clone());
        SourceBlock {
            id,
            kind: SourceBlockKind::RawMarkdown,
            source_range: range.clone(),
            content_range: Some(range.clone()),
            original_source: self.source[range.clone()].to_owned(),
            fingerprint: SourceFingerprint::from_semantics(&self.source[range]),
            inline_nodes: Vec::new(),
            compatibility: SourceNodeCompatibility::PreservedRaw,
        }
    }

    fn record_compatibility(
        &mut self,
        id: SourceNodeId,
        compatibility: SourceNodeCompatibility,
        range: Range<usize>,
    ) {
        if compatibility == SourceNodeCompatibility::Editable {
            return;
        }
        self.source_only_nodes.push(id);
        self.diagnostics.push(SourceDiagnostic {
            severity: SourceDiagnosticSeverity::Warning,
            code: "markdown.source.node_requires_source_editing",
            message: "This Markdown node is preserved and edited as source".to_owned(),
            source_range: Some(range),
            node_id: Some(id),
        });
    }

    fn compatibility(&self, block_count: usize) -> DocumentCompatibility {
        let source_only = self.source_only_nodes.len();
        DocumentCompatibility {
            fully_editable: source_only == 0,
            partially_editable: source_only > 0 && source_only < block_count,
            source_only_nodes: self.source_only_nodes.clone(),
            diagnostics: self.diagnostics.clone(),
        }
    }

    fn allocate_id(&mut self) -> SourceNodeId {
        let id = SourceNodeId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }
}

struct BlockSeed {
    range: Range<usize>,
    node: Option<Node>,
}

fn block_seeds(tree: Node, raw_ranges: &[Range<usize>], shift: usize) -> Vec<BlockSeed> {
    let mut seeds = match tree {
        Node::Root(root) => root
            .children
            .into_iter()
            .filter_map(|node| node_seed(node, raw_ranges, shift))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    seeds.extend(
        raw_ranges
            .iter()
            .cloned()
            .map(|range| BlockSeed { range, node: None }),
    );
    seeds.sort_by_key(|seed| seed.range.start);
    seeds
}

fn node_seed(node: Node, raw_ranges: &[Range<usize>], shift: usize) -> Option<BlockSeed> {
    let position = node.position()?;
    let range = position.start.offset + shift..position.end.offset + shift;
    if raw_ranges.iter().any(|raw| overlaps(raw, &range)) {
        return None;
    }
    Some(BlockSeed {
        range,
        node: Some(node),
    })
}

fn overlaps(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}
