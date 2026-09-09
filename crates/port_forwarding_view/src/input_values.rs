use gpui::{App, Entity};
use gpui_component::input::InputState;
use rust_i18n::t;

pub(super) fn parse_port(input: &Entity<InputState>, label: &str, cx: &App) -> Result<u16, String> {
    trimmed_text(input, cx)
        .parse::<u16>()
        .map_err(|_| t!("PortForwarding.validation_number", field = label).to_string())
}

pub(super) fn trimmed_text(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).text().to_string().trim().to_string()
}
