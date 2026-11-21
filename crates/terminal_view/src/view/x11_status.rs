use super::*;
use gpui_component::tooltip::Tooltip;
use rust_i18n::t;
use ssh::X11ForwardingStatus;

const X11_SESSION_FOOTER_HEIGHT: Pixels = px(28.0);
const X11_SESSION_FOOTER_TEXT_SIZE: Pixels = px(11.0);

pub(super) struct X11SessionStatus {
    label: String,
}

pub(super) fn session_status(
    x11_enabled: bool,
    status: Option<X11ForwardingStatus>,
) -> Option<X11SessionStatus> {
    if !x11_enabled {
        return None;
    }
    let label = match status? {
        X11ForwardingStatus::External { server } => {
            t!("TerminalX11.external", server = server).to_string()
        }
        X11ForwardingStatus::Embedded { version, .. } => {
            t!("TerminalX11.embedded", version = version).to_string()
        }
        X11ForwardingStatus::Disabled { .. } => return None,
    };
    Some(X11SessionStatus { label })
}

impl TerminalView {
    pub(super) fn render_x11_session_footer(&self, cx: &App) -> AnyElement {
        let terminal = self.terminal.read(cx);
        let x11_enabled = terminal
            .ssh_config()
            .is_some_and(|config| config.ssh_config.x11_forwarding);
        let Some(status) = session_status(x11_enabled, terminal.x11_status()) else {
            return div().into_any_element();
        };
        let colors = self.current_theme.colors();
        let status_color = cx.theme().success;
        let tooltip = status.label.clone();
        h_flex()
            .id("terminal-x11-session-footer")
            .debug_selector(|| "terminal-x11-session-footer".to_string())
            .w_full()
            .h(X11_SESSION_FOOTER_HEIGHT)
            .min_h(X11_SESSION_FOOTER_HEIGHT)
            .max_h(X11_SESSION_FOOTER_HEIGHT)
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_3()
            .border_t_1()
            .border_color(colors.border)
            .bg(colors.background)
            .text_size(X11_SESSION_FOOTER_TEXT_SIZE)
            .child(div().size_2().rounded_full().bg(status_color))
            .child(div().whitespace_nowrap().child("X11"))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(status_color)
                    .child(status.label),
            )
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
