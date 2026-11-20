use super::*;
use gpui_component::tooltip::Tooltip;
use rust_i18n::t;
use ssh::X11ForwardingStatus;

const X11_SESSION_FOOTER_HEIGHT: Pixels = px(28.0);
const X11_SESSION_FOOTER_TEXT_SIZE: Pixels = px(11.0);

pub(super) struct X11SessionStatus {
    label: String,
    disabled: bool,
}

pub(super) fn session_status(status: Option<X11ForwardingStatus>) -> Option<X11SessionStatus> {
    let (label, disabled) = match status? {
        X11ForwardingStatus::External { server } => (
            t!("TerminalX11.external", server = server).to_string(),
            false,
        ),
        X11ForwardingStatus::Embedded { version, .. } => (
            t!("TerminalX11.embedded", version = version).to_string(),
            false,
        ),
        X11ForwardingStatus::Disabled { reason } => (
            t!("TerminalX11.disabled", reason = reason).to_string(),
            true,
        ),
    };
    Some(X11SessionStatus { label, disabled })
}

impl TerminalView {
    pub(super) fn render_x11_session_footer(&self, cx: &App) -> AnyElement {
        let Some(status) = session_status(self.terminal.read(cx).x11_status()) else {
            return div().into_any_element();
        };
        let colors = self.current_theme.colors();
        let status_color = if status.disabled {
            cx.theme().warning
        } else {
            cx.theme().success
        };
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
