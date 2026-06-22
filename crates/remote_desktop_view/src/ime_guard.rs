use std::ops::Range;

use gpui::{App, Bounds, InputHandler, Pixels, Point, UTF16Selection, Window};

pub struct RemoteDesktopImeGuard {
    bounds: Bounds<Pixels>,
}

impl RemoteDesktopImeGuard {
    pub fn new(bounds: Bounds<Pixels>) -> Self {
        Self { bounds }
    }

    fn selected_range(&self) -> UTF16Selection {
        UTF16Selection {
            range: 0..0,
            reversed: false,
        }
    }

    fn marked_range(&self) -> Option<Range<usize>> {
        None
    }

    fn text_for_local_range(&self) -> Option<String> {
        None
    }

    fn candidate_bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }

    fn character_index(&self) -> usize {
        0
    }
}

impl InputHandler for RemoteDesktopImeGuard {
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<UTF16Selection> {
        Some(self.selected_range())
    }

    fn marked_text_range(&mut self, _window: &mut Window, _cx: &mut App) -> Option<Range<usize>> {
        self.marked_range()
    }

    fn text_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<String> {
        self.text_for_local_range()
    }

    fn replace_text_in_range(
        &mut self,
        _replacement_range: Option<Range<usize>>,
        _text: &str,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        _new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut App) {}

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        Some(self.candidate_bounds())
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<usize> {
        Some(self.character_index())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{bounds, point, px, size};

    #[test]
    fn exposes_empty_text_state_for_platform_ime() {
        let guard = RemoteDesktopImeGuard::new(bounds(
            point(px(10.0), px(20.0)),
            size(px(300.0), px(200.0)),
        ));

        assert_eq!(guard.selected_range().range, 0..0);
        assert!(!guard.selected_range().reversed);
        assert_eq!(guard.marked_range(), None);
        assert_eq!(guard.text_for_local_range(), None);
        assert_eq!(guard.character_index(), 0);
    }

    #[test]
    fn reports_remote_desktop_bounds_for_ime_positioning() {
        let area = bounds(point(px(10.0), px(20.0)), size(px(300.0), px(200.0)));
        let guard = RemoteDesktopImeGuard::new(area);

        assert_eq!(guard.candidate_bounds(), area);
    }
}
