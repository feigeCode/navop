use alacritty_terminal::index::Point as AlacPoint;
use gpui::{Modifiers, MouseButton};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BlockSelection {
    pub anchor: AlacPoint,
    pub active: AlacPoint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlockSelectionBounds {
    pub start_line: i32,
    pub end_line: i32,
    pub start_col: usize,
    pub end_col: usize,
}

impl BlockSelection {
    pub(super) fn new(anchor: AlacPoint) -> Self {
        Self {
            anchor,
            active: anchor,
        }
    }

    pub(super) fn update(&mut self, active: AlacPoint) {
        self.active = active;
    }

    pub(super) fn bounds(&self) -> BlockSelectionBounds {
        block_selection_bounds(self.anchor, self.active)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.anchor == self.active
    }
}

impl BlockSelectionBounds {
    pub(crate) fn contains_screen_cell(&self, line: usize, col: usize) -> bool {
        let Ok(line) = i32::try_from(line) else {
            return false;
        };
        line >= self.start_line
            && line <= self.end_line
            && col >= self.start_col
            && col <= self.end_col
    }
}

pub(super) fn should_start_block_selection(button: MouseButton, modifiers: Modifiers) -> bool {
    button == MouseButton::Left
        && modifiers.alt
        && !modifiers.shift
        && !modifiers.control
        && !modifiers.platform
}

pub(super) fn block_selection_bounds(start: AlacPoint, end: AlacPoint) -> BlockSelectionBounds {
    let start_line = start.line.0.min(end.line.0);
    let end_line = start.line.0.max(end.line.0);
    let start_col = start.column.0.min(end.column.0);
    let end_col = start.column.0.max(end.column.0);

    BlockSelectionBounds {
        start_line,
        end_line,
        start_col,
        end_col,
    }
}

pub(super) fn block_selection_text_from_rows(
    rows: &[String],
    start: AlacPoint,
    end: AlacPoint,
) -> Option<String> {
    let bounds = block_selection_bounds(start, end);
    let start_line = usize::try_from(bounds.start_line).ok()?;
    let end_line = usize::try_from(bounds.end_line).ok()?;
    if start_line >= rows.len() || bounds.start_col > bounds.end_col {
        return None;
    }

    let lines = rows
        .iter()
        .skip(start_line)
        .take(end_line.saturating_sub(start_line) + 1)
        .map(|row| slice_row(row, bounds.start_col, bounds.end_col))
        .collect::<Vec<_>>();
    let text = lines.join("\n");
    if text.is_empty() { None } else { Some(text) }
}

fn slice_row(row: &str, start_col: usize, end_col: usize) -> String {
    let width = end_col.saturating_sub(start_col) + 1;
    let mut out = row.chars().skip(start_col).take(width).collect::<String>();
    while out.ends_with(' ') {
        out.pop();
    }
    out
}
