use crate::SourceNodeId;
use std::ops::Range;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceSelection {
    pub anchor: usize,
    pub head: usize,
}

impl SourceSelection {
    pub fn ordered_range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveInlineSource {
    pub node_id: SourceNodeId,
    pub source_range: Range<usize>,
    pub content_range: Option<Range<usize>>,
    pub source: String,
}
