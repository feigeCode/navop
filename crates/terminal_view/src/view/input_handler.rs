use super::*;

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        _actual_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        self.marked_text_range()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.clear_marked_text(cx);
    }

    fn replace_text_in_range(
        &mut self,
        _replacement_range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_marked_text(cx);
        self.commit_text(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        new_text: &str,
        new_marked_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_marked_text(new_text.to_string(), new_marked_range, cx);
    }

    fn bounds_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        _bounds: Bounds<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // 获取光标位置用于 IME 定位
        let term = self.terminal.read(cx).term().lock();
        let cursor = term.grid().cursor.point;
        let display_offset = term.grid().display_offset();
        drop(term);

        let screen_line = cursor.line.0 + display_offset as i32;
        let col = cursor.column.0;

        // 计算像素位置
        let origin = Point::new(
            self.terminal_bounds.origin.x + self.cell_width * col as f32,
            self.terminal_bounds.origin.y + self.line_height * screen_line as f32,
        );

        Some(Bounds::new(origin, size(self.cell_width, self.line_height)))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
