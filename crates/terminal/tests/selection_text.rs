use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use terminal::selection_text_from_term;

struct TestDimensions {
    columns: usize,
    screen_lines: usize,
}

struct SelectionBounds {
    start: Point,
    end: Point,
}

impl Dimensions for TestDimensions {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

fn selection_text(input: &[u8], columns: usize, bounds: SelectionBounds) -> Option<String> {
    let dimensions = TestDimensions {
        columns,
        screen_lines: 4,
    };
    let mut term = Term::new(Config::default(), &dimensions, VoidListener);
    let mut processor: Processor<StdSyncHandler> = Processor::new();
    processor.advance(&mut term, input);

    let mut selection = Selection::new(SelectionType::Simple, bounds.start, Side::Left);
    selection.update(bounds.end, Side::Right);
    term.selection = Some(selection);
    selection_text_from_term(&term)
}

fn point(line: i32, column: usize) -> Point {
    Point::new(Line(line), Column(column))
}

#[test]
fn clipboard_selection_expands_tabs_to_visual_spaces() {
    assert_eq!(
        Some("A       B".to_string()),
        selection_text(
            b"A\tB",
            16,
            SelectionBounds {
                start: point(0, 0),
                end: point(0, 8),
            },
        )
    );
}

#[test]
fn clipboard_selection_expands_tabs_from_the_current_column() {
    assert_eq!(
        Some("AB      C".to_string()),
        selection_text(
            b"AB\tC",
            16,
            SelectionBounds {
                start: point(0, 0),
                end: point(0, 8),
            },
        )
    );
}

#[test]
fn clipboard_selection_preserves_existing_spaces() {
    assert_eq!(
        Some("A   B".to_string()),
        selection_text(
            b"A   B",
            16,
            SelectionBounds {
                start: point(0, 0),
                end: point(0, 4),
            },
        )
    );
}

#[test]
fn clipboard_selection_preserves_hard_line_breaks() {
    assert_eq!(
        Some("foo\nbar".to_string()),
        selection_text(
            b"foo\r\nbar",
            16,
            SelectionBounds {
                start: point(0, 0),
                end: point(1, 2),
            },
        )
    );
}

#[test]
fn clipboard_selection_does_not_add_breaks_for_wrapped_lines() {
    assert_eq!(
        Some("abcdefgh".to_string()),
        selection_text(
            b"abcdefgh",
            4,
            SelectionBounds {
                start: point(0, 0),
                end: point(1, 3),
            },
        )
    );
}
