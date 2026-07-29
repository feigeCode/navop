//! Block semantic state and block-level Markdown parsing helpers.
//!
//! This module defines the persistent block record that is serialized to and
//! from Markdown. Block-level parsing stays intentionally narrow: only syntax
//! that the runtime tree can reconstruct is parsed into structured blocks.

use std::ops::Range;
use std::path::PathBuf;

use gpui::{Image, Pixels, Point, SharedString};
use uuid::Uuid;

use crate::components::markdown::html::{HtmlDocument, parse_html_document};
use crate::components::markdown::image::parse_standalone_image;
use crate::components::markdown::inline::InlineTextTree;
use crate::components::{TableAxisKind, TableColumnAlignment, TableData};

/// Supported callout variants parsed from `[!TYPE]` quote headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalloutVariant {
    /// Informational note callout.
    Note,
    /// Helpful tip callout.
    Tip,
    /// High-emphasis important callout.
    Important,
    /// Warning callout for risky or surprising content.
    Warning,
    /// Caution callout for potentially harmful actions.
    Caution,
}

impl CalloutVariant {
    pub fn marker(self) -> &'static str {
        match self {
            Self::Note => "NOTE",
            Self::Tip => "TIP",
            Self::Important => "IMPORTANT",
            Self::Warning => "WARNING",
            Self::Caution => "CAUTION",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Note => "Note",
            Self::Tip => "Tip",
            Self::Important => "Important",
            Self::Warning => "Warning",
            Self::Caution => "Caution",
        }
    }

    pub fn parse_header_line(line: &str) -> Option<(Self, String)> {
        let trimmed = line.trim_start();
        let rest = trimmed.strip_prefix("[!")?;
        let marker_end = rest.find(']')?;
        let marker = &rest[..marker_end];
        let variant = match marker.to_ascii_uppercase().as_str() {
            "NOTE" => Self::Note,
            "TIP" => Self::Tip,
            "IMPORTANT" => Self::Important,
            "WARNING" => Self::Warning,
            "CAUTION" => Self::Caution,
            _ => return None,
        };
        let title = rest[marker_end + 1..].trim_start().to_string();
        Some((variant, title))
    }

    pub fn header_markdown(self, title_markdown: &str) -> String {
        if title_markdown.trim().is_empty() {
            format!("[!{}]", self.marker())
        } else {
            format!("[!{}] {}", self.marker(), title_markdown)
        }
    }

    pub fn escape_plain_quote_header(title_markdown: &str) -> String {
        let mut lines = title_markdown.splitn(2, '\n');
        let first = lines.next().unwrap_or_default();
        let rest = lines.next();
        let escaped_first = if Self::parse_header_line(first).is_some() {
            format!("\\{first}")
        } else {
            first.to_string()
        };
        match rest {
            Some(rest) => format!("{escaped_first}\n{rest}"),
            None => escaped_first,
        }
    }
}

/// The semantic type of a block, determining both its Markdown syntax and
/// visual rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    /// Plain paragraph with inline formatting.
    Paragraph,
    /// Horizontal rule.
    Separator,
    /// ATX or Setext heading with a CommonMark heading level.
    Heading { level: u8 },
    /// Unordered list item.
    BulletedListItem,
    /// Task-list item with checked state.
    TaskListItem { checked: bool },
    /// Ordered list item; serialization uses canonical dot markers.
    NumberedListItem,
    /// Blockquote container.
    Quote,
    /// GitHub-style alert/callout container.
    Callout(CalloutVariant),
    /// Footnote definition container.
    FootnoteDefinition,
    /// Native pipe-table block.
    Table,
    /// Fenced code block with optional language info string.
    CodeBlock { language: Option<SharedString> },
    /// Visible HTML comment block preserved as raw comment text.
    Comment,
    /// Safe raw HTML rendered through native GPUI semantic elements.
    HtmlBlock,
    /// Display math block rendered with the LaTeX pipeline.
    MathBlock,
    /// Mermaid fenced block rendered as SVG.
    MermaidBlock,
    /// Raw Markdown fallback for syntax outside the native runtime subset.
    RawMarkdown,
}

/// Opening fence parsed from a fenced code block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeFenceOpening {
    /// Fence character, either backtick or tilde.
    pub ch: char,
    /// Length of the opening fence run.
    pub len: usize,
    /// Optional language/info string after the opening fence.
    pub language: Option<SharedString>,
}

impl BlockKind {
    /// Returns true when blocks of this kind may own child blocks in the
    /// current runtime tree.
    pub fn supports_children(&self) -> bool {
        self.is_list_item() || self.is_quote_container() || self.is_footnote_definition()
    }

    pub fn is_list_item(&self) -> bool {
        matches!(
            self,
            Self::BulletedListItem | Self::TaskListItem { .. } | Self::NumberedListItem
        )
    }

    pub fn is_numbered_list_item(&self) -> bool {
        matches!(self, Self::NumberedListItem)
    }

    pub fn is_task_list_item(&self) -> bool {
        matches!(self, Self::TaskListItem { .. })
    }

    pub fn is_code_block(&self) -> bool {
        matches!(self, Self::CodeBlock { .. })
    }

    /// Whether the right-click "Insert Table" affordance makes sense when a
    /// block of this kind is the target. Atomic/structural blocks (tables,
    /// code, math, etc.) render as self-contained widgets where inserting a
    /// table from within them is nonsensical, so they are excluded.
    pub fn allows_context_table_insert(&self) -> bool {
        !matches!(
            self,
            Self::Table
                | Self::CodeBlock { .. }
                | Self::MathBlock
                | Self::MermaidBlock
                | Self::HtmlBlock
                | Self::Comment
                | Self::RawMarkdown
        )
    }

    pub fn is_quote_container(&self) -> bool {
        matches!(self, Self::Quote | Self::Callout(_))
    }

    /// Blocks that render as self-contained widgets with no caret position
    /// after them. At the end of a rendered document they need a trailing
    /// paragraph so a rendered-first user can keep typing past the structure
    /// instead of having to drop to source mode.
    pub fn is_atomic_structural(&self) -> bool {
        matches!(
            self,
            Self::Separator
                | Self::Table
                | Self::CodeBlock { .. }
                | Self::MathBlock
                | Self::MermaidBlock
                | Self::HtmlBlock
                | Self::Comment
                | Self::RawMarkdown
        )
    }

    pub fn is_callout(&self) -> bool {
        matches!(self, Self::Callout(_))
    }

    /// Blocks edited as multi-line raw text that render as self-contained
    /// widgets (code, math, HTML, mermaid, comment, raw markdown). Exiting one
    /// downward with `Down` or `Ctrl/Cmd+Enter` needs a line below to land on.
    pub fn is_multiline_text_block(&self) -> bool {
        self.is_code_block()
            || matches!(
                self,
                Self::MathBlock
                    | Self::HtmlBlock
                    | Self::MermaidBlock
                    | Self::Comment
                    | Self::RawMarkdown
            )
    }

    pub fn is_footnote_definition(&self) -> bool {
        matches!(self, Self::FootnoteDefinition)
    }

    pub fn callout_variant(&self) -> Option<CalloutVariant> {
        match self {
            Self::Callout(variant) => Some(*variant),
            _ => None,
        }
    }

    pub fn is_separator(&self) -> bool {
        matches!(self, Self::Separator)
    }

    pub fn can_nest_under(&self, parent: &Self) -> bool {
        if !parent.is_list_item() {
            return false;
        }

        self.is_list_item()
            || matches!(
                self,
                Self::Paragraph
                    | Self::Quote
                    | Self::Callout(_)
                    | Self::FootnoteDefinition
                    | Self::Table
                    | Self::CodeBlock { .. }
                    | Self::Comment
                    | Self::HtmlBlock
                    | Self::MathBlock
                    | Self::MermaidBlock
                    | Self::RawMarkdown
            )
    }

    pub fn newline_sibling_kind(&self) -> Self {
        if matches!(self, Self::TaskListItem { .. }) {
            Self::TaskListItem { checked: false }
        } else if self.is_list_item() {
            self.clone()
        } else if self.is_quote_container() {
            self.clone()
        } else if self.is_footnote_definition() {
            Self::Paragraph
        } else if self.is_code_block() || self.is_separator() {
            Self::Paragraph
        } else {
            Self::Paragraph
        }
    }

    /// Live-detects a Markdown prefix from user input and returns the
    /// corresponding [`BlockKind`] together with the character count of
    /// the prefix that should be stripped.
    pub fn detect_markdown_shortcut(value: &str) -> Option<(Self, usize)> {
        if value.starts_with("###### ") {
            Some((Self::Heading { level: 6 }, 7))
        } else if value.starts_with("##### ") {
            Some((Self::Heading { level: 5 }, 6))
        } else if value.starts_with("#### ") {
            Some((Self::Heading { level: 4 }, 5))
        } else if value.starts_with("### ") {
            Some((Self::Heading { level: 3 }, 4))
        } else if value.starts_with("## ") {
            Some((Self::Heading { level: 2 }, 3))
        } else if value.starts_with("# ") {
            Some((Self::Heading { level: 1 }, 2))
        } else if let Some((checked, prefix_len)) = Self::parse_task_list_shortcut(value) {
            Some((Self::TaskListItem { checked }, prefix_len))
        } else if value.starts_with("* ") || value.starts_with("+ ") {
            Some((Self::BulletedListItem, 2))
        } else if value.starts_with("- ") {
            Some((Self::BulletedListItem, 2))
        } else if let Some(prefix_len) = Self::numbered_list_shortcut_prefix_len(value) {
            Some((Self::NumberedListItem, prefix_len))
        } else if value.starts_with("> ") {
            Some((Self::Quote, 2))
        } else {
            None
        }
    }

    pub fn parse_atx_heading_line(line: &str) -> Option<(u8, String)> {
        let trimmed_end = line.trim_end();
        let leading_spaces = trimmed_end.bytes().take_while(|b| *b == b' ').count();
        if leading_spaces > 3 {
            return None;
        }

        let rest = &trimmed_end[leading_spaces..];
        let level = rest.bytes().take_while(|b| *b == b'#').count();
        if !(1..=6).contains(&level) {
            return None;
        }

        let content = rest[level..].strip_prefix(' ')?;
        let mut content = content.trim_end().to_string();
        if let Some(closing_hash_start) = content.rfind(' ')
            && content[closing_hash_start + 1..]
                .chars()
                .all(|ch| ch == '#')
        {
            content.truncate(closing_hash_start);
            content = content.trim_end().to_string();
        }

        Some((level as u8, content))
    }

    pub fn parse_setext_underline(line: &str) -> Option<u8> {
        let trimmed_end = line.trim_end();
        let leading_spaces = trimmed_end.bytes().take_while(|b| *b == b' ').count();
        if leading_spaces > 3 {
            return None;
        }

        let rest = &trimmed_end[leading_spaces..];
        if rest.len() < 3 {
            return None;
        }

        if rest.bytes().all(|b| b == b'=') {
            Some(1)
        } else if rest.bytes().all(|b| b == b'-') {
            Some(2)
        } else {
            None
        }
    }

    pub fn parse_code_fence_opening(value: &str) -> Option<CodeFenceOpening> {
        let trimmed = value.trim_end();
        let ch = trimmed.chars().next()?;
        if ch != '`' && ch != '~' {
            return None;
        }

        let len = trimmed.chars().take_while(|&c| c == ch).count();
        if len < 3 {
            return None;
        }

        let rest = &trimmed[ch.len_utf8() * len..];
        if ch == '`' && rest.contains('`') {
            return None;
        }

        let language = rest.trim();
        Some(CodeFenceOpening {
            ch,
            len,
            language: if language.is_empty() {
                None
            } else {
                Some(language.to_string().into())
            },
        })
    }

    pub fn parse_separator_line(value: &str) -> bool {
        let trimmed_end = value.trim_end();
        let leading_spaces = trimmed_end.bytes().take_while(|b| *b == b' ').count();
        if leading_spaces > 3 {
            return false;
        }

        let rest = &trimmed_end[leading_spaces..];
        let mut marker = None;
        let mut marker_count = 0usize;
        for ch in rest.chars() {
            if ch == ' ' {
                continue;
            }
            if !matches!(ch, '-' | '*' | '_') {
                return false;
            }
            if let Some(existing) = marker {
                if existing != ch {
                    return false;
                }
            } else {
                marker = Some(ch);
            }
            marker_count += 1;
        }

        marker_count >= 3
    }

    /// Parses a task-list marker at the start of list-item content.
    ///
    /// Accepted forms are `[ ]`, `[x]`, and `[X]`, optionally followed by a
    /// space or tab before the item text. An empty title is also valid.
    pub fn parse_task_list_item_prefix(value: &str) -> Option<(bool, usize)> {
        let bytes = value.as_bytes();
        if bytes.len() < 3 || bytes[0] != b'[' || bytes[2] != b']' {
            return None;
        }

        let checked = match bytes[1] {
            b' ' => false,
            b'x' | b'X' => true,
            _ => return None,
        };

        if bytes.len() == 3 {
            return Some((checked, 3));
        }

        if matches!(bytes[3], b' ' | b'\t') {
            Some((checked, 4))
        } else {
            None
        }
    }

    fn parse_task_list_shortcut(value: &str) -> Option<(bool, usize)> {
        let rest = value.strip_prefix("- ")?;
        let (checked, prefix_len) = Self::parse_task_list_item_prefix(rest)?;
        Some((checked, 2 + prefix_len))
    }

    fn numbered_list_shortcut_prefix_len(value: &str) -> Option<usize> {
        let digit_len = value.bytes().take_while(|b| b.is_ascii_digit()).count();
        if !(1..=9).contains(&digit_len) {
            return None;
        }

        let marker = *value.as_bytes().get(digit_len)?;
        if !matches!(marker, b'.' | b')') {
            return None;
        }

        let separator = *value.as_bytes().get(digit_len + 1)?;
        matches!(separator, b' ' | b'\t').then_some(digit_len + 2)
    }
}

/// Persistent data of a block independent of the editor runtime.
///
/// Holds the block's identity, kind, inline-formatted title, and tree
/// references (parent/children via UUID). Raw-preserved Markdown keeps its
/// original source in `raw_fallback` so it round-trips through save/load
/// losslessly.
#[derive(Debug, Clone)]
pub struct BlockRecord {
    pub id: Uuid,
    pub kind: BlockKind,
    pub title: InlineTextTree,
    pub table: Option<TableData>,
    pub(crate) html: Option<HtmlDocument>,
    pub parent: Option<Uuid>,
    pub content: Vec<Uuid>,
    pub raw_fallback: Option<String>,
}

impl BlockRecord {
    pub fn new(kind: BlockKind, title: InlineTextTree) -> Self {
        let mut record = Self {
            id: Uuid::new_v4(),
            kind,
            title,
            table: None,
            html: None,
            parent: None,
            content: Vec::new(),
            raw_fallback: None,
        };
        record.sync_raw_fallback();
        record
    }

    pub fn with_plain_text(kind: BlockKind, text: impl Into<String>) -> Self {
        Self::new(kind, InlineTextTree::plain(text.into()))
    }

    pub fn paragraph(text: impl Into<String>) -> Self {
        Self::with_plain_text(BlockKind::Paragraph, text)
    }

    pub fn raw_markdown(markdown: impl Into<String>) -> Self {
        let markdown = markdown.into();
        let mut record = Self::with_plain_text(BlockKind::RawMarkdown, markdown.clone());
        record.raw_fallback = Some(markdown);
        record
    }

    pub fn comment(markdown: impl Into<String>) -> Self {
        let markdown = markdown.into();
        let mut record = Self::with_plain_text(BlockKind::Comment, markdown.clone());
        record.raw_fallback = Some(markdown);
        record
    }

    pub fn html(markdown: impl Into<String>) -> Self {
        let markdown = markdown.into();
        let html = parse_html_document(&markdown);
        let mut record = Self::with_plain_text(BlockKind::HtmlBlock, markdown.clone());
        record.html = Some(html);
        record.raw_fallback = Some(markdown);
        record
    }

    pub fn math(markdown: impl Into<String>) -> Self {
        let markdown = markdown.into();
        let mut record = Self::with_plain_text(BlockKind::MathBlock, markdown.clone());
        record.raw_fallback = Some(markdown);
        record
    }

    pub fn mermaid(markdown: impl Into<String>) -> Self {
        let markdown = markdown.into();
        let mut record = Self::with_plain_text(BlockKind::MermaidBlock, markdown.clone());
        record.raw_fallback = Some(markdown);
        record
    }

    pub fn table(table: TableData) -> Self {
        let mut record = Self::new(BlockKind::Table, InlineTextTree::plain(String::new()));
        record.table = Some(table);
        record
    }

    pub fn set_title(&mut self, title: InlineTextTree) {
        self.title = title;
        self.sync_raw_fallback();
    }

    /// Export the block title as Markdown: fragment style flags are
    /// serialized back to delimiter markers via [`InlineTextTree::serialize_markdown`].
    pub fn title_markdown(&self) -> String {
        self.title.serialize_markdown()
    }

    /// Returns true for block kinds that keep their original source text
    /// in `raw_fallback` because they are preserved as opaque Markdown.
    pub fn kind_uses_raw_fallback(&self) -> bool {
        matches!(
            self.kind,
            BlockKind::RawMarkdown
                | BlockKind::Comment
                | BlockKind::HtmlBlock
                | BlockKind::MathBlock
                | BlockKind::MermaidBlock
        )
    }

    /// Serialize this block back to a single Markdown line, including
    /// indentation for nested blocks and list ordinal for numbered items.
    /// Raw-preserved blocks produce their fallback text when at depth 0.
    pub fn markdown_line(&self, depth: usize, list_ordinal: Option<usize>) -> String {
        let indentation = "  ".repeat(depth);
        let title_markdown = self.title_markdown_for_output();
        match self.kind {
            BlockKind::Paragraph => indent_multiline(&title_markdown, &indentation),
            BlockKind::Separator => "---".to_string(),
            BlockKind::Heading { level } => {
                format!(
                    "{indentation}{} {title_markdown}",
                    "#".repeat(level as usize)
                )
            }
            BlockKind::BulletedListItem => prefixed_multiline(
                &title_markdown,
                &format!("{indentation}- "),
                &format!("{indentation}  "),
            ),
            BlockKind::TaskListItem { checked } => prefixed_multiline(
                &title_markdown,
                &format!("{indentation}- [{}] ", if checked { "x" } else { " " }),
                &format!("{indentation}      "),
            ),
            BlockKind::NumberedListItem => {
                let ordinal = list_ordinal.unwrap_or(1);
                prefixed_multiline(
                    &title_markdown,
                    &format!("{indentation}{ordinal}. "),
                    &format!("{indentation}   "),
                )
            }
            BlockKind::Quote => prefixed_multiline(
                &CalloutVariant::escape_plain_quote_header(&title_markdown),
                &format!("{indentation}> "),
                &format!("{indentation}> "),
            ),
            BlockKind::Callout(variant) => format!(
                "{indentation}> {}",
                variant.header_markdown(&title_markdown)
            ),
            BlockKind::FootnoteDefinition => {
                format!("{indentation}[^{}]: ", self.title.visible_text())
            }
            BlockKind::Table => String::new(),
            BlockKind::CodeBlock { .. } => title_markdown,
            BlockKind::RawMarkdown
            | BlockKind::Comment
            | BlockKind::HtmlBlock
            | BlockKind::MathBlock
            | BlockKind::MermaidBlock => {
                if depth == 0 {
                    self.raw_fallback.clone().unwrap_or(title_markdown)
                } else {
                    indent_multiline(
                        &self.raw_fallback.clone().unwrap_or(title_markdown),
                        &indentation,
                    )
                }
            }
        }
    }

    fn title_markdown_for_output(&self) -> String {
        let visible = self.title.visible_text();
        if self.can_present_title_as_standalone_image()
            && parse_standalone_image(&visible).is_some()
        {
            return visible;
        }

        self.title_markdown()
    }

    fn can_present_title_as_standalone_image(&self) -> bool {
        matches!(
            self.kind,
            BlockKind::Paragraph
                | BlockKind::BulletedListItem
                | BlockKind::NumberedListItem
                | BlockKind::TaskListItem { .. }
        )
    }

    fn sync_raw_fallback(&mut self) {
        if self.kind_uses_raw_fallback() {
            self.raw_fallback = Some(self.title.visible_text().to_string());
            if self.kind == BlockKind::HtmlBlock {
                self.html = self
                    .raw_fallback
                    .as_ref()
                    .map(|raw| parse_html_document(raw));
            }
        } else {
            self.raw_fallback = None;
            self.html = None;
        }
    }
}

fn indent_multiline(content: &str, indentation: &str) -> String {
    content
        .split('\n')
        .map(|line| format!("{indentation}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn prefixed_multiline(content: &str, first_prefix: &str, continuation_prefix: &str) -> String {
    let mut lines = content.split('\n');
    let mut rendered = String::new();
    if let Some(first) = lines.next() {
        rendered.push_str(first_prefix);
        rendered.push_str(first);
    }

    for line in lines {
        rendered.push('\n');
        rendered.push_str(continuation_prefix);
        rendered.push_str(line);
    }

    rendered
}

/// Image payload extracted from GPUI's clipboard abstraction.
///
/// File-manager copies are usually represented as local paths, while bitmap
/// copies from image editors or browsers arrive as encoded image bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PastedImageSource {
    ClipboardImage(Image),
    LocalPath(PathBuf),
}

/// Events emitted by a block to its parent editor when structural
/// changes or focus transfers are needed that the block cannot handle alone.
///
/// The Editor subscribes to these events on every block via
/// `cx.subscribe(&block, Self::on_block_event)`.
#[derive(Debug, Clone)]
pub enum BlockEvent {
    /// Capture the current document state before an upcoming mutation.
    PrepareUndo { kind: UndoCaptureKind },
    /// The block's content or kind changed; the editor should mark the
    /// document dirty and optionally scroll to keep the block visible.
    Changed,
    /// The user pressed Enter; a new block should be created after this
    /// one with the given trailing text.
    RequestNewline {
        trailing: InlineTextTree,
        source_already_mutated: bool,
    },
    /// The user pressed Enter on a callout header; the editor should ensure
    /// the callout owns a body entry and move focus into it.
    RequestEnterCalloutBody,
    /// The user requested a quote-group break at the current quote depth.
    /// The editor should insert a new empty quote group at the current depth,
    /// with whatever separator structure is required by Markdown at that level.
    RequestQuoteBreak,
    /// The user requested to exit the current callout into a plain text block.
    /// The editor should insert the separator structure needed to end the
    /// surrounding quote group, then focus a plain paragraph entry below it.
    RequestCalloutBreak,
    /// The user pressed Backspace at the start of this block; its entire
    /// content should be appended to the previous block.
    RequestMergeIntoPrev { content: InlineTextTree },
    /// A multi-line paste was detected; the editor must split the pasted
    /// lines into separate blocks and re-attach the leading/trailing text
    /// to the correct positions.
    RequestPasteMultiline {
        leading: InlineTextTree,
        lines: Vec<String>,
        trailing: InlineTextTree,
        split_physical_lines: bool,
    },
    /// An image-like clipboard payload was pasted. The editor resolves
    /// storage preferences and inserts either an image block or image text.
    RequestPasteImage {
        leading: InlineTextTree,
        source: PastedImageSource,
        trailing: InlineTextTree,
    },
    /// Replace the current editor-level cross-block selection with text
    /// submitted through the focused block input handler.
    RequestReplaceCrossBlockSelection {
        text: String,
        selected_range_relative: Option<Range<usize>>,
        mark_inserted_text: bool,
        undo_kind: UndoCaptureKind,
    },
    /// Ctrl/Cmd+A was pressed in rendered editing. The editor decides whether
    /// this press selects the focused block or upgrades to all rendered blocks.
    RequestRenderedSelectAll,
    /// Tab pressed in list context; increase the current block's nesting when
    /// the previous visible block can adopt it.
    RequestIndent,
    /// Shift-Tab pressed in list context; lift the current block out one level.
    RequestOutdent,
    /// Backspace on a nested list item should remove its marker first,
    /// degrading it into a direct list-child paragraph at the same depth.
    RequestDowngradeNestedListItemToChildParagraph,
    /// Toggle the checked state of a task-list item.
    ToggleTaskChecked,
    /// Prompt to open the clicked inline link destination.
    /// `prompt_target` preserves the raw syntax target shown to the user,
    /// while `open_target` is the resolved destination actually opened.
    RequestOpenLink {
        prompt_target: String,
        open_target: String,
    },
    /// Jump from a rendered footnote reference to the corresponding
    /// in-place footnote definition block.
    RequestJumpToFootnoteDefinition { id: String },
    /// Jump from an in-place footnote definition back to its first reference.
    RequestJumpToFootnoteBackref { id: String },
    /// Move focus horizontally across native table cells.
    RequestTableCellMoveHorizontal { delta: i32 },
    /// Move focus vertically across native table cells.
    RequestTableCellMoveVertical { delta: i32 },
    /// Append one empty column to a native table.
    RequestAppendTableColumn,
    /// Append one empty body row to a native table.
    RequestAppendTableRow,
    /// Insert an empty visual row before/after the active row.
    RequestInsertTableRow { visual_row: usize, after: bool },
    /// Insert an empty column before/after the active column.
    RequestInsertTableColumn { column: usize, after: bool },
    /// Delete the active visual row. A header-only table is removed when its
    /// header is deleted; deleting the final body row keeps that header.
    RequestDeleteTableRow { visual_row: usize },
    /// Delete the active column; deleting the final column removes the table.
    RequestDeleteTableColumn { column: usize },
    /// Change the active column's Markdown delimiter alignment.
    RequestAlignTableColumn {
        column: usize,
        alignment: TableColumnAlignment,
    },
    /// Resize a native table. `visual_rows` includes the header row.
    RequestResizeTable { visual_rows: usize, columns: usize },
    /// Delete the complete native table block.
    RequestDeleteTable,
    /// A native table axis handle was entered or left by the pointer.
    /// `hovered` distinguishes the two so the editor can ignore a leave
    /// that arrives after an adjacent handle has already taken the preview.
    RequestTableAxisPreview {
        kind: TableAxisKind,
        index: usize,
        hovered: bool,
    },
    /// Select one native table row or column for batch operations.
    RequestSelectTableAxis { kind: TableAxisKind, index: usize },
    /// Open the axis context menu for a native table row or column.
    RequestOpenTableAxisMenu {
        kind: TableAxisKind,
        index: usize,
        position: Point<Pixels>,
    },
    /// Cursor reached the top of this block; move focus to the previous
    /// visible block, preserving the preferred horizontal position.
    RequestFocusPrev { preferred_x: Option<f32> },
    /// Cursor reached the bottom of this block; move focus to the next
    /// visible block, preserving the preferred horizontal position.
    RequestFocusNext { preferred_x: Option<f32> },
    /// Move focus to the start of the previous visible block.
    RequestBlockUp,
    /// Move focus to the start of the next visible block.
    RequestBlockDown,
    /// This block should be deleted (empty and backspace/delete pressed).
    RequestDelete,
    /// The user clicked this block; notify siblings so they re-render
    /// in display mode.
    RequestFocus,
}

/// Undo coalescing category captured before a mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoCaptureKind {
    /// Text edits that may merge with adjacent typing within the coalescing window.
    CoalescibleText,
    /// Structural or discrete edits that always form their own undo entry.
    NonCoalescible,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_table_insert_excludes_atomic_blocks() {
        assert!(BlockKind::Paragraph.allows_context_table_insert());
        assert!(BlockKind::Heading { level: 1 }.allows_context_table_insert());
        assert!(BlockKind::Quote.allows_context_table_insert());
        assert!(!BlockKind::Table.allows_context_table_insert());
        assert!(!BlockKind::CodeBlock { language: None }.allows_context_table_insert());
        assert!(!BlockKind::MathBlock.allows_context_table_insert());
        assert!(!BlockKind::MermaidBlock.allows_context_table_insert());
    }

    #[test]
    fn detects_markdown_shortcuts() {
        assert_eq!(
            BlockKind::detect_markdown_shortcut("- item"),
            Some((BlockKind::BulletedListItem, 2))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("1. item"),
            Some((BlockKind::NumberedListItem, 3))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("12. item"),
            Some((BlockKind::NumberedListItem, 4))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("1) item"),
            Some((BlockKind::NumberedListItem, 3))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("12)\titem"),
            Some((BlockKind::NumberedListItem, 4))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("1234567890) item"),
            None
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("# heading"),
            Some((BlockKind::Heading { level: 1 }, 2))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("## heading"),
            Some((BlockKind::Heading { level: 2 }, 3))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("### heading"),
            Some((BlockKind::Heading { level: 3 }, 4))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("#### heading"),
            Some((BlockKind::Heading { level: 4 }, 5))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("##### heading"),
            Some((BlockKind::Heading { level: 5 }, 6))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("###### heading"),
            Some((BlockKind::Heading { level: 6 }, 7))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("- [ ] task"),
            Some((BlockKind::TaskListItem { checked: false }, 6))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("- [x] task"),
            Some((BlockKind::TaskListItem { checked: true }, 6))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("* item"),
            Some((BlockKind::BulletedListItem, 2))
        );
        assert_eq!(
            BlockKind::detect_markdown_shortcut("+ item"),
            Some((BlockKind::BulletedListItem, 2))
        );
        assert_eq!(BlockKind::detect_markdown_shortcut("#no-space"), None);
        assert_eq!(BlockKind::detect_markdown_shortcut("```"), None);
        assert_eq!(
            BlockKind::detect_markdown_shortcut("> quote"),
            Some((BlockKind::Quote, 2))
        );
        assert_eq!(BlockKind::detect_markdown_shortcut(">no-space"), None);
    }

    #[test]
    fn parses_separator_lines() {
        assert!(BlockKind::parse_separator_line("---"));
        assert!(BlockKind::parse_separator_line("----"));
        assert!(BlockKind::parse_separator_line("***"));
        assert!(BlockKind::parse_separator_line("_ _ _"));
        assert!(BlockKind::parse_separator_line(" - - - "));
        assert!(!BlockKind::parse_separator_line("--"));
        assert!(BlockKind::parse_separator_line(" ---"));
        assert!(!BlockKind::parse_separator_line("---x"));
    }

    #[test]
    fn parses_code_fence_openings() {
        assert_eq!(
            BlockKind::parse_code_fence_opening("```rust"),
            Some(CodeFenceOpening {
                ch: '`',
                len: 3,
                language: Some("rust".into()),
            })
        );
        assert_eq!(
            BlockKind::parse_code_fence_opening("~~~ts"),
            Some(CodeFenceOpening {
                ch: '~',
                len: 3,
                language: Some("ts".into()),
            })
        );
        assert_eq!(
            BlockKind::parse_code_fence_opening("```"),
            Some(CodeFenceOpening {
                ch: '`',
                len: 3,
                language: None,
            })
        );
        assert_eq!(BlockKind::parse_code_fence_opening("``"), None);
        assert_eq!(BlockKind::parse_code_fence_opening("```ru`st"), None);
    }

    #[test]
    fn parses_task_list_item_prefixes() {
        assert_eq!(
            BlockKind::parse_task_list_item_prefix("[ ] a"),
            Some((false, 4))
        );
        assert_eq!(
            BlockKind::parse_task_list_item_prefix("[x] a"),
            Some((true, 4))
        );
        assert_eq!(
            BlockKind::parse_task_list_item_prefix("[X] a"),
            Some((true, 4))
        );
        assert_eq!(BlockKind::parse_task_list_item_prefix("[a] a"), None);
    }

    #[test]
    fn serializes_supported_block_kinds() {
        let list = BlockRecord::new(
            BlockKind::BulletedListItem,
            InlineTextTree::from_markdown("*item*"),
        );
        let numbered = BlockRecord::new(
            BlockKind::NumberedListItem,
            InlineTextTree::from_markdown("step"),
        );
        let task = BlockRecord::new(
            BlockKind::TaskListItem { checked: true },
            InlineTextTree::from_markdown("done"),
        );
        let heading = BlockRecord::new(
            BlockKind::Heading { level: 2 },
            InlineTextTree::from_markdown("**title**"),
        );
        let quote = BlockRecord::new(BlockKind::Quote, InlineTextTree::plain("quoted text"));
        let paragraph = BlockRecord::paragraph("plain");
        let comment = BlockRecord::comment("<!--\ncomment\n-->");

        assert_eq!(list.markdown_line(0, None), "- *item*");
        assert_eq!(list.markdown_line(2, None), "    - *item*");
        assert_eq!(task.markdown_line(0, None), "- [x] done");
        assert_eq!(task.markdown_line(2, None), "    - [x] done");
        assert_eq!(numbered.markdown_line(0, Some(3)), "3. step");
        assert_eq!(numbered.markdown_line(2, Some(12)), "    12. step");
        assert_eq!(heading.markdown_line(0, None), "## **title**");
        assert_eq!(quote.markdown_line(0, None), "> quoted text");
        assert_eq!(quote.markdown_line(2, None), "    > quoted text");
        assert_eq!(paragraph.markdown_line(1, None), "  plain");
        assert_eq!(comment.markdown_line(0, None), "<!--\ncomment\n-->");
        assert_eq!(comment.markdown_line(1, None), "  <!--\n  comment\n  -->");
    }

    #[test]
    fn standalone_image_markdown_line_preserves_underscores() {
        let markdown = "![1.1_进制转换例子](./NetworkEngineerSummer.assets/1.1_进制转换例子.jpg)";
        let paragraph = BlockRecord::paragraph(markdown);

        assert_eq!(paragraph.markdown_line(0, None), markdown);
    }

    #[test]
    fn quote_serializes_back_to_markdown() {
        let record = BlockRecord::new(BlockKind::Quote, InlineTextTree::plain("text"));
        let line = record.markdown_line(0, None);
        assert_eq!(line, "> text");
    }

    #[test]
    fn parses_h2_and_h3_lines_with_correct_levels() {
        let h2 = BlockKind::parse_atx_heading_line("## hello");
        assert_eq!(h2, Some((2, "hello".to_string())));

        let h3 = BlockKind::parse_atx_heading_line("### hello");
        assert_eq!(h3, Some((3, "hello".to_string())));
    }

    #[test]
    fn parses_atx_headings_with_closing_hashes_and_setext_underlines() {
        let atx = BlockKind::parse_atx_heading_line("  ### title ######");
        assert_eq!(atx, Some((3, "title".to_string())));

        assert_eq!(BlockKind::parse_setext_underline("==="), Some(1));
        assert_eq!(BlockKind::parse_setext_underline("---"), Some(2));
        assert_eq!(BlockKind::parse_setext_underline("- - -"), None);
    }

    #[test]
    fn code_block_kind_stores_language() {
        let kind = BlockKind::CodeBlock {
            language: Some(SharedString::from("rust")),
        };
        assert!(kind.is_code_block());
        assert!(!kind.is_list_item());

        let no_lang = BlockKind::CodeBlock { language: None };
        assert!(no_lang.is_code_block());
    }

    #[test]
    fn code_block_markdown_line_returns_plain_content() {
        let record = BlockRecord::new(
            BlockKind::CodeBlock {
                language: Some("rust".into()),
            },
            InlineTextTree::plain("let x = 1;\nprintln!(\"hi\");"),
        );
        // markdown_line returns bare content; fences are added by persistence layer.
        let line = record.markdown_line(0, None);
        assert_eq!(line, "let x = 1;\nprintln!(\"hi\");");
    }

    #[test]
    fn separator_markdown_line_round_trips() {
        let record = BlockRecord::new(BlockKind::Separator, InlineTextTree::plain(String::new()));
        assert_eq!(record.markdown_line(0, None), "---");
        assert!(BlockKind::parse_separator_line("---"));
    }

    #[test]
    fn task_list_serializes_canonical_markdown() {
        let record = BlockRecord::new(
            BlockKind::TaskListItem { checked: false },
            InlineTextTree::plain("todo"),
        );
        assert_eq!(record.markdown_line(0, None), "- [ ] todo");
    }
}
