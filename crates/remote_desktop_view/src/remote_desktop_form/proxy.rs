use gpui::{Context, Window};
use gpui_component::input::InputState;
use one_core::storage::{ProxyConfig, ProxyType};
use rust_i18n::t;

use super::RemoteDesktopFormWindow;

pub(super) fn build_proxy_config(
    enabled: bool,
    proxy_type: ProxyType,
    host: &str,
    port: &str,
    username: &str,
    password: &str,
) -> Result<Option<ProxyConfig>, &'static str> {
    if !enabled {
        return Ok(None);
    }
    let host = optional_value(host).ok_or("proxy_host")?;
    let port = port
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or("proxy_port")?;
    let username = optional_value(username);
    let password = optional_secret(password);
    if username.is_none() && password.is_some() {
        return Err("proxy_username");
    }
    Ok(Some(ProxyConfig {
        proxy_type,
        host,
        port,
        username,
        password,
    }))
}

fn optional_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn optional_secret(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

impl RemoteDesktopFormWindow {
    pub(super) fn apply_proxy(
        &mut self,
        proxy: Option<ProxyConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(proxy) = proxy else {
            return;
        };
        self.proxy_enabled = true;
        self.proxy_type = proxy.proxy_type;
        set_input(&self.proxy_host_input, proxy.host, window, cx);
        set_input(&self.proxy_port_input, proxy.port.to_string(), window, cx);
        set_input(
            &self.proxy_username_input,
            proxy.username.unwrap_or_default(),
            window,
            cx,
        );
        set_input(
            &self.proxy_password_input,
            proxy.password.unwrap_or_default(),
            window,
            cx,
        );
    }
}

fn set_input(
    input: &gpui::Entity<InputState>,
    value: String,
    window: &mut Window,
    cx: &mut Context<RemoteDesktopFormWindow>,
) {
    input.update(cx, |state, cx| state.set_value(value, window, cx));
}

pub(super) fn proxy_error_message(field: &'static str) -> String {
    let label = match field {
        "proxy_host" => t!("RemoteDesktopForm.label_proxy_host"),
        "proxy_port" => t!("RemoteDesktopForm.label_proxy_port"),
        "proxy_username" => t!("RemoteDesktopForm.label_proxy_username"),
        _ => t!("RemoteDesktopForm.label_proxy"),
    };
    t!("RemoteDesktopForm.proxy_invalid", field = label).to_string()
}

#[cfg(test)]
mod tests {
    use one_core::storage::ProxyType;

    use super::build_proxy_config;

    #[test]
    fn disabled_proxy_builds_none() {
        assert!(
            build_proxy_config(false, ProxyType::Socks5, "", "", "", "")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn enabled_proxy_validates_required_fields() {
        assert_eq!(
            "proxy_host",
            build_proxy_config(true, ProxyType::Socks5, "", "1080", "", "").unwrap_err()
        );
        assert_eq!(
            "proxy_port",
            build_proxy_config(true, ProxyType::Socks5, "proxy", "0", "", "").unwrap_err()
        );
        assert_eq!(
            "proxy_username",
            build_proxy_config(true, ProxyType::Http, "proxy", "8080", "", "secret").unwrap_err()
        );
    }

    #[test]
    fn enabled_proxy_builds_trimmed_config() {
        let proxy = build_proxy_config(
            true,
            ProxyType::Http,
            " proxy.example.com ",
            "8080",
            " alice ",
            " secret ",
        )
        .unwrap()
        .unwrap();

        assert_eq!(ProxyType::Http, proxy.proxy_type);
        assert_eq!("proxy.example.com", proxy.host);
        assert_eq!(8080, proxy.port);
        assert_eq!(Some("alice".to_string()), proxy.username);
        assert_eq!(Some(" secret ".to_string()), proxy.password);
    }
}
