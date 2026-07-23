use super::{SourceParseError, parse_document};
use crate::{
    DocumentCompatibility, SourceBlock, SourceBlockKind, SourceDiagnostic, SourceEdit,
    SourceInlineKind, SourceMarkdownDocument, SourceNodeCompatibility,
};
use std::ops::Range;

pub(super) fn reparse_single_block(
    document: &SourceMarkdownDocument,
    source: &str,
    edits: &[SourceEdit],
    revision: u64,
) -> Result<Option<SourceMarkdownDocument>, SourceParseError> {
    let Some((block_index, block)) = edited_block(document, edits) else {
        return Ok(None);
    };
    let Some(delta) = edit_delta(edits) else {
        return Ok(None);
    };
    let Some(new_end) = block.source_range.end.checked_add_signed(delta) else {
        return Ok(None);
    };
    let fragment = &source[block.source_range.start..new_end];
    let parsed = parse_document(fragment.to_owned(), revision)?;
    if parsed.blocks.len() != 1 {
        return Ok(None);
    }
    let mut new_block = parsed.blocks[0].clone();
    if kind_tag(&block.kind) != kind_tag(&new_block.kind) {
        return Ok(None);
    }
    shift_block(&mut new_block, block.source_range.start as isize)?;
    new_block.id = block.id;
    reassign_inline_ids(&mut new_block, max_node_id(document).saturating_add(1));
    let mut blocks = document.blocks.clone();
    blocks[block_index] = new_block;
    for trailing in &mut blocks[block_index + 1..] {
        shift_block(trailing, delta)?;
    }
    let compatibility = document_compatibility(&blocks);
    Ok(Some(SourceMarkdownDocument {
        source: source.to_owned(),
        revision,
        blocks,
        diagnostics: compatibility.diagnostics.clone(),
        compatibility,
    }))
}

fn max_node_id(document: &SourceMarkdownDocument) -> u64 {
    document
        .blocks
        .iter()
        .flat_map(|block| {
            std::iter::once(block.id.0)
                .chain(block.inline_nodes.iter().map(|node| node.id.0))
                .chain(match &block.kind {
                    SourceBlockKind::Table(table) => table
                        .rows
                        .iter()
                        .flat_map(|row| row.cells.iter())
                        .flat_map(|cell| cell.inline_nodes.iter())
                        .map(|node| node.id.0)
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                })
        })
        .max()
        .unwrap_or(0)
}

fn reassign_inline_ids(block: &mut SourceBlock, mut next: u64) {
    for inline in &mut block.inline_nodes {
        inline.id = crate::SourceNodeId(next);
        next = next.saturating_add(1);
    }
    if let SourceBlockKind::Table(table) = &mut block.kind {
        for inline in table
            .rows
            .iter_mut()
            .flat_map(|row| row.cells.iter_mut())
            .flat_map(|cell| cell.inline_nodes.iter_mut())
        {
            inline.id = crate::SourceNodeId(next);
            next = next.saturating_add(1);
        }
    }
}

fn edited_block<'a>(
    document: &'a SourceMarkdownDocument,
    edits: &[SourceEdit],
) -> Option<(usize, &'a SourceBlock)> {
    let first = edits.first()?;
    let index = document.blocks.iter().position(|block| {
        block.source_range.start <= first.range.start && first.range.end <= block.source_range.end
    })?;
    let block = &document.blocks[index];
    edits
        .iter()
        .all(|edit| contains(block, &edit.range))
        .then_some((index, block))
}

fn contains(block: &SourceBlock, range: &Range<usize>) -> bool {
    block.source_range.start <= range.start && range.end <= block.source_range.end
}

fn edit_delta(edits: &[SourceEdit]) -> Option<isize> {
    edits.iter().try_fold(0_isize, |delta, edit| {
        delta.checked_add(edit.replacement.len() as isize - edit.range.len() as isize)
    })
}

fn shift_block(block: &mut SourceBlock, delta: isize) -> Result<(), SourceParseError> {
    shift_range(&mut block.source_range, delta)?;
    shift_optional(&mut block.content_range, delta)?;
    shift_kind(&mut block.kind, delta)?;
    for inline in &mut block.inline_nodes {
        shift_range(&mut inline.source_range, delta)?;
        shift_optional(&mut inline.content_range, delta)?;
        shift_inline_kind(&mut inline.kind, delta)?;
    }
    Ok(())
}

fn shift_kind(kind: &mut SourceBlockKind, delta: isize) -> Result<(), SourceParseError> {
    match kind {
        SourceBlockKind::Heading { marker_range, .. } => shift_range(marker_range, delta),
        SourceBlockKind::CodeFence {
            opening_fence,
            closing_fence,
            language_range,
        } => {
            shift_range(opening_fence, delta)?;
            shift_optional(closing_fence, delta)?;
            shift_optional(language_range, delta)
        }
        SourceBlockKind::MathBlock {
            opening_marker,
            closing_marker,
        } => {
            shift_range(opening_marker, delta)?;
            shift_range(closing_marker, delta)
        }
        SourceBlockKind::Table(table) => {
            shift_range(&mut table.table_range, delta)?;
            shift_range(&mut table.delimiter_row, delta)?;
            for row in &mut table.rows {
                shift_range(&mut row.full_range, delta)?;
                for cell in &mut row.cells {
                    shift_range(&mut cell.full_range, delta)?;
                    shift_range(&mut cell.content_range, delta)?;
                    for inline in &mut cell.inline_nodes {
                        shift_range(&mut inline.source_range, delta)?;
                        shift_optional(&mut inline.content_range, delta)?;
                        shift_inline_kind(&mut inline.kind, delta)?;
                    }
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn shift_inline_kind(kind: &mut SourceInlineKind, delta: isize) -> Result<(), SourceParseError> {
    match kind {
        SourceInlineKind::Emphasis {
            opening_marker,
            closing_marker,
        }
        | SourceInlineKind::Strong {
            opening_marker,
            closing_marker,
        }
        | SourceInlineKind::InlineCode {
            opening_marker,
            closing_marker,
        }
        | SourceInlineKind::InlineMath {
            opening_marker,
            closing_marker,
        }
        | SourceInlineKind::Delete {
            opening_marker,
            closing_marker,
        } => {
            shift_range(opening_marker, delta)?;
            shift_range(closing_marker, delta)
        }
        SourceInlineKind::Link(link) => {
            shift_range(&mut link.full_range, delta)?;
            shift_range(&mut link.label_range, delta)?;
            shift_range(&mut link.destination_range, delta)
        }
        SourceInlineKind::Image(image) => {
            shift_range(&mut image.full_range, delta)?;
            shift_range(&mut image.alt_range, delta)?;
            shift_range(&mut image.destination_range, delta)?;
            if let Some(link) = &mut image.outer_link {
                shift_range(&mut link.full_range, delta)?;
                shift_range(&mut link.label_range, delta)?;
                shift_range(&mut link.destination_range, delta)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn shift_optional(range: &mut Option<Range<usize>>, delta: isize) -> Result<(), SourceParseError> {
    if let Some(range) = range {
        shift_range(range, delta)?;
    }
    Ok(())
}

fn shift_range(range: &mut Range<usize>, delta: isize) -> Result<(), SourceParseError> {
    range.start = range.start.checked_add_signed(delta).ok_or_else(overflow)?;
    range.end = range.end.checked_add_signed(delta).ok_or_else(overflow)?;
    Ok(())
}

fn overflow() -> SourceParseError {
    SourceParseError::Markdown("incremental range overflow".to_owned())
}

fn kind_tag(kind: &SourceBlockKind) -> u8 {
    match kind {
        SourceBlockKind::Heading { .. } => 0,
        SourceBlockKind::Paragraph => 1,
        SourceBlockKind::BlockQuote => 2,
        SourceBlockKind::OrderedList { .. } => 3,
        SourceBlockKind::UnorderedList => 4,
        SourceBlockKind::CodeFence { .. } => 5,
        SourceBlockKind::MathBlock { .. } => 6,
        SourceBlockKind::Table(_) => 7,
        SourceBlockKind::FrontMatter => 8,
        SourceBlockKind::Html => 9,
        SourceBlockKind::ThematicBreak => 10,
        SourceBlockKind::RawMarkdown => 11,
    }
}

fn document_compatibility(blocks: &[SourceBlock]) -> DocumentCompatibility {
    let source_only_nodes = blocks
        .iter()
        .filter(|block| block.compatibility != SourceNodeCompatibility::Editable)
        .map(|block| block.id)
        .collect::<Vec<_>>();
    let diagnostics = blocks
        .iter()
        .filter(|block| block.compatibility != SourceNodeCompatibility::Editable)
        .map(source_diagnostic)
        .collect();
    let source_only = source_only_nodes.len();
    DocumentCompatibility {
        fully_editable: source_only == 0,
        partially_editable: source_only > 0 && source_only < blocks.len(),
        source_only_nodes,
        diagnostics,
    }
}

fn source_diagnostic(block: &SourceBlock) -> SourceDiagnostic {
    SourceDiagnostic {
        severity: crate::SourceDiagnosticSeverity::Warning,
        code: "markdown.source.node_requires_source_editing",
        message: "This Markdown node is preserved and edited as source".to_owned(),
        source_range: Some(block.source_range.clone()),
        node_id: Some(block.id),
    }
}
