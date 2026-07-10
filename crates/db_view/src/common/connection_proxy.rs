use db::plugin_manifest::{FormValueCondition, FormVisibilityRule};
use one_core::storage::{ProxyConfig, ProxyType};
use rust_i18n::t;

use crate::common::db_connection_form::{DbFormConfig, FormField, FormFieldType, TabGroup};

const PROXY_ENABLED: &str = "proxy_enabled";
const PROXY_TYPE: &str = "proxy_type";
const PROXY_HOST: &str = "proxy_host";
const PROXY_PORT: &str = "proxy_port";
const PROXY_USERNAME: &str = "proxy_username";
const PROXY_PASSWORD: &str = "proxy_password";

const PROXY_FIELDS: &[&str] = &[
    PROXY_ENABLED,
    PROXY_TYPE,
    PROXY_HOST,
    PROXY_PORT,
    PROXY_USERNAME,
    PROXY_PASSWORD,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProxyValidationError {
    field: &'static str,
}

impl ProxyValidationError {
    pub(crate) fn field(self) -> Option<&'static str> {
        Some(self.field)
    }
}

pub(crate) fn is_proxy_field(name: &str) -> bool {
    PROXY_FIELDS.contains(&name)
}

pub(crate) fn with_proxy_tab(mut config: DbFormConfig) -> DbFormConfig {
    if !supports_network_proxy(&config) || config.tab_groups.iter().any(|tab| tab.name == "proxy") {
        return config;
    }
    let index = config
        .tab_groups
        .iter()
        .position(|tab| tab.name == "notes")
        .unwrap_or(config.tab_groups.len());
    config.tab_groups.insert(index, proxy_tab_group());
    config
}

pub(crate) fn build_proxy_config(
    enabled: bool,
    proxy_type: &str,
    host: &str,
    port: &str,
    username: &str,
    password: &str,
) -> Result<Option<ProxyConfig>, ProxyValidationError> {
    if !enabled {
        return Ok(None);
    }
    let host = required_value(PROXY_HOST, host)?;
    let port = port
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port > 0)
        .ok_or(ProxyValidationError { field: PROXY_PORT })?;
    let username = optional_value(username);
    let password = optional_secret(password);
    if username.is_none() && password.is_some() {
        return Err(ProxyValidationError {
            field: PROXY_USERNAME,
        });
    }
    Ok(Some(ProxyConfig {
        proxy_type: match proxy_type.trim().to_ascii_lowercase().as_str() {
            "http" => ProxyType::Http,
            _ => ProxyType::Socks5,
        },
        host,
        port,
        username,
        password,
    }))
}

fn supports_network_proxy(config: &DbFormConfig) -> bool {
    let has_field = |name: &str| {
        config
            .tab_groups
            .iter()
            .flat_map(|tab| &tab.fields)
            .any(|field| field.name == name)
    };
    has_field("host") && has_field("port")
}

fn proxy_tab_group() -> TabGroup {
    TabGroup::new("proxy", t!("ConnectionForm.proxy")).fields(vec![
        FormField::new(
            PROXY_ENABLED,
            t!("ConnectionForm.proxy_enabled"),
            FormFieldType::Checkbox,
        )
        .optional()
        .default("false"),
        proxy_field(FormField::new(
            PROXY_TYPE,
            t!("ConnectionForm.proxy_type"),
            FormFieldType::Select,
        ))
        .default("socks5")
        .options(vec![
            ("socks5".to_string(), "SOCKS5".to_string()),
            ("http".to_string(), "HTTP CONNECT".to_string()),
        ]),
        proxy_field(FormField::new(
            PROXY_HOST,
            t!("ConnectionForm.proxy_host"),
            FormFieldType::Text,
        ))
        .placeholder("127.0.0.1"),
        proxy_field(FormField::new(
            PROXY_PORT,
            t!("ConnectionForm.proxy_port"),
            FormFieldType::Number,
        ))
        .default("1080")
        .placeholder("1080"),
        proxy_field(FormField::new(
            PROXY_USERNAME,
            t!("ConnectionForm.proxy_username"),
            FormFieldType::Text,
        ))
        .optional(),
        proxy_field(FormField::new(
            PROXY_PASSWORD,
            t!("ConnectionForm.proxy_password"),
            FormFieldType::Password,
        ))
        .optional(),
    ])
}

fn proxy_field(mut field: FormField) -> FormField {
    field.visible_when.push(FormVisibilityRule {
        when_field: PROXY_ENABLED.to_string(),
        condition: FormValueCondition::Equals("true".to_string()),
    });
    field
}

fn required_value(field: &'static str, value: &str) -> Result<String, ProxyValidationError> {
    optional_value(value).ok_or(ProxyValidationError { field })
}

fn optional_value(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn optional_secret(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use one_core::storage::ProxyType;

    use super::{build_proxy_config, with_proxy_tab};
    use crate::common::db_connection_form::DbFormConfig;

    #[test]
    fn network_database_form_adds_proxy_tab_before_notes() {
        let config = with_proxy_tab(DbFormConfig::mysql());
        let names = config
            .tab_groups
            .iter()
            .map(|tab| tab.name.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"proxy"));
        assert!(
            names.iter().position(|name| *name == "proxy")
                < names.iter().position(|name| *name == "notes")
        );
    }

    #[test]
    fn file_database_form_does_not_add_proxy_tab() {
        let config = with_proxy_tab(DbFormConfig::sqlite());

        assert!(config.tab_groups.iter().all(|tab| tab.name != "proxy"));
    }

    #[test]
    fn disabled_proxy_builds_none() {
        assert!(
            build_proxy_config(false, "socks5", "", "", "", "")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn enabled_proxy_requires_host_port_and_username_for_password() {
        assert_eq!(
            Some("proxy_host"),
            build_proxy_config(true, "socks5", "", "1080", "", "")
                .unwrap_err()
                .field()
        );
        assert_eq!(
            Some("proxy_port"),
            build_proxy_config(true, "socks5", "proxy", "0", "", "")
                .unwrap_err()
                .field()
        );
        assert_eq!(
            Some("proxy_username"),
            build_proxy_config(true, "http", "proxy", "8080", "", "secret")
                .unwrap_err()
                .field()
        );
    }

    #[test]
    fn enabled_proxy_builds_trimmed_config() {
        let proxy = build_proxy_config(
            true,
            "http",
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
