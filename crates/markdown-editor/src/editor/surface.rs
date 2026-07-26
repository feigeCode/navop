use markdown_source::{SourceBlockKind, SourceMarkdownDocument, SourceNodeId, TableCellAddress};
use std::ops::Range;

mod state;

/// The mode belongs to a surface rather than to the editor as a whole.  A
/// document can have a rich paragraph, a code fence and a table cell mounted
/// at the same time, so changing focus must not mutate the presentation mode
/// of another mounted input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MarkdownInputMode {
    RichText,
    Code(String),
}

/// A key for an editor-owned, long-lived editable surface.
///
/// `TableCellAddress` intentionally does not implement `Hash` in
/// markdown-source, therefore table coordinates are represented explicitly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum MarkdownSurfaceKey {
    Empty,
    Block(SourceNodeId),
    TableCell {
        block_id: SourceNodeId,
        row: usize,
        column: usize,
    },
}

impl MarkdownSurfaceKey {
    pub(super) fn block(block_id: SourceNodeId) -> Self {
        Self::Block(block_id)
    }

    pub(super) fn table_cell(address: TableCellAddress) -> Self {
        Self::TableCell {
            block_id: address.block_id,
            row: address.row,
            column: address.column,
        }
    }

    pub(super) fn table_address(self) -> Option<TableCellAddress> {
        match self {
            Self::TableCell {
                block_id,
                row,
                column,
            } => Some(TableCellAddress {
                block_id,
                row,
                column,
            }),
            _ => None,
        }
    }
}

/// State that must remain mounted at one physical location in the element
/// tree.  In particular, an InputState must never be shared by two blocks:
/// its layout cache, bounds, scroll handle and focus handle are all
/// surface-local.
pub(super) struct MarkdownEditSurface {
    pub(super) input: gpui::Entity<gpui_component::input::InputState>,
    pub(super) projection: crate::MarkdownProjection,
    pub(super) mode: MarkdownInputMode,
    pub(super) _subscriptions: Vec<gpui::Subscription>,
}

pub(super) struct SurfaceProjectionUpdate {
    pub(super) key: MarkdownSurfaceKey,
    pub(super) projection: crate::MarkdownProjection,
    pub(super) selection: Option<markdown_source::SourceSelection>,
}

pub(super) fn surface_specs(
    document: &SourceMarkdownDocument,
) -> Vec<(MarkdownSurfaceKey, Range<usize>)> {
    let mut specs = vec![(MarkdownSurfaceKey::Empty, 0..document.source.len())];
    for block in &document.blocks {
        match &block.kind {
            SourceBlockKind::Table(table) => {
                for (row, source_row) in table.rows.iter().enumerate() {
                    if row == 1 {
                        continue;
                    }
                    for (column, cell) in source_row.cells.iter().enumerate() {
                        specs.push((
                            MarkdownSurfaceKey::TableCell {
                                block_id: block.id,
                                row,
                                column,
                            },
                            cell.content_range.clone(),
                        ));
                    }
                }
            }
            _ => specs.push((
                MarkdownSurfaceKey::Block(block.id),
                block.source_range.clone(),
            )),
        }
    }
    specs
}

pub(super) fn projection_for(
    document: &SourceMarkdownDocument,
    key: MarkdownSurfaceKey,
    active_inline: Option<SourceNodeId>,
) -> Option<crate::MarkdownProjection> {
    let range = match key {
        MarkdownSurfaceKey::Empty => 0..document.source.len(),
        MarkdownSurfaceKey::Block(block_id) => document.block_by_id(block_id)?.source_range.clone(),
        MarkdownSurfaceKey::TableCell {
            block_id,
            row,
            column,
        } => document
            .table_cell(TableCellAddress {
                block_id,
                row,
                column,
            })
            .ok()?
            .content_range
            .clone(),
    };
    Some(crate::MarkdownProjection::build_surface_range(
        document,
        active_inline,
        range,
    ))
}

pub(super) fn mode_for(
    document: &SourceMarkdownDocument,
    key: MarkdownSurfaceKey,
) -> MarkdownInputMode {
    let block_id = match key {
        MarkdownSurfaceKey::Block(id) | MarkdownSurfaceKey::TableCell { block_id: id, .. } => {
            Some(id)
        }
        MarkdownSurfaceKey::Empty => None,
    };
    let language = block_id.and_then(|id| {
        let block = document.block_by_id(id)?;
        match &block.kind {
            SourceBlockKind::CodeFence { language_range, .. } => {
                Some(fenced_code_language(document, language_range.as_ref()))
            }
            SourceBlockKind::MathBlock { .. } => Some("latex".to_owned()),
            _ => None,
        }
    });
    language.map_or(MarkdownInputMode::RichText, MarkdownInputMode::Code)
}

fn fenced_code_language(
    document: &SourceMarkdownDocument,
    language_range: Option<&Range<usize>>,
) -> String {
    let Some(range) = language_range else {
        return "text".to_owned();
    };
    let language = &document.source[range.clone()];
    if language.eq_ignore_ascii_case("mermaid") {
        return "text".to_owned();
    }
    gpui_component::highlighter::LanguageRegistry::singleton()
        .resolve_language_name(language)
        .unwrap_or_else(|| "text".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_component::highlighter::{LanguageManifest, LanguageRegistry};
    use std::path::PathBuf;

    fn first_block_mode(source: &str) -> MarkdownInputMode {
        let document = SourceMarkdownDocument::parse(source).unwrap();
        mode_for(&document, MarkdownSurfaceKey::Block(document.blocks[0].id))
    }

    #[test]
    fn code_fence_mode_resolves_registered_wasm_extension_alias() {
        let registry = LanguageRegistry::singleton();
        let language = "__markdown_fenced_wasm_test__";
        registry.register_wasm_manifest(
            LanguageManifest {
                name: language.to_string(),
                version: "0.1.0".to_string(),
                file_extensions: vec!["fence_alias".to_string()],
                injection_languages: Vec::new(),
                requires: Vec::new(),
                sha256_wasm: None,
            },
            PathBuf::from("/definitely/missing/markdown-language-extension"),
        );

        assert_eq!(
            MarkdownInputMode::Code(language.to_string()),
            first_block_mode("```FENCE_ALIAS title=\"example\"\nfn main() {}\n```")
        );
        assert!(registry.unregister(language));
    }

    #[test]
    fn unavailable_and_rendered_code_fences_use_plain_text_highlighting() {
        assert_eq!(
            MarkdownInputMode::Code("text".to_string()),
            first_block_mode("```not-installed\ncontent\n```")
        );
        assert_eq!(
            MarkdownInputMode::Code("text".to_string()),
            first_block_mode("```mermaid\ngraph LR\n```")
        );
        assert_eq!(
            MarkdownInputMode::Code("text".to_string()),
            first_block_mode("```\ncontent\n```")
        );
    }
}
