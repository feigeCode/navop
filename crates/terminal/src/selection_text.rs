use std::cmp;
use std::ops::Range;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::selection::{SelectionRange, SelectionType};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::{Flags, LineLength};

struct LineSelection {
    line: Line,
    columns: Range<Column>,
    include_wrapped_wide: bool,
}

/// Convert the active terminal selection to clipboard text.
pub fn selection_text_from_term<T>(term: &Term<T>) -> Option<String> {
    let selection = term.selection.as_ref()?;
    let SelectionRange { start, end, .. } = selection.to_range(term)?;

    let text = match selection.ty {
        SelectionType::Block => block_selection_text(term, start, end),
        SelectionType::Lines => bounds_text(term, start, end) + "\n",
        SelectionType::Simple | SelectionType::Semantic => bounds_text(term, start, end),
    };
    Some(text)
}

fn block_selection_text<T>(
    term: &Term<T>,
    start: alacritty_terminal::index::Point,
    end: alacritty_terminal::index::Point,
) -> String {
    let mut lines = Vec::new();
    for line in (start.line.0..end.line.0).map(Line::from) {
        let selection = LineSelection {
            line,
            columns: start.column..end.column,
            include_wrapped_wide: start.column.0 != 0,
        };
        lines.push(visual_line_text(term, selection).trim_end().to_string());
    }

    let selection = LineSelection {
        line: end.line,
        columns: start.column..end.column,
        include_wrapped_wide: true,
    };
    lines.push(visual_line_text(term, selection).trim_end().to_string());
    lines.join("\n")
}

fn bounds_text<T>(
    term: &Term<T>,
    start: alacritty_terminal::index::Point,
    end: alacritty_terminal::index::Point,
) -> String {
    let mut text = String::new();
    for line in (start.line.0..=end.line.0).map(Line::from) {
        let start_col = if line == start.line {
            start.column
        } else {
            Column(0)
        };
        let end_col = if line == end.line {
            end.column
        } else {
            term.last_column()
        };
        let selection = LineSelection {
            line,
            columns: start_col..end_col,
            include_wrapped_wide: line == end.line,
        };
        text += &visual_line_text(term, selection);
    }
    text.strip_suffix('\n').map(str::to_owned).unwrap_or(text)
}

fn visual_line_text<T>(term: &Term<T>, mut selection: LineSelection) -> String {
    let grid_line = &term.grid()[selection.line];
    let columns = &mut selection.columns;
    let line_length = cmp::min(grid_line.line_length(), columns.end + 1);
    if grid_line[columns.start]
        .flags
        .contains(Flags::WIDE_CHAR_SPACER)
    {
        columns.start -= 1;
    }

    let mut text = String::new();
    for column in (columns.start.0..line_length.0).map(Column::from) {
        push_visual_cell(&mut text, &grid_line[column]);
    }

    if columns.end >= term.columns() - 1
        && (line_length.0 == 0 || !grid_line[line_length - 1].flags.contains(Flags::WRAPLINE))
    {
        text.push('\n');
    }

    if line_length == term.columns()
        && line_length.0 >= 2
        && grid_line[line_length - 1]
            .flags
            .contains(Flags::LEADING_WIDE_CHAR_SPACER)
        && selection.include_wrapped_wide
    {
        text.push(term.grid()[selection.line - 1i32][Column(0)].c);
    }

    text
}

fn push_visual_cell(text: &mut String, cell: &alacritty_terminal::term::cell::Cell) {
    if cell
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
        return;
    }
    // Preserve the grid's visual columns instead of copying a literal tab whose width can differ
    // in the paste target.
    text.push(if cell.c == '\t' { ' ' } else { cell.c });
    if let Some(characters) = cell.zerowidth() {
        text.extend(characters);
    }
}
