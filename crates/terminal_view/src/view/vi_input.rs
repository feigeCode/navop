use super::*;

const MAX_PENDING_TERMINAL_ACTIONS: usize = 256;
const MAX_TERMINAL_ACTIONS_PER_FRAME: usize = 64;

enum TerminalActionOutcome {
    Noop,
    Notify,
    Yank(Option<String>),
    ToggleViMode(bool),
    SendEscape,
    ClearScreenApplied,
}

impl TerminalView {
    fn try_apply_terminal_action(
        &mut self,
        action: PendingTerminalAction,
        cx: &mut Context<Self>,
    ) -> Option<TerminalActionOutcome> {
        if action == PendingTerminalAction::ClearScreen {
            return self
                .terminal
                .update(cx, |terminal, cx| terminal.try_clear_screen(cx))
                .then_some(TerminalActionOutcome::ClearScreenApplied);
        }

        let term = self.terminal.read(cx).term().clone();
        let mut term = term.try_lock_unfair()?;
        let outcome = match action {
            PendingTerminalAction::ViMotion {
                motion,
                repetitions,
            } => {
                if term.mode().contains(TermMode::VI) {
                    for _ in 0..repetitions {
                        term.vi_motion(motion);
                    }
                    TerminalActionOutcome::Notify
                } else {
                    TerminalActionOutcome::Noop
                }
            }
            PendingTerminalAction::ViToggleSelection(selection_type) => {
                if term.mode().contains(TermMode::VI) {
                    let point = term.vi_mode_cursor.point;
                    if term.selection.is_some() {
                        term.selection = None;
                    } else {
                        term.selection = Some(Selection::new(selection_type, point, Side::Left));
                    }
                    TerminalActionOutcome::Notify
                } else {
                    TerminalActionOutcome::Noop
                }
            }
            PendingTerminalAction::ViYank => {
                if term.mode().contains(TermMode::VI) {
                    let text = term.selection_to_string();
                    term.selection = None;
                    TerminalActionOutcome::Yank(text)
                } else {
                    TerminalActionOutcome::Noop
                }
            }
            PendingTerminalAction::ViScrollHalf(direction) => {
                if term.mode().contains(TermMode::VI) {
                    let lines = (term.screen_lines() as i32 / 2).saturating_mul(direction);
                    let vi_cursor = term.vi_mode_cursor.scroll(&term, lines);
                    term.vi_goto_point(vi_cursor.point);
                    TerminalActionOutcome::Notify
                } else {
                    TerminalActionOutcome::Noop
                }
            }
            PendingTerminalAction::ViGotoTop => {
                if term.mode().contains(TermMode::VI) {
                    let point = AlacPoint::new(Line(term.topmost_line().0), Column(0));
                    term.vi_goto_point(point);
                    TerminalActionOutcome::Notify
                } else {
                    TerminalActionOutcome::Noop
                }
            }
            PendingTerminalAction::ViGotoBottom => {
                if term.mode().contains(TermMode::VI) {
                    let point = AlacPoint::new(term.bottommost_line(), Column(0));
                    term.vi_goto_point(point);
                    TerminalActionOutcome::Notify
                } else {
                    TerminalActionOutcome::Noop
                }
            }
            PendingTerminalAction::ToggleViMode => {
                term.toggle_vi_mode();
                TerminalActionOutcome::ToggleViMode(term.mode().contains(TermMode::VI))
            }
            PendingTerminalAction::ResolveClearSelection {
                accepts_live_input,
                had_block_selection,
            } => {
                let in_vi_mode = term.mode().contains(TermMode::VI);
                let has_selection = term.selection.is_some();

                if in_vi_mode {
                    if has_selection {
                        term.selection = None;
                        TerminalActionOutcome::Notify
                    } else if accepts_live_input {
                        term.toggle_vi_mode();
                        TerminalActionOutcome::Notify
                    } else {
                        TerminalActionOutcome::Noop
                    }
                } else if has_selection || had_block_selection {
                    if has_selection {
                        term.selection = None;
                    }
                    TerminalActionOutcome::Notify
                } else if accepts_live_input {
                    TerminalActionOutcome::SendEscape
                } else {
                    TerminalActionOutcome::Noop
                }
            }
            PendingTerminalAction::SelectAll => {
                let start = AlacPoint::new(Line(-(term.history_size() as i32)), Column(0));
                let end = AlacPoint::new(
                    Line(term.screen_lines() as i32 - 1),
                    Column(term.columns().saturating_sub(1)),
                );
                term.selection = Some(Selection::new(SelectionType::Simple, start, Side::Left));
                if let Some(selection) = &mut term.selection {
                    selection.update(end, Side::Right);
                }
                TerminalActionOutcome::Notify
            }
            PendingTerminalAction::ClearScreen => unreachable!(),
        };
        drop(term);
        Some(outcome)
    }

    fn finish_terminal_action(
        &mut self,
        outcome: TerminalActionOutcome,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        match outcome {
            TerminalActionOutcome::Noop => {}
            TerminalActionOutcome::Notify => cx.notify(),
            TerminalActionOutcome::Yank(text) => {
                if let Some(text) = text {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                cx.notify();
            }
            TerminalActionOutcome::ToggleViMode(in_vi_mode) => {
                if let Some(window) = window {
                    let shortcut = terminal_shortcut_label(TERMINAL_TOGGLE_VI_MODE_SHORTCUT);
                    let message = if in_vi_mode {
                        t!("TerminalView.vi_mode_enabled", shortcut = shortcut).to_string()
                    } else {
                        t!("TerminalView.vi_mode_disabled", shortcut = shortcut).to_string()
                    };
                    window.push_notification(message, cx);
                }
                cx.notify();
            }
            TerminalActionOutcome::SendEscape => {
                self.write_to_pty(b"\x1b".to_vec(), cx);
            }
            TerminalActionOutcome::ClearScreenApplied => {
                self.reset_render_cache(cx);
                cx.notify();
            }
        }
    }

    fn enqueue_terminal_action(&mut self, action: PendingTerminalAction) {
        let coalesced_motion = if let (
            Some(PendingTerminalAction::ViMotion {
                motion: queued_motion,
                repetitions,
            }),
            PendingTerminalAction::ViMotion {
                motion,
                repetitions: additional_repetitions,
            },
        ) = (self.pending_terminal_actions.back_mut(), action)
        {
            if *queued_motion == motion {
                *repetitions = repetitions.saturating_add(additional_repetitions);
                true
            } else {
                false
            }
        } else {
            false
        };
        if coalesced_motion {
            return;
        }

        if self.pending_terminal_actions.len() >= MAX_PENDING_TERMINAL_ACTIONS {
            tracing::warn!(
                limit = MAX_PENDING_TERMINAL_ACTIONS,
                ?action,
                "dropping terminal UI action because the non-blocking retry queue is full"
            );
            return;
        }
        self.pending_terminal_actions.push_back(action);
    }

    pub(super) fn apply_or_queue_terminal_action(
        &mut self,
        action: PendingTerminalAction,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        if self.pending_terminal_actions.is_empty() {
            if let Some(outcome) = self.try_apply_terminal_action(action, cx) {
                self.finish_terminal_action(outcome, window, cx);
                return;
            }
        }

        self.enqueue_terminal_action(action);
        self.schedule_terminal_render_retry(cx);
    }

    pub(super) fn apply_pending_terminal_actions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut applied = 0;
        while applied < MAX_TERMINAL_ACTIONS_PER_FRAME {
            let Some(action) = self.pending_terminal_actions.front().copied() else {
                return;
            };
            let Some(outcome) = self.try_apply_terminal_action(action, cx) else {
                self.schedule_terminal_render_retry(cx);
                return;
            };
            self.pending_terminal_actions.pop_front();
            self.finish_terminal_action(outcome, Some(&mut *window), cx);
            applied += 1;
        }

        if !self.pending_terminal_actions.is_empty() {
            self.schedule_terminal_render_retry(cx);
        }
    }

    pub(super) fn handle_vi_key_event(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }

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

        if let Some(motion) = motion {
            self.apply_or_queue_terminal_action(
                PendingTerminalAction::ViMotion {
                    motion,
                    repetitions: 1,
                },
                None,
                cx,
            );
            return;
        }

        let action = match key.as_str() {
            "v" if !ctrl && !shift => {
                self.vi_start_selection(SelectionType::Simple, cx);
                return;
            }
            "v" if shift => {
                self.vi_start_selection(SelectionType::Lines, cx);
                return;
            }
            "y" => Some(PendingTerminalAction::ViYank),
            "u" if ctrl => Some(PendingTerminalAction::ViScrollHalf(1)),
            "d" if ctrl => Some(PendingTerminalAction::ViScrollHalf(-1)),
            "g" if !shift => Some(PendingTerminalAction::ViGotoTop),
            "g" if shift => Some(PendingTerminalAction::ViGotoBottom),
            _ => None,
        };

        if let Some(action) = action {
            self.apply_or_queue_terminal_action(action, None, cx);
        }
    }

    pub(super) fn vi_start_selection(
        &mut self,
        selection_type: SelectionType,
        cx: &mut Context<Self>,
    ) {
        self.apply_or_queue_terminal_action(
            PendingTerminalAction::ViToggleSelection(selection_type),
            None,
            cx,
        );
    }

    pub(super) fn toggle_vi_mode(
        &mut self,
        _: &ToggleViMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        self.apply_or_queue_terminal_action(PendingTerminalAction::ToggleViMode, Some(window), cx);
    }
}
