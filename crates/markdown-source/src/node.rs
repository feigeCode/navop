use crate::{SourceFingerprint, SourceNodeCompatibility, SourceTableMap};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceNodeId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBlock {
    pub id: SourceNodeId,
    pub kind: SourceBlockKind,
    pub source_range: Range<usize>,
    pub content_range: Option<Range<usize>>,
    pub original_source: String,
    pub fingerprint: SourceFingerprint,
    pub inline_nodes: Vec<SourceInlineNode>,
    pub compatibility: SourceNodeCompatibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceBlockKind {
    Heading {
        level: u8,
        marker_range: Range<usize>,
    },
    Paragraph,
    BlockQuote,
    OrderedList {
        start: u64,
    },
    UnorderedList,
    CodeFence {
        opening_fence: Range<usize>,
        closing_fence: Option<Range<usize>>,
        language_range: Option<Range<usize>>,
    },
    MathBlock {
        opening_marker: Range<usize>,
        closing_marker: Range<usize>,
    },
    Table(SourceTableMap),
    FrontMatter,
    Html,
    ThematicBreak,
    RawMarkdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInlineNode {
    pub id: SourceNodeId,
    pub kind: SourceInlineKind,
    pub source_range: Range<usize>,
    pub content_range: Option<Range<usize>>,
    pub fingerprint: SourceFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceInlineKind {
    Text,
    Emphasis {
        opening_marker: Range<usize>,
        closing_marker: Range<usize>,
    },
    Strong {
        opening_marker: Range<usize>,
        closing_marker: Range<usize>,
    },
    InlineCode {
        opening_marker: Range<usize>,
        closing_marker: Range<usize>,
    },
    InlineMath {
        opening_marker: Range<usize>,
        closing_marker: Range<usize>,
    },
    Link(SourceLinkMap),
    Image(SourceImageMap),
    Delete {
        opening_marker: Range<usize>,
        closing_marker: Range<usize>,
    },
    HardBreak,
    SoftBreak,
    Html,
    RawMarkdown,
}

impl SourceInlineKind {
    pub(crate) fn reveals_source_when_active(&self) -> bool {
        !matches!(self, Self::Text | Self::HardBreak | Self::SoftBreak)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLinkMap {
    pub full_range: Range<usize>,
    pub label_range: Range<usize>,
    pub destination_range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceImageMap {
    pub full_range: Range<usize>,
    pub alt_range: Range<usize>,
    pub destination_range: Range<usize>,
    pub outer_link: Option<SourceLinkMap>,
}
