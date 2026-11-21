use super::*;
use crate::view::command_bar_model::{command_batch_lines, command_submission_bytes};
use std::time::Duration;

/// Interval between batch statements so the shell can execute them one by one.
const BATCH_COMMAND_INTERVAL: Duration = Duration::from_millis(80);

impl TerminalView {
    pub(super) fn handle_command_bar_event(
        &mut self,
        _command_bar: &Entity<TerminalCommandBar>,
        event: &TerminalCommandBarEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalCommandBarEvent::Submit(command) => {
                if !self.accepts_live_terminal_input(cx) {
                    return;
                }
                let mut lines = command_batch_lines(command).into_iter();
                let Some(first) = lines.next() else {
                    return;
                };
                if let Some(input) = command_submission_bytes(&first) {
                    self.write_to_pty(input, cx);
                }
                let remaining: Vec<String> = lines.collect();
                if !remaining.is_empty() {
                    cx.spawn(async move |this, cx| {
                        for line in remaining {
                            cx.background_executor().timer(BATCH_COMMAND_INTERVAL).await;
                            if let Some(input) = command_submission_bytes(&line) {
                                let _ = this.update(cx, |this, cx| this.write_to_pty(input, cx));
                            }
                        }
                    })
                    .detach();
                }
                self.focus_terminal(window, cx);
            }
            TerminalCommandBarEvent::InputToPty(command) => {
                if !self.accepts_live_terminal_input(cx) {
                    return;
                }
                self.paste_text(command, window, cx);
            }
            TerminalCommandBarEvent::FocusTerminal => self.focus_terminal(window, cx),
        }
    }
}
