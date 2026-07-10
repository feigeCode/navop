use gpui::{App, AppContext, Context, Entity, Window};
use gpui_component::input::InputState;
use one_core::storage::RemoteDesktopProtocol;
use rust_i18n::t;

use super::RemoteDesktopFormWindow;

pub struct RemoteDesktopInputs {
    pub name: Entity<InputState>,
    pub host: Entity<InputState>,
    pub port: Entity<InputState>,
    pub username: Entity<InputState>,
    pub password: Entity<InputState>,
    pub domain: Entity<InputState>,
    pub proxy_host: Entity<InputState>,
    pub proxy_port: Entity<InputState>,
    pub proxy_username: Entity<InputState>,
    pub proxy_password: Entity<InputState>,
}

pub fn create_inputs(
    protocol: RemoteDesktopProtocol,
    window: &mut Window,
    cx: &mut Context<RemoteDesktopFormWindow>,
) -> RemoteDesktopInputs {
    RemoteDesktopInputs {
        name: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("RemoteDesktopForm.placeholder_name").to_string())
        }),
        host: cx.new(|cx| InputState::new(window, cx).placeholder("10.2.178.12")),
        port: cx.new(|cx| input_with_value(protocol.default_port().to_string(), window, cx)),
        username: cx.new(|cx| InputState::new(window, cx).placeholder("administrator")),
        password: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("RemoteDesktopForm.placeholder_password").to_string())
                .masked(true)
        }),
        domain: cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("RemoteDesktopForm.placeholder_domain").to_string())
        }),
        proxy_host: cx.new(|cx| InputState::new(window, cx).placeholder("127.0.0.1")),
        proxy_port: cx.new(|cx| input_with_value("1080".to_string(), window, cx)),
        proxy_username: cx.new(|cx| InputState::new(window, cx)),
        proxy_password: cx.new(|cx| InputState::new(window, cx).masked(true)),
    }
}

fn input_with_value(
    value: String,
    window: &mut Window,
    cx: &mut Context<InputState>,
) -> InputState {
    let mut state = InputState::new(window, cx);
    state.set_value(&value, window, cx);
    state
}

pub fn parse_u16(value: &str, label: &str) -> Result<u16, String> {
    value
        .trim()
        .parse::<u16>()
        .map_err(|_| t!("RemoteDesktopForm.invalid_number", field = label).to_string())
}

pub fn non_empty_text(input: &Entity<InputState>, cx: &App) -> Option<String> {
    let value = input_text(input, cx).trim().to_string();
    (!value.is_empty()).then_some(value)
}

pub fn input_text(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).text().to_string()
}

#[cfg(test)]
mod tests {
    use super::parse_u16;

    #[test]
    fn parses_port() {
        assert_eq!(Ok(3389), parse_u16("3389", "Port"));
        assert!(parse_u16("abc", "Port").is_err());
    }
}
