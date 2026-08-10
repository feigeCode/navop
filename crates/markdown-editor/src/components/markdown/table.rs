//! Native Markdown table data model plus parse and serialize helpers.
//!
//! Tables are supported as native blocks at the root level and inside
//! quote-like containers in rendered mode. More complex nested contexts that
//! are still outside the runtime-safe subset continue to use raw-Markdown
//! fallback paths.

use gpui::{Entity, FontStyle, FontWeight, Pixels, SharedString, TextRun, Window, px};

use crate::components::{Block, InlineTextTree};
use crate::theme::Theme;

/// Horizontal alignment declared by the table's delimiter row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableColumnAlignment {
    /// No explicit alignment marker (`---`). Renders left, but stays distinct
    /// from [`Left`](Self::Left) so an unmarked column is not silently rewritten
    /// with a leading colon on the next serialize.
    Default,
    /// Explicit left alignment (`:---`).
    Left,
    /// Center-aligned cells (`:---:`).
    Center,
    /// Right-aligned cells (`---:`).
    Right,
}

/// Axis kinds addressable by rendered-mode native table maintenance UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableAxisKind {
    /// Table row axis.
    Row,
    /// Table column axis.
    Column,
}

/// A row or column marker inside one native table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableAxisMarker {
    pub kind: TableAxisKind,
    pub index: usize,
}

/// Visual emphasis level used when previewing or selecting table axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableAxisHighlight {
    /// No axis emphasis.
    #[default]
    None,
    /// Hover preview emphasis.
    Preview,
    /// Persistent selected-axis emphasis.
    Selected,
}

/// Persistent cell contents for a native table block.
#[derive(Debug, Clone)]
pub struct TableData {
    pub header: Vec<InlineTextTree>,
    pub rows: Vec<Vec<InlineTextTree>>,
    pub alignments: Vec<TableColumnAlignment>,
}

impl PartialEq for TableData {
    fn eq(&self, other: &Self) -> bool {
        self.header == other.header
            && self.rows == other.rows
            && self.alignments == other.alignments
    }
}

impl Eq for TableData {}

impl TableData {
    /// Creates an empty table with one header row, `body_rows` body rows, and
    /// `columns` left-aligned columns.
    pub fn new_empty(body_rows: usize, columns: usize) -> Self {
        let columns = columns.max(1);
        let header = (0..columns)
            .map(|_| InlineTextTree::plain(String::new()))
            .collect::<Vec<_>>();
        let rows = (0..body_rows.max(1))
            .map(|_| {
                (0..columns)
                    .map(|_| InlineTextTree::plain(String::new()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let alignments = vec![TableColumnAlignment::Default; columns];
        Self {
            header,
            rows,
            alignments,
        }
    }

    pub(crate) fn column_count(&self) -> usize {
        self.header
            .len()
            .max(self.alignments.len())
            .max(self.rows.iter().map(Vec::len).max().unwrap_or(0))
            .max(1)
    }

    fn normalize_shape(&mut self) {
        let columns = self.column_count();
        while self.header.len() < columns {
            self.header.push(InlineTextTree::plain(String::new()));
        }
        while self.alignments.len() < columns {
            self.alignments.push(TableColumnAlignment::Default);
        }
        for row in &mut self.rows {
            while row.len() < columns {
                row.push(InlineTextTree::plain(String::new()));
            }
        }
    }

    /// Appends one empty body row while preserving the current column count.
    pub fn append_row(&mut self) {
        self.normalize_shape();
        let columns = self.column_count();
        self.rows.push(
            (0..columns)
                .map(|_| InlineTextTree::plain(String::new()))
                .collect(),
        );
    }

    /// Appends one empty column to the header and every body row.
    pub fn append_column(&mut self, alignment: TableColumnAlignment) {
        self.normalize_shape();
        self.header.push(InlineTextTree::plain(String::new()));
        self.alignments.push(alignment);
        for row in &mut self.rows {
            row.push(InlineTextTree::plain(String::new()));
        }
    }

    /// Inserts an empty visual row. Visual row `0` is the header; inserting
    /// there promotes the previous header to the first body row so the table
    /// remains valid Markdown with exactly one delimiter-backed header.
    pub fn insert_visual_row(&mut self, visual_row: usize) {
        self.normalize_shape();
        let columns = self.column_count();
        let empty_row = || {
            (0..columns)
                .map(|_| InlineTextTree::plain(String::new()))
                .collect::<Vec<_>>()
        };
        if visual_row == 0 {
            let previous_header = std::mem::replace(&mut self.header, empty_row());
            self.rows.insert(0, previous_header);
        } else {
            self.rows
                .insert((visual_row - 1).min(self.rows.len()), empty_row());
        }
    }

    /// Inserts an empty column before the supplied column index, or appends
    /// when the index is at/past the current right edge.
    pub fn insert_column(&mut self, column: usize, alignment: TableColumnAlignment) {
        self.normalize_shape();
        let column = column.min(self.column_count());
        self.header
            .insert(column, InlineTextTree::plain(String::new()));
        self.alignments.insert(column, alignment);
        for row in &mut self.rows {
            row.insert(column, InlineTextTree::plain(String::new()));
        }
    }

    /// Resizes the table to `visual_rows` rows (including the header) and
    /// `columns` columns, preserving the top-left rectangle of existing data.
    ///
    /// Markdown tables always retain at least one visible header row and one
    /// column. Body rows may be empty, which represents a valid header-only
    /// table.
    pub fn resize(&mut self, visual_rows: usize, columns: usize) {
        self.normalize_shape();
        let visual_rows = visual_rows.max(1);
        let columns = columns.max(1);

        self.header
            .resize_with(columns, || InlineTextTree::plain(String::new()));
        self.header.truncate(columns);

        self.alignments
            .resize(columns, TableColumnAlignment::Default);
        self.alignments.truncate(columns);

        self.rows
            .resize_with(visual_rows - 1, || Vec::with_capacity(columns));
        self.rows.truncate(visual_rows - 1);
        for row in &mut self.rows {
            row.resize_with(columns, || InlineTextTree::plain(String::new()));
            row.truncate(columns);
        }
    }

    /// Sets the alignment of one column if it exists.
    pub fn set_column_alignment(&mut self, column: usize, alignment: TableColumnAlignment) {
        self.normalize_shape();
        if let Some(slot) = self.alignments.get_mut(column) {
            *slot = alignment;
        }
    }

    /// Swaps two rows addressed by their visual index, where row `0` is the
    /// header and rows `1..=rows.len()` are the body rows. Swapping the header
    /// with the first body row exchanges header and body content, mirroring how
    /// the row handles treat the header as just another movable row.
    pub fn swap_visual_rows(&mut self, row_a: usize, row_b: usize) {
        self.normalize_shape();
        let total = self.rows.len() + 1;
        if row_a >= total || row_b >= total || row_a == row_b {
            return;
        }
        match (row_a, row_b) {
            (0, other) | (other, 0) => {
                std::mem::swap(&mut self.header, &mut self.rows[other - 1]);
            }
            (a, b) => self.rows.swap(a - 1, b - 1),
        }
    }

    /// Swaps two columns across header, body, and alignment vectors.
    pub fn swap_columns(&mut self, col_a: usize, col_b: usize) {
        self.normalize_shape();
        let columns = self.column_count();
        if col_a >= columns || col_b >= columns || col_a == col_b {
            return;
        }

        self.header.swap(col_a, col_b);
        self.alignments.swap(col_a, col_b);
        for row in &mut self.rows {
            row.swap(col_a, col_b);
        }
    }

    /// Removes one body row while preserving at least one body row.
    pub fn remove_body_row(&mut self, row_index: usize) {
        self.normalize_shape();
        if row_index >= self.rows.len() {
            return;
        }
        // A table may be left header-only; the editor removes the whole block
        // when the header itself is then deleted.
        self.rows.remove(row_index);
    }

    /// Removes the header row by promoting the first body row into its place.
    /// Returns false (leaving the table unchanged) when there are no body rows,
    /// since a pipe table must keep a header row.
    pub fn remove_header_row(&mut self) -> bool {
        self.normalize_shape();
        if self.rows.is_empty() {
            return false;
        }
        self.header = self.rows.remove(0);
        true
    }

    /// Removes one column while preserving at least one column.
    pub fn remove_column(&mut self, col_index: usize) {
        self.normalize_shape();
        let columns = self.column_count();
        if columns <= 1 || col_index >= columns {
            return;
        }

        self.header.remove(col_index);
        self.alignments.remove(col_index);
        for row in &mut self.rows {
            row.remove(col_index);
        }
    }
}

/// Responsive width fractions shared by every row of a native table.
#[derive(Debug, Clone, PartialEq)]
pub struct TableColumnLayout {
    fractions: Vec<f32>,
}

impl TableColumnLayout {
    pub fn equal(column_count: usize) -> Self {
        let column_count = column_count.max(1);
        let fraction = 1.0 / column_count as f32;
        Self {
            fractions: vec![fraction; column_count],
        }
    }

    #[cfg(test)]
    pub(crate) fn fractions(&self) -> &[f32] {
        &self.fractions
    }

    pub fn fraction(&self, column: usize) -> f32 {
        self.fractions
            .get(column)
            .copied()
            .unwrap_or_else(|| 1.0 / self.fractions.len().max(1) as f32)
    }

    pub fn measure(
        table: &TableData,
        table_width: f32,
        window: &mut Window,
        theme: &Theme,
    ) -> Self {
        let preferred_widths = measure_preferred_column_widths(table, window, theme)
            .into_iter()
            .map(f32::from)
            .collect::<Vec<_>>();
        Self::from_preferred_widths(&preferred_widths, table_width, minimum_column_width(theme))
    }

    pub fn from_preferred_widths(
        preferred_widths: &[f32],
        table_width: f32,
        min_column_width: f32,
    ) -> Self {
        if preferred_widths.is_empty() {
            return Self::equal(1);
        }

        let column_count = preferred_widths.len();
        let safe_table_width = table_width.max(1.0);
        let equal_share = safe_table_width / column_count as f32;
        if preferred_widths
            .iter()
            .all(|preferred| *preferred <= equal_share + f32::EPSILON)
        {
            return Self::equal(column_count);
        }

        let floor_width = min_column_width
            .max(0.0)
            .min(safe_table_width / column_count as f32);
        let weights = preferred_widths
            .iter()
            .map(|preferred| preferred.max(equal_share))
            .collect::<Vec<_>>();
        let mut assigned_widths = vec![0.0; column_count];
        let mut remaining_indices = (0..column_count).collect::<Vec<_>>();
        let mut remaining_width = safe_table_width;

        loop {
            if remaining_indices.is_empty() {
                break;
            }

            let weight_sum = remaining_indices
                .iter()
                .map(|index| weights[*index])
                .sum::<f32>();
            if weight_sum <= f32::EPSILON {
                let share = remaining_width / remaining_indices.len() as f32;
                for index in remaining_indices {
                    assigned_widths[index] = share;
                }
                break;
            }

            let mut newly_floored = Vec::new();
            for index in &remaining_indices {
                let width = remaining_width * (weights[*index] / weight_sum);
                if width < floor_width - f32::EPSILON {
                    newly_floored.push(*index);
                } else {
                    assigned_widths[*index] = width;
                }
            }

            if newly_floored.is_empty() {
                break;
            }

            if newly_floored.len() == remaining_indices.len() {
                let share = remaining_width / remaining_indices.len() as f32;
                for index in remaining_indices {
                    assigned_widths[index] = share;
                }
                break;
            }

            for index in &newly_floored {
                assigned_widths[*index] = floor_width;
                remaining_width -= floor_width;
            }
            remaining_indices.retain(|index| !newly_floored.contains(index));
        }

        let assigned_sum = assigned_widths.iter().sum::<f32>();
        if assigned_sum <= f32::EPSILON {
            return Self::equal(column_count);
        }

        let fractions = assigned_widths
            .into_iter()
            .map(|width| width / assigned_sum)
            .collect::<Vec<_>>();
        Self { fractions }
    }
}

/// Runtime-only location of a cell inside a native table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableCellPosition {
    /// Zero-based visual row. Header is row `0`; first body row is `1`.
    pub row: usize,
    pub column: usize,
}

impl TableCellPosition {
    pub fn is_header(self) -> bool {
        self.row == 0
    }

    pub fn body_row_index(self) -> Option<usize> {
        self.row.checked_sub(1)
    }
}

/// Runtime cell editors attached to one native table block.
#[derive(Clone)]
pub struct TableRuntime {
    pub header: Vec<Entity<Block>>,
    pub rows: Vec<Vec<Entity<Block>>>,
}

impl TableRuntime {
    pub fn cell(&self, position: TableCellPosition) -> Option<Entity<Block>> {
        if position.is_header() {
            self.header.get(position.column).cloned()
        } else {
            self.rows
                .get(position.body_row_index()?)
                .and_then(|row| row.get(position.column))
                .cloned()
        }
    }
}

fn measure_preferred_column_widths(
    table: &TableData,
    window: &mut Window,
    theme: &Theme,
) -> Vec<Pixels> {
    let column_count = table.header.len().max(1);
    let mut preferred_widths = vec![Pixels::ZERO; column_count];

    for (column, cell) in table.header.iter().enumerate() {
        preferred_widths[column] =
            preferred_widths[column].max(measure_cell_preferred_width(cell, true, window, theme));
    }

    for row in &table.rows {
        for (column, cell) in row.iter().enumerate().take(column_count) {
            preferred_widths[column] = preferred_widths[column]
                .max(measure_cell_preferred_width(cell, false, window, theme));
        }
    }

    preferred_widths
}

fn measure_cell_preferred_width(
    cell: &InlineTextTree,
    is_header: bool,
    window: &mut Window,
    theme: &Theme,
) -> Pixels {
    let cache = cell.render_cache();
    let text = cache.visible_text();
    let cell_chrome_width = cell_chrome_width(theme);
    if text.is_empty() {
        return cell_chrome_width;
    }

    let display_text = SharedString::from(text.to_string());
    let text_style = window.text_style();
    let mut font = text_style.font();
    if is_header && font.weight < FontWeight::MEDIUM {
        font.weight = FontWeight::MEDIUM;
    }
    let base_run = TextRun {
        len: display_text.len(),
        font,
        color: theme.colors.text_default,
        background_color: None,
        underline: None,
        strikethrough: None,
        letter_spacing: text_style.letter_spacing,
    };
    let runs = measurement_runs(&cache, &base_run);
    let font_size = px(theme.typography.text_size);

    let text_width = window
        .text_system()
        .shape_text(display_text, font_size, &runs, None, None)
        .ok()
        .map(|lines| {
            lines
                .iter()
                .map(|line| line.width())
                .max()
                .unwrap_or(Pixels::ZERO)
        })
        .unwrap_or(Pixels::ZERO);

    text_width + cell_chrome_width
}

fn measurement_runs(
    cache: &crate::components::InlineRenderCache,
    base_run: &TextRun,
) -> Vec<TextRun> {
    let mut boundaries = vec![0, cache.visible_text().len()];
    for span in cache.spans() {
        boundaries.push(span.range.start);
        boundaries.push(span.range.end);
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut runs = Vec::new();
    for boundary_pair in boundaries.windows(2) {
        let start = boundary_pair[0];
        let end = boundary_pair[1];
        if start >= end {
            continue;
        }

        let inline_style = cache.style_at(start);
        let mut font = base_run.font.clone();
        if inline_style.bold && font.weight < FontWeight::BOLD {
            font.weight = FontWeight::BOLD;
        }
        if inline_style.italic {
            font.style = FontStyle::Italic;
        }

        runs.push(TextRun {
            len: end - start,
            font,
            color: base_run.color,
            background_color: None,
            underline: None,
            strikethrough: None,
            letter_spacing: base_run.letter_spacing,
        });
    }

    if runs.is_empty() {
        vec![base_run.clone()]
    } else {
        runs
    }
}

fn cell_chrome_width(theme: &Theme) -> Pixels {
    px(theme.dimensions.table_cell_padding_x * 2.0 + 2.0)
}

fn minimum_column_width(theme: &Theme) -> f32 {
    theme.dimensions.table_cell_padding_x * 2.0 + theme.typography.text_size * 4.0 + 2.0
}

fn strip_table_indent(line: &str) -> Option<&str> {
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    (indent <= 3).then_some(&line[indent..])
}

fn split_table_cells(line: &str) -> Option<Vec<String>> {
    let rest = strip_table_indent(line)?.trim_end();
    if rest.is_empty() {
        return None;
    }
    // Outer pipes are optional (GFM): strip them when present so pipeless rows
    // like `Name | Score` split the same way as `| Name | Score |`.
    let inner = rest.strip_prefix('|').unwrap_or(rest);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut escaping = false;

    for ch in inner.chars() {
        if escaping {
            match ch {
                '|' | '\\' => current.push(ch),
                _ => {
                    current.push('\\');
                    current.push(ch);
                }
            }
            escaping = false;
            continue;
        }

        match ch {
            '\\' => escaping = true,
            '|' => {
                cells.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    if escaping {
        current.push('\\');
    }
    cells.push(current.trim().to_string());
    Some(cells)
}

fn parse_alignment_cell(cell: &str) -> Option<TableColumnAlignment> {
    let trimmed = cell.trim();
    if trimmed.len() < 3 {
        return None;
    }

    let left = trimmed.starts_with(':');
    let right = trimmed.ends_with(':');
    let core = trimmed.trim_start_matches(':').trim_end_matches(':');
    if core.len() < 3 || !core.chars().all(|ch| ch == '-') {
        return None;
    }

    Some(match (left, right) {
        (true, true) => TableColumnAlignment::Center,
        (false, true) => TableColumnAlignment::Right,
        (true, false) => TableColumnAlignment::Left,
        (false, false) => TableColumnAlignment::Default,
    })
}

fn serialize_alignment(alignment: TableColumnAlignment) -> &'static str {
    match alignment {
        TableColumnAlignment::Default => "---",
        TableColumnAlignment::Left => ":---",
        TableColumnAlignment::Center => ":---:",
        TableColumnAlignment::Right => "---:",
    }
}

pub(crate) fn serialize_table_cell_markdown(tree: &InlineTextTree) -> String {
    tree.serialize_markdown()
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', " ")
}

fn serialize_row<'a>(cells: impl IntoIterator<Item = &'a InlineTextTree>) -> String {
    let rendered = cells
        .into_iter()
        .map(serialize_table_cell_markdown)
        .collect::<Vec<_>>();
    format!("| {} |", rendered.join(" | "))
}

/// Returns true when a line is a candidate native table row in the current
/// container scope.
pub fn is_table_candidate_line(line: &str) -> bool {
    strip_table_indent(line)
        .map(str::trim_end)
        .is_some_and(|rest| rest.starts_with('|'))
}

/// Number of pipe-separated cells in `line`, treating outer pipes as optional
/// (GFM) so pipeless rows like `Name | Score` are recognized. Returns `None`
/// for single-column lines so prose containing a stray `|` is not mistaken for
/// a table row.
pub fn table_row_column_count(line: &str) -> Option<usize> {
    split_table_cells(line)
        .map(|cells| cells.len())
        .filter(|count| *count >= 2)
}

/// True when `line` could be a table row, including a pipeless GFM row.
pub fn is_table_row_candidate(line: &str) -> bool {
    table_row_column_count(line).is_some()
}

/// Collects a contiguous table-candidate region in the current container
/// scope.
pub fn collect_table_candidate_region(lines: &[String], start: usize) -> usize {
    let mut index = start + 1;
    while index < lines.len() && is_table_candidate_line(&lines[index]) {
        index += 1;
    }
    index
}

/// Parses a pipe-table region into native table data.
pub fn parse_table_region(lines: &[String]) -> Option<TableData> {
    if lines.len() < 2 {
        return None;
    }

    let header = split_table_cells(&lines[0])?;
    let alignment_cells = split_table_cells(&lines[1])?;
    if header.is_empty() || alignment_cells.len() != header.len() {
        return None;
    }

    let alignments = alignment_cells
        .iter()
        .map(|cell| parse_alignment_cell(cell))
        .collect::<Option<Vec<_>>>()?;

    let mut rows = Vec::new();
    for line in &lines[2..] {
        // GFM normalizes body rows to the header width: short rows are padded
        // with empty cells and long rows drop their trailing cells, instead of
        // invalidating the whole table.
        let mut cells = split_table_cells(line)?;
        cells.resize(header.len(), String::new());
        rows.push(
            cells
                .into_iter()
                .map(|cell| InlineTextTree::from_markdown(&cell))
                .collect::<Vec<_>>(),
        );
    }

    Some(TableData {
        header: header
            .into_iter()
            .map(|cell| InlineTextTree::from_markdown(&cell))
            .collect(),
        rows,
        alignments,
    })
}

/// Returns true when `line` is a delimiter row of exactly `columns` cells, each
/// a valid alignment specifier.
fn is_delimiter_row(line: &str, columns: usize) -> bool {
    split_table_cells(line).is_some_and(|cells| {
        cells.len() == columns
            && cells
                .iter()
                .all(|cell| parse_alignment_cell(cell).is_some())
    })
}

/// Detects a table that starts at `start` without requiring outer pipes,
/// returning the region end (exclusive) when `lines[start]` is a multi-column
/// header followed by a matching delimiter row. Body rows extend to the next
/// blank line, matching GFM. Returns `None` for ordinary prose so a stray `|`
/// is never mistaken for a table; single-column pipeless candidates are also
/// rejected because they are ambiguous with setext headings.
pub fn collect_pipeless_table_region(lines: &[String], start: usize) -> Option<usize> {
    let header = split_table_cells(lines.get(start)?)?;
    if header.len() < 2 {
        return None;
    }
    if !is_delimiter_row(lines.get(start + 1)?, header.len()) {
        return None;
    }

    let mut end = start + 2;
    while end < lines.len() && !lines[end].trim().is_empty() {
        end += 1;
    }
    Some(end)
}

/// Returns true when a root-level line is a candidate native table row.
pub fn is_root_table_candidate_line(line: &str) -> bool {
    is_table_candidate_line(line)
}

/// Collects a contiguous root-level table candidate region.
pub fn collect_root_table_candidate_region(lines: &[String], start: usize) -> usize {
    collect_table_candidate_region(lines, start)
}

/// Parses a root-level pipe table region into native table data.
pub fn parse_root_table_region(lines: &[String]) -> Option<TableData> {
    parse_table_region(lines)
}

/// Parses a single table body row, normalized to `columns` cells (padded when
/// short, truncated when long). Returns `None` when the line is not a table
/// row at all.
pub fn parse_table_body_row(line: &str, columns: usize) -> Option<Vec<InlineTextTree>> {
    let mut cells = split_table_cells(line)?;
    cells.resize(columns, String::new());
    Some(
        cells
            .into_iter()
            .map(|cell| InlineTextTree::from_markdown(&cell))
            .collect(),
    )
}

/// Serializes native table data to canonical pipe-table Markdown lines.
pub fn serialize_table_markdown_lines(table: &TableData) -> Vec<String> {
    let mut lines = Vec::with_capacity(2 + table.rows.len());
    lines.push(serialize_row(table.header.iter()));
    lines.push(format!(
        "| {} |",
        table
            .alignments
            .iter()
            .map(|alignment| serialize_alignment(*alignment))
            .collect::<Vec<_>>()
            .join(" | ")
    ));
    lines.extend(table.rows.iter().map(|row| serialize_row(row.iter())));
    lines
}

#[cfg(test)]
mod tests {
    use super::{
        TableColumnAlignment, TableColumnLayout, TableData, collect_pipeless_table_region,
        collect_root_table_candidate_region, is_root_table_candidate_line, parse_root_table_region,
        serialize_table_markdown_lines,
    };
    use crate::components::InlineTextTree;

    fn assert_close(left: f32, right: f32) {
        assert!(
            (left - right).abs() < 0.0001,
            "expected {left} to be close to {right}"
        );
    }

    #[test]
    fn parses_valid_root_table_region() {
        let lines = vec![
            "| Left | Center | Right |".to_string(),
            "| :--- | :---: | ---: |".to_string(),
            "| a | b | c |".to_string(),
        ];
        let table = parse_root_table_region(&lines).expect("table should parse");
        assert_eq!(table.alignments.len(), 3);
        assert_eq!(
            table.alignments,
            vec![
                TableColumnAlignment::Left,
                TableColumnAlignment::Center,
                TableColumnAlignment::Right
            ]
        );
        assert_eq!(table.header[0].serialize_markdown(), "Left");
        assert_eq!(table.rows[0][2].serialize_markdown(), "c");
    }

    #[test]
    fn rejects_invalid_alignment_row() {
        let lines = vec!["| Left | Right |".to_string(), "| nope | --- |".to_string()];
        assert!(parse_root_table_region(&lines).is_none());
    }

    #[test]
    fn rejects_alignment_row_with_wrong_column_count() {
        let lines = vec!["| A | B | C |".to_string(), "| --- | --- |".to_string()];
        assert!(parse_root_table_region(&lines).is_none());
    }

    #[test]
    fn preserves_explicit_left_alignment_colon() {
        // ":---" is explicit left and must survive a parse/serialize round-trip
        // instead of being silently rewritten to a bare "---".
        let lines = vec![
            "| L | D | R |".to_string(),
            "| :--- | --- | ---: |".to_string(),
            "| a | b | c |".to_string(),
        ];
        let table = parse_root_table_region(&lines).expect("table should parse");
        assert_eq!(
            table.alignments,
            vec![
                TableColumnAlignment::Left,
                TableColumnAlignment::Default,
                TableColumnAlignment::Right
            ]
        );
        assert_eq!(
            serialize_table_markdown_lines(&table)[1],
            "| :--- | --- | ---: |"
        );
    }

    #[test]
    fn pads_short_body_rows_and_truncates_long_ones() {
        let lines = vec![
            "| A | B | C |".to_string(),
            "| --- | --- | --- |".to_string(),
            "| short |".to_string(),
            "| 1 | 2 | 3 | 4 |".to_string(),
        ];
        let table = parse_root_table_region(&lines).expect("table should parse");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0].len(), 3);
        assert_eq!(table.rows[0][0].serialize_markdown(), "short");
        assert!(table.rows[0][1].serialize_markdown().is_empty());
        assert!(table.rows[0][2].serialize_markdown().is_empty());
        assert_eq!(table.rows[1].len(), 3);
        assert_eq!(table.rows[1][2].serialize_markdown(), "3");
    }

    #[test]
    fn parses_pipeless_table() {
        let lines = vec![
            "Name | Score".to_string(),
            "--- | ---".to_string(),
            "Alice | 10".to_string(),
            "Bob | 7".to_string(),
        ];
        let end = collect_pipeless_table_region(&lines, 0).expect("region");
        assert_eq!(end, 4);
        let table = parse_root_table_region(&lines[..end]).expect("table should parse");
        assert_eq!(table.header.len(), 2);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.header[0].serialize_markdown(), "Name");
        assert_eq!(table.rows[1][1].serialize_markdown(), "7");
    }

    #[test]
    fn prose_with_pipe_is_not_a_pipeless_table() {
        let lines = vec!["this | that".to_string(), "and the next line".to_string()];
        assert!(collect_pipeless_table_region(&lines, 0).is_none());
    }

    #[test]
    fn pipeless_table_requires_valid_delimiter_row() {
        let lines = vec!["Name | Score".to_string(), "Alice | 10".to_string()];
        assert!(collect_pipeless_table_region(&lines, 0).is_none());
    }

    #[test]
    fn single_column_pipeless_is_not_a_table() {
        // Ambiguous with a setext heading; must not be captured as a table.
        let lines = vec!["Title".to_string(), "---".to_string()];
        assert!(collect_pipeless_table_region(&lines, 0).is_none());
    }

    #[test]
    fn serializes_canonical_pipe_table() {
        let table = TableData {
            header: vec![
                InlineTextTree::from_markdown("**bold**"),
                InlineTextTree::from_markdown("[link](https://example.com)"),
            ],
            rows: vec![vec![
                InlineTextTree::plain("A | B".to_string()),
                InlineTextTree::plain("value".to_string()),
            ]],
            alignments: vec![TableColumnAlignment::Default, TableColumnAlignment::Right],
        };
        assert_eq!(
            serialize_table_markdown_lines(&table),
            vec![
                "| **bold** | [link](https://example.com) |".to_string(),
                "| --- | ---: |".to_string(),
                "| A \\| B | value |".to_string(),
            ]
        );
    }

    #[test]
    fn detects_root_table_candidate_runs() {
        let lines = vec![
            "| A | B |".to_string(),
            "| --- | --- |".to_string(),
            "| 1 | 2 |".to_string(),
            "paragraph".to_string(),
        ];
        assert!(is_root_table_candidate_line(&lines[0]));
        assert_eq!(collect_root_table_candidate_region(&lines, 0), 3);
    }

    #[test]
    fn equal_share_fast_path_keeps_columns_uniform() {
        let layout = TableColumnLayout::from_preferred_widths(&[32.0, 64.0, 48.0], 360.0, 60.0);
        let fractions = layout.fractions();
        assert_eq!(fractions.len(), 3);
        assert_close(fractions[0], 1.0 / 3.0);
        assert_close(fractions[1], 1.0 / 3.0);
        assert_close(fractions[2], 1.0 / 3.0);
    }

    #[test]
    fn content_pressure_redistributes_width_across_the_whole_column() {
        let layout = TableColumnLayout::from_preferred_widths(&[48.0, 220.0, 48.0], 360.0, 60.0);
        let fractions = layout.fractions();
        assert_eq!(fractions.len(), 3);
        assert!(fractions[1] > fractions[0]);
        assert!(fractions[1] > fractions[2]);
        assert_close(fractions[0], fractions[2]);
    }

    #[test]
    fn minimum_column_floor_prevents_neighbor_collapse() {
        let layout = TableColumnLayout::from_preferred_widths(&[16.0, 900.0, 16.0], 300.0, 70.0);
        let fractions = layout.fractions();
        let widths = fractions
            .iter()
            .map(|fraction| fraction * 300.0)
            .collect::<Vec<_>>();
        assert!(widths[0] >= 70.0 - 0.001);
        assert!(widths[2] >= 70.0 - 0.001);
        assert_close(fractions.iter().sum::<f32>(), 1.0);
    }

    #[test]
    fn moderate_single_cell_growth_stays_equal_when_share_is_sufficient() {
        let layout = TableColumnLayout::from_preferred_widths(&[56.0, 92.0, 56.0], 360.0, 60.0);
        let fractions = layout.fractions();
        assert_close(fractions[0], 1.0 / 3.0);
        assert_close(fractions[1], 1.0 / 3.0);
        assert_close(fractions[2], 1.0 / 3.0);
    }

    #[test]
    fn append_row_preserves_column_count_and_creates_empty_cells() {
        let mut table = TableData::new_empty(1, 3);
        table.append_row();

        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[1].len(), 3);
        assert!(
            table.rows[1]
                .iter()
                .all(|cell| cell.serialize_markdown().is_empty())
        );
    }

    #[test]
    fn append_column_extends_every_row_and_uses_requested_alignment() {
        let mut table = TableData {
            header: vec![
                InlineTextTree::plain("A".to_string()),
                InlineTextTree::plain("B".to_string()),
            ],
            rows: vec![
                vec![
                    InlineTextTree::plain("1".to_string()),
                    InlineTextTree::plain("2".to_string()),
                ],
                vec![
                    InlineTextTree::plain("3".to_string()),
                    InlineTextTree::plain("4".to_string()),
                ],
            ],
            alignments: vec![TableColumnAlignment::Left, TableColumnAlignment::Right],
        };

        table.append_column(TableColumnAlignment::Right);

        assert_eq!(table.header.len(), 3);
        assert_eq!(table.rows[0].len(), 3);
        assert_eq!(table.rows[1].len(), 3);
        assert_eq!(
            table.alignments,
            vec![
                TableColumnAlignment::Left,
                TableColumnAlignment::Right,
                TableColumnAlignment::Right,
            ]
        );
        assert!(table.header[2].serialize_markdown().is_empty());
        assert!(table.rows[0][2].serialize_markdown().is_empty());
        assert!(table.rows[1][2].serialize_markdown().is_empty());
    }

    #[test]
    fn append_column_pads_missing_alignments_with_default() {
        let mut table = TableData {
            header: vec![InlineTextTree::plain("A".to_string())],
            rows: vec![vec![InlineTextTree::plain("1".to_string())]],
            alignments: Vec::new(),
        };

        table.append_column(TableColumnAlignment::Left);

        assert_eq!(
            table.alignments,
            vec![TableColumnAlignment::Default, TableColumnAlignment::Left]
        );
        assert_eq!(table.header.len(), 2);
        assert_eq!(table.rows[0].len(), 2);
    }

    #[test]
    fn insert_visual_row_at_header_promotes_previous_header() {
        let mut table = TableData {
            header: vec![
                InlineTextTree::plain("A".to_string()),
                InlineTextTree::plain("B".to_string()),
            ],
            rows: vec![vec![
                InlineTextTree::plain("1".to_string()),
                InlineTextTree::plain("2".to_string()),
            ]],
            alignments: vec![TableColumnAlignment::Left, TableColumnAlignment::Right],
        };

        table.insert_visual_row(0);

        assert_eq!(table.header.len(), 2);
        assert!(
            table
                .header
                .iter()
                .all(|cell| cell.serialize_markdown().is_empty())
        );
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0][0].serialize_markdown(), "A");
        assert_eq!(table.rows[0][1].serialize_markdown(), "B");
        assert_eq!(table.rows[1][0].serialize_markdown(), "1");
        assert_eq!(
            table.alignments,
            vec![TableColumnAlignment::Left, TableColumnAlignment::Right]
        );
    }

    #[test]
    fn insert_visual_row_before_body_and_at_bottom_preserves_shape() {
        let mut table = TableData {
            header: vec![
                InlineTextTree::plain("A".to_string()),
                InlineTextTree::plain("B".to_string()),
            ],
            rows: vec![vec![
                InlineTextTree::plain("1".to_string()),
                InlineTextTree::plain("2".to_string()),
            ]],
            alignments: vec![TableColumnAlignment::Default, TableColumnAlignment::Center],
        };

        table.insert_visual_row(1);
        assert_eq!(table.rows.len(), 2);
        assert!(
            table.rows[0]
                .iter()
                .all(|cell| cell.serialize_markdown().is_empty())
        );
        assert_eq!(table.rows[1][0].serialize_markdown(), "1");

        table.insert_visual_row(usize::MAX);
        assert_eq!(table.rows.len(), 3);
        assert!(
            table.rows[2]
                .iter()
                .all(|cell| cell.serialize_markdown().is_empty())
        );
        assert!(table.rows.iter().all(|row| row.len() == 2));
    }

    #[test]
    fn insert_column_updates_header_rows_and_alignment() {
        let mut table = TableData {
            header: vec![
                InlineTextTree::plain("A".to_string()),
                InlineTextTree::plain("B".to_string()),
            ],
            rows: vec![
                vec![
                    InlineTextTree::plain("1".to_string()),
                    InlineTextTree::plain("2".to_string()),
                ],
                vec![
                    InlineTextTree::plain("3".to_string()),
                    InlineTextTree::plain("4".to_string()),
                ],
            ],
            alignments: vec![TableColumnAlignment::Left, TableColumnAlignment::Right],
        };

        table.insert_column(0, TableColumnAlignment::Center);
        assert_eq!(
            table.alignments,
            vec![
                TableColumnAlignment::Center,
                TableColumnAlignment::Left,
                TableColumnAlignment::Right,
            ]
        );
        assert!(table.header[0].serialize_markdown().is_empty());
        assert_eq!(table.header[1].serialize_markdown(), "A");
        assert!(table.rows.iter().all(|row| row.len() == 3));
        assert!(table.rows[0][0].serialize_markdown().is_empty());
        assert_eq!(table.rows[0][1].serialize_markdown(), "1");

        table.insert_column(usize::MAX, TableColumnAlignment::Default);
        assert_eq!(table.column_count(), 4);
        assert_eq!(
            table.alignments.last(),
            Some(&TableColumnAlignment::Default)
        );
        assert!(table.header[3].serialize_markdown().is_empty());
        assert!(
            table
                .rows
                .iter()
                .all(|row| row[3].serialize_markdown().is_empty())
        );
    }

    #[test]
    fn resize_preserves_top_left_data_and_allows_header_only_tables() {
        let mut table = TableData {
            header: vec![
                InlineTextTree::plain("A".to_string()),
                InlineTextTree::plain("B".to_string()),
                InlineTextTree::plain("C".to_string()),
            ],
            rows: vec![
                vec![
                    InlineTextTree::plain("1".to_string()),
                    InlineTextTree::plain("2".to_string()),
                    InlineTextTree::plain("3".to_string()),
                ],
                vec![
                    InlineTextTree::plain("4".to_string()),
                    InlineTextTree::plain("5".to_string()),
                    InlineTextTree::plain("6".to_string()),
                ],
            ],
            alignments: vec![
                TableColumnAlignment::Left,
                TableColumnAlignment::Center,
                TableColumnAlignment::Right,
            ],
        };

        table.resize(2, 2);
        assert_eq!(table.header.len(), 2);
        assert_eq!(table.header[0].serialize_markdown(), "A");
        assert_eq!(table.header[1].serialize_markdown(), "B");
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0][0].serialize_markdown(), "1");
        assert_eq!(table.rows[0][1].serialize_markdown(), "2");
        assert_eq!(
            table.alignments,
            vec![TableColumnAlignment::Left, TableColumnAlignment::Center]
        );

        table.resize(1, 4);
        assert!(table.rows.is_empty());
        assert_eq!(table.header.len(), 4);
        assert_eq!(table.header[0].serialize_markdown(), "A");
        assert_eq!(table.header[1].serialize_markdown(), "B");
        assert!(table.header[2].serialize_markdown().is_empty());
        assert!(table.header[3].serialize_markdown().is_empty());
        assert_eq!(
            table.alignments,
            vec![
                TableColumnAlignment::Left,
                TableColumnAlignment::Center,
                TableColumnAlignment::Default,
                TableColumnAlignment::Default,
            ]
        );
    }

    #[test]
    fn set_column_alignment_updates_requested_column() {
        let mut table = TableData::new_empty(2, 3);
        table.set_column_alignment(1, TableColumnAlignment::Center);
        assert_eq!(
            table.alignments,
            vec![
                TableColumnAlignment::Default,
                TableColumnAlignment::Center,
                TableColumnAlignment::Default
            ]
        );
    }

    #[test]
    fn swap_visual_rows_exchanges_header_with_first_body_row() {
        let mut table = TableData {
            header: vec![InlineTextTree::plain("A".to_string())],
            rows: vec![
                vec![InlineTextTree::plain("1".to_string())],
                vec![InlineTextTree::plain("2".to_string())],
            ],
            alignments: vec![TableColumnAlignment::Left],
        };
        // Visual row 0 is the header; swapping it with visual row 1 exchanges
        // header and first-body content.
        table.swap_visual_rows(0, 1);
        assert_eq!(table.header[0].serialize_markdown(), "1");
        assert_eq!(table.rows[0][0].serialize_markdown(), "A");
        assert_eq!(table.rows[1][0].serialize_markdown(), "2");

        // Two body rows (visual 1 and 2) swap like ordinary rows.
        table.swap_visual_rows(1, 2);
        assert_eq!(table.rows[0][0].serialize_markdown(), "2");
        assert_eq!(table.rows[1][0].serialize_markdown(), "A");
    }

    #[test]
    fn swap_columns_exchanges_header_body_and_alignment() {
        let mut table = TableData {
            header: vec![
                InlineTextTree::plain("A".to_string()),
                InlineTextTree::plain("B".to_string()),
            ],
            rows: vec![vec![
                InlineTextTree::plain("1".to_string()),
                InlineTextTree::plain("2".to_string()),
            ]],
            alignments: vec![TableColumnAlignment::Left, TableColumnAlignment::Right],
        };
        table.swap_columns(0, 1);
        assert_eq!(table.header[0].serialize_markdown(), "B");
        assert_eq!(table.rows[0][0].serialize_markdown(), "2");
        assert_eq!(
            table.alignments,
            vec![TableColumnAlignment::Right, TableColumnAlignment::Left]
        );
    }

    #[test]
    fn remove_body_row_can_empty_the_table() {
        let mut table = TableData::new_empty(2, 2);
        table.remove_body_row(0);
        assert_eq!(table.rows.len(), 1);
        table.remove_body_row(0);
        // The last body row can be removed, leaving a header-only table.
        assert!(table.rows.is_empty());
        // Out-of-range removal is a no-op.
        table.remove_body_row(0);
        assert!(table.rows.is_empty());
    }

    #[test]
    fn remove_header_row_promotes_first_body_row() {
        let mut table = parse_root_table_region(&[
            "| A | B |".to_string(),
            "| --- | --- |".to_string(),
            "| 1 | 2 |".to_string(),
            "| 3 | 4 |".to_string(),
        ])
        .expect("valid table");

        assert!(table.remove_header_row());
        assert_eq!(table.header[0].serialize_markdown(), "1");
        assert_eq!(table.header[1].serialize_markdown(), "2");
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0][0].serialize_markdown(), "3");

        // Promoting the last remaining row leaves a header-only table.
        assert!(table.remove_header_row());
        assert!(table.rows.is_empty());
        assert!(!table.remove_header_row());
        assert_eq!(table.header[0].serialize_markdown(), "3");
    }

    #[test]
    fn remove_column_preserves_at_least_one_column() {
        let mut table = TableData::new_empty(2, 2);
        table.remove_column(0);
        assert_eq!(table.column_count(), 1);
        table.remove_column(0);
        assert_eq!(table.column_count(), 1);
    }
}
