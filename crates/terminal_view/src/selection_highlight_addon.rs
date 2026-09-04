//! 选区相同文本高亮：选中文本后，可见区域内所有相同文本以淡色背景标记。
//!
//! 复用 addon 装饰管线：每帧从当前选区提取单行文本作为 needle，在可见行内
//! 精确匹配并产出 `CellDecoration::Background` 装饰；渲染层对选中 cell 跳过
//! 装饰，因此选区本体保持原选区颜色。

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::Term;
use gpui::{Hsla, hsla};
use std::any::Any;
use std::ops::Range;

use crate::addon::{CellDecoration, DecorationSpan, TerminalAddon, TerminalAddonFrameContext};
use terminal::pty_backend::GpuiEventProxy;

/// 触发高亮的选中文本最大字符数，避免超长选区造成大面积误匹配
const MAX_HIGHLIGHT_CHARS: usize = 200;
/// 装饰优先级：低于搜索命中（当前 100 / 其他 90），搜索结果优先展示
const SELECTION_HIGHLIGHT_PRIORITY: u8 = 80;
/// 相同文本标记色：半透明淡蓝背景，仅改背景、保留原前景色
const HIGHLIGHT_BACKGROUND: Hsla = hsla(0.58, 0.75, 0.62, 0.35);

pub struct SelectionHighlightAddon {
    enabled: bool,
    cached_spans: Vec<DecorationSpan>,
}

impl SelectionHighlightAddon {
    pub fn new() -> Self {
        Self {
            enabled: true,
            cached_spans: Vec::new(),
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.cached_spans.clear();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

impl Default for SelectionHighlightAddon {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalAddon for SelectionHighlightAddon {
    fn id(&self) -> &'static str {
        "selection_highlight"
    }

    fn on_frame(&mut self, context: &TerminalAddonFrameContext) {
        self.cached_spans.clear();
        if !self.enabled || context.visible_lines.is_empty() {
            return;
        }
        let term = context.term;
        if term.columns() == 0 {
            return;
        }
        let Some(needle) = selection_needle(term) else {
            return;
        };

        for screen_line in context.visible_lines.clone() {
            let grid_line = Line(screen_line as i32 - context.display_offset as i32);
            let line_text = grid_line_text(term, grid_line);
            for col_range in match_columns(&line_text, &needle) {
                self.cached_spans.push(DecorationSpan {
                    line: screen_line,
                    col_range,
                    decoration: CellDecoration::Background {
                        color: HIGHLIGHT_BACKGROUND,
                        priority: SELECTION_HIGHLIGHT_PRIORITY,
                    },
                });
            }
        }
    }

    fn provide_decorations(
        &self,
        visible_lines: Range<usize>,
        _display_offset: usize,
    ) -> Vec<DecorationSpan> {
        self.cached_spans
            .iter()
            .filter(|span| visible_lines.contains(&span.line))
            .cloned()
            .collect()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// 从当前终端选区提取高亮 needle；仅接受单行、非空白、不超长的选区
fn selection_needle(term: &Term<GpuiEventProxy>) -> Option<String> {
    let content = term.renderable_content();
    let range = content.selection?;
    if range.start.line != range.end.line {
        return None;
    }
    if range.start.column.0 > range.end.column.0 || range.end.column.0 >= term.columns() {
        return None;
    }

    let line_text = grid_line_text(term, range.start.line);
    let needle: String = line_text
        .chars()
        .skip(range.start.column.0)
        .take(range.end.column.0 - range.start.column.0 + 1)
        .collect();
    if needle.chars().count() > MAX_HIGHLIGHT_CHARS || needle.trim().is_empty() {
        return None;
    }
    Some(needle)
}

/// 按网格列构建一行文本：每个列恰好贡献一个字符，保证字符下标 == 列下标。
/// 空单元格（含 wide char spacer）以空格占位，wide char 本体占其首列。
fn grid_line_text(term: &Term<GpuiEventProxy>, line: Line) -> String {
    let row = &term.grid()[line];
    let mut text = String::with_capacity(term.columns());
    for column in 0..term.columns() {
        let cell = &row[Column(column)];
        text.push(if cell.c == '\0' { ' ' } else { cell.c });
    }
    text
}

/// 在一行文本中查找 needle 的所有出现位置，返回列区间（左闭右开）
fn match_columns(line_text: &str, needle: &str) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    line_text
        .match_indices(needle)
        .map(|(byte_offset, _)| {
            let start = line_text[..byte_offset].chars().count();
            let length = needle.chars().count();
            start..start + length
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{SelectionHighlightAddon, grid_line_text, match_columns, selection_needle};
    use crate::addon::{TerminalAddon, TerminalAddonFrameContext};
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line, Point};
    use alacritty_terminal::selection::{Selection, SelectionType};
    use alacritty_terminal::term::{Config as TermConfig, Term};
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
    use terminal::pty_backend::GpuiEventProxy;
    use tokio::sync::mpsc::unbounded_channel;

    struct TestTermDimensions {
        columns: usize,
        screen_lines: usize,
    }

    impl Dimensions for TestTermDimensions {
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

    fn test_term_with_content(content: &[u8]) -> Term<GpuiEventProxy> {
        let (event_tx, _event_rx) = unbounded_channel();
        let dimensions = TestTermDimensions {
            columns: 20,
            screen_lines: 3,
        };
        let mut term = Term::new(
            TermConfig::default(),
            &dimensions,
            GpuiEventProxy::new(event_tx),
        );
        let mut processor: Processor<StdSyncHandler> = Processor::new();
        processor.advance(&mut term, content);
        term
    }

    fn frame_context(term: &Term<GpuiEventProxy>) -> TerminalAddonFrameContext<'_> {
        TerminalAddonFrameContext {
            term,
            visible_lines: 0..term.screen_lines(),
            display_offset: term.grid().display_offset(),
            is_local: false,
            base_dir: None,
        }
    }

    fn select(term: &mut Term<GpuiEventProxy>, start: Point, end: Point) {
        use alacritty_terminal::index::Side;

        // 起点用 Left（包含起始列），终点用 Right（包含结束列），
        // 使 to_range 结果恰好覆盖选中单元格
        let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
        selection.update(end, Side::Right);
        term.selection = Some(selection);
    }

    fn column_ranges(
        addon: &SelectionHighlightAddon,
        term: &Term<GpuiEventProxy>,
    ) -> Vec<(usize, std::ops::Range<usize>)> {
        addon
            .provide_decorations(0..term.screen_lines(), term.grid().display_offset())
            .into_iter()
            .map(|span| (span.line, span.col_range))
            .collect()
    }

    #[test]
    fn highlights_every_occurrence_of_single_line_selection() {
        let mut term = test_term_with_content(b"foo foo\r\nbar foo\r\n");
        select(
            &mut term,
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(2)),
        );

        let mut addon = SelectionHighlightAddon::new();
        addon.on_frame(&frame_context(&term));

        assert_eq!(
            vec![(0, 0..3), (0, 4..7), (1, 4..7)],
            column_ranges(&addon, &term)
        );
    }

    #[test]
    fn selected_cells_keep_selection_color_via_priority() {
        // 选区所在行同样会产出装饰 span，但渲染层对选中 cell 跳过装饰；
        // 此处只验证优先级低于搜索（100/90），搜索命中时保持搜索配色。
        let mut term = test_term_with_content(b"foo foo\r\nbar foo\r\n");
        select(
            &mut term,
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(2)),
        );

        let mut addon = SelectionHighlightAddon::new();
        addon.on_frame(&frame_context(&term));

        assert!(addon
            .provide_decorations(0..term.screen_lines(), term.grid().display_offset())
            .iter()
            .all(|span| span.decoration.priority() <= 90));
    }

    #[test]
    fn disabled_addon_produces_no_decorations() {
        let mut term = test_term_with_content(b"foo foo\r\nbar foo\r\n");
        select(
            &mut term,
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(2)),
        );

        let mut addon = SelectionHighlightAddon::new();
        addon.set_enabled(false);
        addon.on_frame(&frame_context(&term));

        assert!(column_ranges(&addon, &term).is_empty());
    }

    #[test]
    fn multi_line_selection_is_ignored() {
        let mut term = test_term_with_content(b"foo foo\r\nbar foo\r\n");
        select(
            &mut term,
            Point::new(Line(0), Column(0)),
            Point::new(Line(1), Column(2)),
        );

        let mut addon = SelectionHighlightAddon::new();
        addon.on_frame(&frame_context(&term));

        assert!(column_ranges(&addon, &term).is_empty());
    }

    #[test]
    fn whitespace_only_selection_is_ignored() {
        let mut term = test_term_with_content(b"foo    foo\r\nbar foo\r\n");
        select(
            &mut term,
            Point::new(Line(0), Column(3)),
            Point::new(Line(0), Column(6)),
        );

        let mut addon = SelectionHighlightAddon::new();
        addon.on_frame(&frame_context(&term));

        assert!(column_ranges(&addon, &term).is_empty());
    }

    #[test]
    fn clears_highlights_when_selection_is_removed() {
        let mut term = test_term_with_content(b"foo foo\r\nbar foo\r\n");
        select(
            &mut term,
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(2)),
        );

        let mut addon = SelectionHighlightAddon::new();
        addon.on_frame(&frame_context(&term));
        assert!(!column_ranges(&addon, &term).is_empty());

        term.selection = None;
        addon.on_frame(&frame_context(&term));
        assert!(column_ranges(&addon, &term).is_empty());
    }

    #[test]
    fn chinese_text_matches_by_column_not_byte() {
        // wide char 占 2 列："中文" 覆盖列 0..=3（含 spacer）
        let mut term = test_term_with_content("中文 中文\r\n中文\r\n".as_bytes());
        select(
            &mut term,
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(3)),
        );

        let mut addon = SelectionHighlightAddon::new();
        addon.on_frame(&frame_context(&term));

        assert_eq!(
            vec![(0, 0..4), (0, 5..9), (1, 0..4)],
            column_ranges(&addon, &term)
        );
    }

    #[test]
    fn needle_follows_live_selection_changes() {
        let mut term = test_term_with_content(b"foo bar\r\nbar foo\r\n");
        let mut addon = SelectionHighlightAddon::new();

        select(
            &mut term,
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(2)),
        );
        addon.on_frame(&frame_context(&term));
        assert_eq!(
            vec![(0, 0..3), (1, 4..7)],
            column_ranges(&addon, &term)
        );

        select(
            &mut term,
            Point::new(Line(0), Column(4)),
            Point::new(Line(0), Column(6)),
        );
        addon.on_frame(&frame_context(&term));
        assert_eq!(
            vec![(0, 4..7), (1, 0..3)],
            column_ranges(&addon, &term)
        );
    }

    #[test]
    fn match_columns_supports_multibyte_needles() {
        assert_eq!(vec![0..2, 3..5], match_columns("中文 中文", "中文"));
        assert_eq!(Vec::<std::ops::Range<usize>>::new(), match_columns("abc", "中文"));
    }

    #[test]
    fn needle_clamps_beyond_last_column_selection_to_full_line() {
        // to_range 会把超过最后一列的终点钳制到行尾，needle 为整行文本（含尾部空格）
        let mut term = test_term_with_content(b"foo\r\nbar\r\n");
        select(
            &mut term,
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(21)),
        );

        let needle = selection_needle(&term).expect("行尾选区应产出 needle");
        assert_eq!(term.columns(), needle.chars().count());
        assert!(needle.starts_with("foo"));
    }

    #[test]
    fn grid_line_text_keeps_char_index_aligned_with_column() {
        let term = test_term_with_content("中文\n".as_bytes());
        let text = grid_line_text(&term, Line(0));

        // 每个网格列对应一个字符：wide char 本体占其首列，spacer 以空格占位
        assert_eq!(term.columns(), text.chars().count());
        assert!(text.starts_with("中 文"));
    }
}
