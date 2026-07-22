use crate::{
    DocumentCompatibility, SourceBlock, SourceDiagnostic, SourceEditTransaction, SourceParseError,
    SourceTransaction,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMarkdownDocument {
    pub source: String,
    pub revision: u64,
    pub blocks: Vec<SourceBlock>,
    pub diagnostics: Vec<SourceDiagnostic>,
    pub compatibility: DocumentCompatibility,
}

impl SourceMarkdownDocument {
    pub fn parse(source: impl Into<String>) -> Result<Self, SourceParseError> {
        crate::parser::parse_document(source.into(), 0)
    }

    pub fn replace_source(&self, source: impl Into<String>) -> Result<Self, SourceParseError> {
        crate::parser::parse_document(source.into(), self.revision.saturating_add(1))
    }

    pub fn apply_transaction(
        &self,
        transaction: &SourceTransaction,
    ) -> Result<SourceEditTransaction, crate::PatchError> {
        crate::transaction::apply_transaction(self, transaction)
    }

    pub fn inline_node_at(&self, byte_offset: usize) -> Option<&crate::SourceInlineNode> {
        self.blocks
            .iter()
            .flat_map(block_inline_nodes)
            .filter(|node| node.kind.reveals_source_when_active())
            .filter(|node| {
                node.source_range.start <= byte_offset && byte_offset < node.source_range.end
            })
            .min_by_key(|node| node.source_range.len())
    }

    pub fn block_at(&self, byte_offset: usize) -> Option<&SourceBlock> {
        self.blocks.iter().find(|block| {
            block.source_range.start <= byte_offset && byte_offset < block.source_range.end
        })
    }

    pub fn block_by_id(&self, id: crate::SourceNodeId) -> Option<&SourceBlock> {
        self.blocks.iter().find(|block| block.id == id)
    }

    pub fn active_inline_source(&self, byte_offset: usize) -> Option<crate::ActiveInlineSource> {
        let node = self.inline_node_at(byte_offset)?;
        Some(crate::ActiveInlineSource {
            node_id: node.id,
            source_range: node.source_range.clone(),
            content_range: node.content_range.clone(),
            source: self.source[node.source_range.clone()].to_owned(),
        })
    }
}

fn block_inline_nodes(block: &SourceBlock) -> Vec<&crate::SourceInlineNode> {
    let mut nodes = block.inline_nodes.iter().collect::<Vec<_>>();
    if let crate::SourceBlockKind::Table(table) = &block.kind {
        nodes.extend(
            table
                .rows
                .iter()
                .flat_map(|row| row.cells.iter())
                .flat_map(|cell| cell.inline_nodes.iter()),
        );
    }
    nodes
}
