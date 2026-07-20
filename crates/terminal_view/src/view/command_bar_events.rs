use super::*;
use crate::view::command_bar_model::command_submission_bytes;

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
                let Some(input) = command_submission_bytes(command) else {
                    return;
                };
                self.write_to_pty(input, cx);
                self.focus_terminal(window, cx);
            }
            TerminalCommandBarEvent::PasteTerminal(command) => {
                self.paste_text(command, window, cx);
            }
            TerminalCommandBarEvent::FocusTerminal => self.focus_terminal(window, cx),
        }
    }
}
