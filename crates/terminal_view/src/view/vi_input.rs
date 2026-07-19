use super::*;

impl TerminalView {
    pub(super) fn handle_vi_key_event(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        use alacritty_terminal::vi_mode::ViMotion;

        let key = &event.keystroke.key;
        let shift = event.keystroke.modifiers.shift;
        let ctrl = event.keystroke.modifiers.control;

        let motion = match (key.as_str(), shift, ctrl) {
            ("h", true, false) => Some(ViMotion::High),
            ("m", true, false) => Some(ViMotion::Middle),
            ("l", true, false) => Some(ViMotion::Low),
            ("b", true, false) => Some(ViMotion::WordLeft),
            ("w", true, false) => Some(ViMotion::WordRight),
            ("e", true, false) => Some(ViMotion::WordRightEnd),
            ("h" | "left", false, false) => Some(ViMotion::Left),
            ("j" | "down", false, false) => Some(ViMotion::Down),
            ("k" | "up", false, false) => Some(ViMotion::Up),
            ("l" | "right", false, false) => Some(ViMotion::Right),
            ("0", _, false) => Some(ViMotion::First),
            ("$", _, false) => Some(ViMotion::Last),
            ("^", _, false) => Some(ViMotion::FirstOccupied),
            ("b", false, false) => Some(ViMotion::SemanticLeft),
            ("w", false, false) => Some(ViMotion::SemanticRight),
            ("e", false, false) => Some(ViMotion::SemanticRightEnd),
            ("%", _, false) => Some(ViMotion::Bracket),
            ("{", _, false) => Some(ViMotion::ParagraphUp),
            ("}", _, false) => Some(ViMotion::ParagraphDown),
            _ => None,
        };

        if let Some(ref motion) = motion {
            let term = self.terminal.read(cx).term().clone();
            let mut term = term.lock();
            term.vi_motion(motion.clone());
            drop(term);
            cx.notify();
            return;
        }

        let term = self.terminal.read(cx).term().clone();

        match key.as_str() {
            "v" if !ctrl && !shift => {
                self.vi_start_selection(SelectionType::Simple, cx);
            }
            "v" if shift => {
                self.vi_start_selection(SelectionType::Lines, cx);
            }
            "y" => {
                let term = term.lock();
                if let Some(text) = term.selection_to_string() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                drop(term);
                self.terminal.read(cx).term().lock().selection = None;
                cx.notify();
            }
            "u" if ctrl => {
                let mut term = term.lock();
                let lines = term.screen_lines() as i32 / 2;
                let vi_cursor = term.vi_mode_cursor.scroll(&term, lines);
                term.vi_goto_point(vi_cursor.point);
                drop(term);
                cx.notify();
            }
            "d" if ctrl => {
                let mut term = term.lock();
                let lines = term.screen_lines() as i32 / 2;
                let vi_cursor = term.vi_mode_cursor.scroll(&term, -lines);
                term.vi_goto_point(vi_cursor.point);
                drop(term);
                cx.notify();
            }
            "g" if !shift => {
                let mut term = term.lock();
                let point = AlacPoint::new(Line(term.topmost_line().0), Column(0));
                term.vi_goto_point(point);
                drop(term);
                cx.notify();
            }
            "g" if shift => {
                let mut term = term.lock();
                let point = AlacPoint::new(term.bottommost_line(), Column(0));
                term.vi_goto_point(point);
                drop(term);
                cx.notify();
            }
            _ => {}
        }
    }

    pub(super) fn vi_start_selection(
        &mut self,
        selection_type: SelectionType,
        cx: &mut Context<Self>,
    ) {
        use alacritty_terminal::selection::Selection;

        let term = self.terminal.read(cx).term().clone();
        let mut term = term.lock();
        let point = term.vi_mode_cursor.point;
        if term.selection.is_some() {
            term.selection = None;
        } else {
            term.selection = Some(Selection::new(selection_type, point, Side::Left));
        }
        drop(term);
        cx.notify();
    }

    pub(super) fn toggle_vi_mode(
        &mut self,
        _: &ToggleViMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let in_vi_mode = self.terminal.update(cx, |terminal, _| {
            terminal.toggle_vi_mode();
            terminal.mode().contains(TermMode::VI)
        });
        let shortcut = terminal_shortcut_label(TERMINAL_TOGGLE_VI_MODE_SHORTCUT);
        let message = if in_vi_mode {
            t!("TerminalView.vi_mode_enabled", shortcut = shortcut).to_string()
        } else {
            t!("TerminalView.vi_mode_disabled", shortcut = shortcut).to_string()
        };
        window.push_notification(message, cx);
        cx.notify();
    }
}
