use super::*;
use ssh::X11ForwardingStatus;

fn session_notice(x11_enabled: bool, status: Option<&X11ForwardingStatus>) -> Option<String> {
    if !x11_enabled {
        return None;
    }
    match status? {
        X11ForwardingStatus::External { server } => {
            Some(t!("TerminalX11.external", server = server).to_string())
        }
        X11ForwardingStatus::Disabled { .. } => None,
    }
}

fn take_session_notice(
    notified: &mut bool,
    x11_enabled: bool,
    status: Option<&X11ForwardingStatus>,
) -> Option<String> {
    let Some(notice) = session_notice(x11_enabled, status) else {
        *notified = false;
        return None;
    };
    if *notified {
        return None;
    }
    *notified = true;
    Some(notice)
}

impl TerminalView {
    pub(super) fn notify_x11_ready_once(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let terminal = self.terminal.read(cx);
        let x11_enabled = terminal
            .ssh_config()
            .is_some_and(|config| config.ssh_config.x11_forwarding);
        let Some(notice) = take_session_notice(
            &mut self.x11_ready_notified,
            x11_enabled,
            terminal.x11_status().as_ref(),
        ) else {
            return;
        };
        window.push_notification(Notification::info(notice).autohide(true), cx);
    }
}

#[cfg(test)]
mod tests;
