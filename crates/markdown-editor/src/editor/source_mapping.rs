//! Source-offset mapping between canonical Markdown and rendered blocks.

use std::ops::Range;

use super::*;

impl Editor {
    pub(super) fn current_document_source(&self, cx: &App) -> String {
        match self.view_mode {
            ViewMode::Rendered => self.document.markdown_text(cx),
            ViewMode::Source => self.document.raw_source_text(cx),
        }
    }

    pub(super) fn is_empty_paragraph_separator(block: &Block) -> bool {
        block.kind() == BlockKind::Paragraph
            && block.record.title.visible_text().is_empty()
            && block.children.is_empty()
    }

    pub(super) fn is_empty_root_paragraph(block: &Block) -> bool {
        Self::is_empty_paragraph_separator(block)
    }

    pub(super) fn build_prefixed_content_mapping(
        content: &str,
        first_prefix: &str,
        continuation_prefix: &str,
    ) -> (String, Vec<usize>, Vec<usize>) {
        let mut full = String::new();
        let mut content_to_source = vec![0; content.len() + 1];
        let mut source_to_content = vec![0];

        full.push_str(first_prefix);
        source_to_content.resize(full.len() + 1, 0);

        let mut content_offset = 0usize;
        while content_offset < content.len() {
            content_to_source[content_offset] = full.len();
            let ch = content[content_offset..]
                .chars()
                .next()
                .expect("content offset should stay on char boundaries");
            let start = full.len();
            full.push(ch);
            source_to_content.resize(full.len() + 1, content_offset);
            for index in start..=full.len() {
                source_to_content[index] = content_offset;
            }
            content_offset += ch.len_utf8();
            if ch == '\n' {
                let prefix_start = full.len();
                full.push_str(continuation_prefix);
                source_to_content.resize(full.len() + 1, content_offset);
                for index in prefix_start..=full.len() {
                    source_to_content[index] = content_offset;
                }
            }
        }
        content_to_source[content.len()] = full.len();
        source_to_content[full.len()] = content.len();

        (full, content_to_source, source_to_content)
    }

    pub(super) fn build_code_block_content_mapping(
        content: &str,
        indentation: &str,
        language: Option<&SharedString>,
    ) -> (String, Vec<usize>, Vec<usize>) {
        let fence = self::persistence::safe_code_fence_with_info(
            content,
            language.map(|language| language.as_ref()),
        );
        let mut full = String::new();
        let mut content_to_source = vec![0; content.len() + 1];
        let mut source_to_content = vec![0];

        full.push_str(&fence);
        if let Some(language) = language {
            full.push_str(language);
        }
        full.push('\n');
        source_to_content.resize(full.len() + 1, 0);

        let prefix_start = full.len();
        full.push_str(indentation);
        source_to_content.resize(full.len() + 1, 0);
        for index in prefix_start..=full.len() {
            source_to_content[index] = 0;
        }

        let mut content_offset = 0usize;
        while content_offset < content.len() {
            content_to_source[content_offset] = full.len();
            let ch = content[content_offset..]
                .chars()
                .next()
                .expect("content offset should stay on char boundaries");
            let start = full.len();
            full.push(ch);
            source_to_content.resize(full.len() + 1, content_offset);
            for index in start..=full.len() {
                source_to_content[index] = content_offset;
            }
            content_offset += ch.len_utf8();
            if ch == '\n' {
                let line_prefix_start = full.len();
                full.push_str(indentation);
                source_to_content.resize(full.len() + 1, content_offset);
                for index in line_prefix_start..=full.len() {
                    source_to_content[index] = content_offset;
                }
            }
        }
        content_to_source[content.len()] = full.len();
        source_to_content[full.len()] = content.len();

        full.push('\n');
        source_to_content.resize(full.len() + 1, content.len());
        full.push_str(&fence);
        source_to_content.resize(full.len() + 1, content.len());
        source_to_content[full.len()] = content.len();

        (full, content_to_source, source_to_content)
    }

    pub(super) fn push_inline_block_mapping(
        &self,
        block: &Entity<Block>,
        content_markdown: String,
        first_prefix: String,
        continuation_prefix: String,
        quote_depth: usize,
        absolute_start: usize,
        mappings: &mut Vec<SourceTargetMapping>,
    ) -> usize {
        let (full_text, content_to_source, source_to_content) =
            Self::build_prefixed_content_mapping(
                &content_markdown,
                &first_prefix,
                &continuation_prefix,
            );
        let (full_text, content_to_source, source_to_content) =
            Self::wrap_source_mapping_with_quotes(
                full_text,
                content_to_source,
                source_to_content,
                quote_depth,
            );
        mappings.push(SourceTargetMapping {
            entity: block.clone(),
            full_source_range: absolute_start..absolute_start + full_text.len(),
            content_to_source,
            source_to_content,
        });
        full_text.len()
    }

    pub(super) fn push_footnote_definition_head_mapping(
        block: &Entity<Block>,
        footnote_id: &str,
        include_trailing_space: bool,
        quote_depth: usize,
        absolute_start: usize,
        mappings: &mut Vec<SourceTargetMapping>,
    ) -> usize {
        let mut full_text = format!("[^{footnote_id}]:");
        if include_trailing_space {
            full_text.push(' ');
        }

        let mut content_to_source = vec![0; footnote_id.len() + 1];
        let mut source_to_content = vec![0; full_text.len() + 1];
        let id_start = 2usize;
        for offset in 0..=footnote_id.len() {
            content_to_source[offset] = id_start + offset;
        }
        for source_offset in 0..=full_text.len() {
            source_to_content[source_offset] = if source_offset <= id_start {
                0
            } else if source_offset >= id_start + footnote_id.len() {
                footnote_id.len()
            } else {
                source_offset - id_start
            };
        }

        let (full_text, content_to_source, source_to_content) =
            Self::wrap_source_mapping_with_quotes(
                full_text,
                content_to_source,
                source_to_content,
                quote_depth,
            );
        mappings.push(SourceTargetMapping {
            entity: block.clone(),
            full_source_range: absolute_start..absolute_start + full_text.len(),
            content_to_source,
            source_to_content,
        });
        full_text.len()
    }

    pub(super) fn push_raw_block_mapping(
        &self,
        block: &Entity<Block>,
        quote_depth: usize,
        absolute_start: usize,
        mappings: &mut Vec<SourceTargetMapping>,
        cx: &App,
    ) -> usize {
        let (content, indentation) = {
            let block_ref = block.read(cx);
            (
                block_ref.display_text().to_string(),
                if block_ref.render_depth == 0 {
                    String::new()
                } else {
                    "  ".repeat(block_ref.render_depth)
                },
            )
        };
        let (full_text, content_to_source, source_to_content) =
            Self::build_prefixed_content_mapping(&content, &indentation, &indentation);
        let (full_text, content_to_source, source_to_content) =
            Self::wrap_source_mapping_with_quotes(
                full_text,
                content_to_source,
                source_to_content,
                quote_depth,
            );
        mappings.push(SourceTargetMapping {
            entity: block.clone(),
            full_source_range: absolute_start..absolute_start + full_text.len(),
            content_to_source,
            source_to_content,
        });
        full_text.len()
    }

    pub(super) fn push_code_block_mapping(
        &self,
        block: &Entity<Block>,
        quote_depth: usize,
        absolute_start: usize,
        mappings: &mut Vec<SourceTargetMapping>,
        cx: &App,
    ) -> usize {
        let (language, indentation, content) = {
            let block_ref = block.read(cx);
            (
                match block_ref.kind() {
                    BlockKind::CodeBlock { language } => language.clone(),
                    _ => None,
                },
                "  ".repeat(block_ref.render_depth),
                block_ref.display_text().to_string(),
            )
        };

        let (full_text, content_to_source, source_to_content) =
            Self::build_code_block_content_mapping(&content, &indentation, language.as_ref());
        let (full_text, content_to_source, source_to_content) =
            Self::wrap_source_mapping_with_quotes(
                full_text,
                content_to_source,
                source_to_content,
                quote_depth,
            );
        mappings.push(SourceTargetMapping {
            entity: block.clone(),
            full_source_range: absolute_start..absolute_start + full_text.len(),
            content_to_source,
            source_to_content,
        });
        full_text.len()
    }

    pub(super) fn wrap_source_mapping_with_quotes(
        mut full_text: String,
        mut content_to_source: Vec<usize>,
        mut source_to_content: Vec<usize>,
        quote_depth: usize,
    ) -> (String, Vec<usize>, Vec<usize>) {
        for _ in 0..quote_depth {
            let (wrapped_text, inner_to_wrapped, wrapped_to_inner) =
                Self::build_prefixed_content_mapping(&full_text, "> ", "> ");
            let max_inner_to_wrapped = inner_to_wrapped.len().saturating_sub(1);
            let max_source_to_content = source_to_content.len().saturating_sub(1);

            let wrapped_content_to_source = content_to_source
                .iter()
                .map(|offset| inner_to_wrapped[(*offset).min(max_inner_to_wrapped)])
                .collect::<Vec<_>>();
            let wrapped_source_to_content = wrapped_to_inner
                .iter()
                .map(|offset| source_to_content[(*offset).min(max_source_to_content)])
                .collect::<Vec<_>>();

            full_text = wrapped_text;
            content_to_source = wrapped_content_to_source;
            source_to_content = wrapped_source_to_content;
        }

        (full_text, content_to_source, source_to_content)
    }

    pub(super) fn push_table_mappings(
        &self,
        block: &Entity<Block>,
        list_depth: usize,
        quote_depth: usize,
        absolute_start: usize,
        mappings: &mut Vec<SourceTargetMapping>,
        cx: &App,
    ) -> usize {
        let Some(table) = block.read(cx).record.table.clone() else {
            return 0;
        };
        let Some(runtime) = block.read(cx).table_runtime.clone() else {
            return 0;
        };

        let lines = crate::components::serialize_table_markdown_lines(&table);
        let indentation = "  ".repeat(list_depth);
        let quote_prefix = "> ".repeat(quote_depth);
        let line_prefix_len = indentation.len() + quote_prefix.len();
        let mut line_start = absolute_start;

        if let Some(header_line) = lines.first() {
            let mut line_cursor = line_prefix_len + 2usize;
            for (column, cell) in runtime.header.iter().enumerate() {
                let Some(tree) = table.header.get(column) else {
                    continue;
                };
                let cell_markdown = serialize_table_cell_markdown(tree);
                let start = line_start + line_cursor;
                let len = cell_markdown.len();
                mappings.push(SourceTargetMapping {
                    entity: cell.clone(),
                    full_source_range: start..start + len,
                    content_to_source: (0..=len).collect(),
                    source_to_content: (0..=len).collect(),
                });
                line_cursor += len + 3;
            }
            line_start += line_prefix_len + header_line.len() + 1;
        }

        if lines.len() > 1 {
            line_start += line_prefix_len + lines[1].len() + 1;
        }

        for (body_row_index, row) in runtime.rows.iter().enumerate() {
            let Some(row_line) = lines.get(body_row_index + 2) else {
                break;
            };
            let mut line_cursor = line_prefix_len + 2usize;
            for (column, cell) in row.iter().enumerate() {
                let Some(tree) = table
                    .rows
                    .get(body_row_index)
                    .and_then(|table_row| table_row.get(column))
                else {
                    continue;
                };
                let cell_markdown = serialize_table_cell_markdown(tree);
                let start = line_start + line_cursor;
                let len = cell_markdown.len();
                mappings.push(SourceTargetMapping {
                    entity: cell.clone(),
                    full_source_range: start..start + len,
                    content_to_source: (0..=len).collect(),
                    source_to_content: (0..=len).collect(),
                });
                line_cursor += len + 3;
            }
            line_start += line_prefix_len + row_line.len() + 1;
        }

        lines
            .iter()
            .map(|line| line_prefix_len + line.len())
            .sum::<usize>()
            + lines.len().saturating_sub(1)
    }

    pub(super) fn collect_single_block_source_mappings(
        &self,
        block: &Entity<Block>,
        list_depth: usize,
        quote_depth: usize,
        absolute_start: usize,
        mappings: &mut Vec<SourceTargetMapping>,
        block_ranges: &mut HashMap<EntityId, Range<usize>>,
        cx: &App,
    ) -> usize {
        let (kind, list_ordinal, title, children) = {
            let block_ref = block.read(cx);
            let kind = block_ref.kind();
            let title = (!matches!(
                kind,
                BlockKind::Table
                    | BlockKind::CodeBlock { .. }
                    | BlockKind::Comment
                    | BlockKind::HtmlBlock
                    | BlockKind::MathBlock
                    | BlockKind::MermaidBlock
                    | BlockKind::RawMarkdown
                    | BlockKind::Separator
            ))
            .then(|| block_ref.record.title.markdown_offset_map());
            (
                kind,
                block_ref.list_ordinal,
                title,
                block_ref.children.clone(),
            )
        };

        let own_len = match kind {
            BlockKind::Table => self.push_table_mappings(
                block,
                list_depth,
                quote_depth,
                absolute_start,
                mappings,
                cx,
            ),
            BlockKind::CodeBlock { .. } => {
                self.push_code_block_mapping(block, quote_depth, absolute_start, mappings, cx)
            }
            BlockKind::RawMarkdown
            | BlockKind::Comment
            | BlockKind::HtmlBlock
            | BlockKind::MathBlock
            | BlockKind::MermaidBlock => {
                self.push_raw_block_mapping(block, quote_depth, absolute_start, mappings, cx)
            }
            BlockKind::Separator => {
                let line = block
                    .read(cx)
                    .record
                    .markdown_line(list_depth, list_ordinal);
                if quote_depth == 0 {
                    line.len()
                } else {
                    Self::wrap_source_mapping_with_quotes(
                        line.clone(),
                        (0..=line.len()).collect(),
                        (0..=line.len()).collect(),
                        quote_depth,
                    )
                    .0
                    .len()
                }
            }
            BlockKind::Heading { level } => self.push_inline_block_mapping(
                block,
                title.expect("heading title").markdown().to_string(),
                format!("{}{} ", "  ".repeat(list_depth), "#".repeat(level as usize)),
                String::new(),
                quote_depth,
                absolute_start,
                mappings,
            ),
            BlockKind::Paragraph => {
                let indentation = "  ".repeat(list_depth);
                self.push_inline_block_mapping(
                    block,
                    title.expect("paragraph title").markdown().to_string(),
                    indentation.clone(),
                    indentation,
                    quote_depth,
                    absolute_start,
                    mappings,
                )
            }
            BlockKind::BulletedListItem => {
                let indentation = "  ".repeat(list_depth);
                self.push_inline_block_mapping(
                    block,
                    title.expect("bullet title").markdown().to_string(),
                    format!("{indentation}- "),
                    format!("{indentation}  "),
                    quote_depth,
                    absolute_start,
                    mappings,
                )
            }
            BlockKind::TaskListItem { checked } => {
                let indentation = "  ".repeat(list_depth);
                self.push_inline_block_mapping(
                    block,
                    title.expect("task title").markdown().to_string(),
                    format!("{indentation}- [{}] ", if checked { "x" } else { " " }),
                    format!("{indentation}      "),
                    quote_depth,
                    absolute_start,
                    mappings,
                )
            }
            BlockKind::NumberedListItem => {
                let indentation = "  ".repeat(list_depth);
                let ordinal = list_ordinal.unwrap_or(1);
                self.push_inline_block_mapping(
                    block,
                    title.expect("numbered title").markdown().to_string(),
                    format!("{indentation}{ordinal}. "),
                    format!("{indentation}   "),
                    quote_depth,
                    absolute_start,
                    mappings,
                )
            }
            BlockKind::Quote => {
                let title = title.expect("quote title").markdown().to_string();
                if title.is_empty() && !children.is_empty() {
                    0
                } else {
                    self.push_inline_block_mapping(
                        block,
                        title,
                        String::new(),
                        String::new(),
                        quote_depth + 1,
                        absolute_start,
                        mappings,
                    )
                }
            }
            BlockKind::Callout(variant) => {
                let title_markdown = title.expect("callout title").markdown().to_string();
                if title_markdown.is_empty() {
                    let full_text = Self::wrap_source_mapping_with_quotes(
                        format!("[!{}]", variant.marker()),
                        vec![0],
                        vec![0; format!("[!{}]", variant.marker()).len() + 1],
                        quote_depth + 1,
                    )
                    .0;
                    mappings.push(SourceTargetMapping {
                        entity: block.clone(),
                        full_source_range: absolute_start..absolute_start + full_text.len(),
                        content_to_source: vec![full_text.len()],
                        source_to_content: vec![0; full_text.len() + 1],
                    });
                    full_text.len()
                } else {
                    self.push_inline_block_mapping(
                        block,
                        title_markdown,
                        format!("[!{}] ", variant.marker()),
                        String::new(),
                        quote_depth + 1,
                        absolute_start,
                        mappings,
                    )
                }
            }
            BlockKind::FootnoteDefinition => {
                let footnote_id = title.expect("footnote id").markdown().to_string();
                let first_child = children.first().cloned();
                let first_is_paragraph = first_child
                    .as_ref()
                    .is_some_and(|child| child.read(cx).kind() == BlockKind::Paragraph);
                Self::push_footnote_definition_head_mapping(
                    block,
                    &footnote_id,
                    first_is_paragraph,
                    quote_depth,
                    absolute_start,
                    mappings,
                )
            }
        };

        if kind == BlockKind::FootnoteDefinition {
            let mut total_len = own_len;
            let mut child_index = 0usize;
            if let Some(first_child) = children.first()
                && first_child.read(cx).kind() == BlockKind::Paragraph
            {
                total_len = self.push_inline_block_mapping(
                    first_child,
                    first_child
                        .read(cx)
                        .record
                        .title
                        .markdown_offset_map()
                        .markdown()
                        .to_string(),
                    block
                        .read(cx)
                        .footnote_definition_id()
                        .map(|id| format!("[^{id}]: "))
                        .unwrap_or_else(|| "[^]: ".to_string()),
                    "    ".to_string(),
                    quote_depth,
                    absolute_start,
                    mappings,
                );
                child_index = 1;
            }

            let mut previous_kind = if child_index > 0 {
                Some(BlockKind::Paragraph)
            } else {
                None
            };
            for child in children.iter().skip(child_index) {
                let current_kind = child.read(cx).kind();
                if total_len > 0 {
                    total_len += if previous_kind.is_none() {
                        1
                    } else if previous_kind.as_ref().is_some_and(|previous| {
                        previous.is_list_item() && current_kind.is_list_item()
                    }) {
                        1
                    } else {
                        2
                    };
                }
                total_len += self.collect_single_block_source_mappings(
                    child,
                    2,
                    quote_depth,
                    absolute_start + total_len,
                    mappings,
                    block_ranges,
                    cx,
                );
                previous_kind = Some(current_kind);
            }
            block_ranges.insert(
                block.entity_id(),
                absolute_start..absolute_start + total_len,
            );
            return total_len;
        }

        let child_list_depth = list_depth + usize::from(kind.is_list_item());
        let child_quote_depth = quote_depth + usize::from(kind.is_quote_container());
        let mut total_len = own_len;
        for child in children {
            if total_len > 0 {
                total_len += 1;
            }
            total_len += self.collect_single_block_source_mappings(
                &child,
                child_list_depth,
                child_quote_depth,
                absolute_start + total_len,
                mappings,
                block_ranges,
                cx,
            );
        }

        block_ranges.insert(
            block.entity_id(),
            absolute_start..absolute_start + total_len,
        );
        total_len
    }

    pub(super) fn build_source_target_mappings(&self, cx: &App) -> Vec<SourceTargetMapping> {
        self.build_source_target_mappings_with_block_ranges(cx).0
    }

    /// Like [`Self::build_source_target_mappings`], but also returns the source
    /// span of every block keyed by entity id. Atomic blocks (e.g. tables) have
    /// no per-block text mapping, so this is the only way to recover their full
    /// source extent for selection/deletion.
    pub(super) fn build_source_target_mappings_with_block_ranges(
        &self,
        cx: &App,
    ) -> (Vec<SourceTargetMapping>, HashMap<EntityId, Range<usize>>) {
        let mut mappings = Vec::new();
        let mut block_ranges = HashMap::new();
        let mut absolute = 0usize;
        let mut pending_empty_roots = 0usize;
        let mut wrote_non_empty_root = false;
        let mut previous_was_list_item = false;

        for block in self.document.root_blocks() {
            let (is_empty_root, current_is_list_item) = {
                let block_ref = block.read(cx);
                (
                    Self::is_empty_root_paragraph(block_ref),
                    block_ref.kind().is_list_item(),
                )
            };
            if is_empty_root {
                // Empty roots carry no text mapping, but they still need a source
                // span so a cross-block selection whose boundary lands on one can
                // be resolved (otherwise deletion of the selection aborts). A
                // zero-width anchor at the current cursor is the right position:
                // 0 for a leading empty root, source end for a trailing one.
                block_ranges.insert(block.entity_id(), absolute..absolute);
                pending_empty_roots += 1;
                continue;
            }

            if wrote_non_empty_root {
                let separator_count = if previous_was_list_item && current_is_list_item {
                    if pending_empty_roots == 0 {
                        0
                    } else {
                        pending_empty_roots + 1
                    }
                } else {
                    pending_empty_roots + 1
                };
                absolute += separator_count;
            } else if pending_empty_roots > 0 {
                absolute += pending_empty_roots;
            }

            absolute += self.collect_single_block_source_mappings(
                block,
                0,
                0,
                absolute,
                &mut mappings,
                &mut block_ranges,
                cx,
            );

            wrote_non_empty_root = true;
            pending_empty_roots = 0;
            previous_was_list_item = current_is_list_item;
            absolute += 1;
        }

        (mappings, block_ranges)
    }
}
