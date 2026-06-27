use crate::settings::ai_chat_agent_json::acp_agents_json_item;
use gpui::{App, SharedString};
use gpui_component::setting::{SettingField, SettingGroup, SettingItem};
use one_core::settings::{
    AcpAgentSettings, AcpAgentTransportSettings, AiChatSettings, AppSettings,
};
use rust_i18n::t;
use serde_json::Value;
use std::collections::BTreeMap;

pub fn ai_chat_setting_group(default_settings: &AiChatSettings) -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.General.AiChat.group_title"))
        .items(vec![
            enabled_item(default_settings),
            name_item(default_settings),
            transport_item(default_settings),
            http_url_item(default_settings),
            stdio_command_item(default_settings),
            stdio_args_item(default_settings),
            stdio_env_item(default_settings),
            acp_agents_json_item(default_settings),
        ])
}

fn enabled_item(default_settings: &AiChatSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.AiChat.acp_enabled"),
        SettingField::checkbox(
            |cx: &App| primary_agent(&AppSettings::global(cx).ai_chat).enabled,
            |val: bool, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    primary_agent_mut(settings).enabled = val;
                });
            },
        )
        .default_value(primary_agent(default_settings).enabled),
    )
    .description(t!("Settings.General.AiChat.acp_enabled_desc").to_string())
}

fn name_item(default_settings: &AiChatSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.AiChat.acp_name"),
        SettingField::input(
            |cx: &App| SharedString::from(primary_agent(&AppSettings::global(cx).ai_chat).name),
            |val: SharedString, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    primary_agent_mut(settings).name = val.trim().to_string();
                });
            },
        )
        .default_value(SharedString::from(primary_agent(default_settings).name)),
    )
    .description(t!("Settings.General.AiChat.acp_name_desc").to_string())
}

fn transport_item(default_settings: &AiChatSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.AiChat.acp_transport"),
        SettingField::dropdown(
            transport_options(),
            |cx: &App| primary_transport_kind(&AppSettings::global(cx).ai_chat).into(),
            |val: SharedString, cx: &mut App| set_primary_transport_kind(val.as_ref(), cx),
        )
        .default_value(primary_transport_kind(default_settings)),
    )
    .description(t!("Settings.General.AiChat.acp_transport_desc").to_string())
}

fn http_url_item(default_settings: &AiChatSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.AiChat.acp_http_url"),
        SettingField::input(primary_http_url, |val: SharedString, cx: &mut App| {
            AppSettings::update_and_save(cx, |settings| {
                primary_agent_mut(settings).transport = AcpAgentTransportSettings::Http {
                    url: val.trim().to_string(),
                };
            });
        })
        .default_value(default_http_url(default_settings)),
    )
    .description(t!("Settings.General.AiChat.acp_http_url_desc").to_string())
}

fn stdio_command_item(default_settings: &AiChatSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.AiChat.acp_stdio_command"),
        SettingField::input(primary_stdio_command, |val: SharedString, cx: &mut App| {
            update_primary_stdio(cx, |transport| {
                transport.command = val.trim().to_string();
            });
        })
        .default_value(default_stdio_command(default_settings)),
    )
    .description(t!("Settings.General.AiChat.acp_stdio_command_desc").to_string())
}

fn stdio_args_item(default_settings: &AiChatSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.AiChat.acp_stdio_args"),
        SettingField::input(
            primary_stdio_args_json,
            |val: SharedString, cx: &mut App| {
                if let Some(args) = parse_json_string_array(val.as_ref()) {
                    update_primary_stdio(cx, |transport| transport.args = args);
                }
            },
        )
        .default_value(default_stdio_args_json(default_settings)),
    )
    .description(t!("Settings.General.AiChat.acp_stdio_args_desc").to_string())
}

fn stdio_env_item(default_settings: &AiChatSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.AiChat.acp_stdio_env"),
        SettingField::input(primary_stdio_env_json, |val: SharedString, cx: &mut App| {
            if let Some(env) = parse_json_string_object(val.as_ref()) {
                update_primary_stdio(cx, |transport| transport.env = env);
            }
        })
        .default_value(default_stdio_env_json(default_settings)),
    )
    .description(t!("Settings.General.AiChat.acp_stdio_env_desc").to_string())
}

fn transport_options() -> Vec<(SharedString, SharedString)> {
    vec![
        (
            "http".into(),
            t!("Settings.General.AiChat.acp_transport_http").into(),
        ),
        (
            "stdio".into(),
            t!("Settings.General.AiChat.acp_transport_stdio").into(),
        ),
    ]
}

fn primary_agent(settings: &AiChatSettings) -> AcpAgentSettings {
    settings.acp_agents.first().cloned().unwrap_or_default()
}

fn primary_agent_mut(settings: &mut AppSettings) -> &mut AcpAgentSettings {
    if settings.ai_chat.acp_agents.is_empty() {
        settings
            .ai_chat
            .acp_agents
            .push(AcpAgentSettings::default());
    }
    settings
        .ai_chat
        .acp_agents
        .first_mut()
        .expect("agent inserted")
}

fn primary_transport_kind(settings: &AiChatSettings) -> &'static str {
    primary_agent(settings).transport.kind()
}

fn set_primary_transport_kind(kind: &str, cx: &mut App) {
    AppSettings::update_and_save(cx, |settings| {
        primary_agent_mut(settings).transport = AcpAgentTransportSettings::default_for_kind(kind);
    });
}

fn primary_http_url(cx: &App) -> SharedString {
    default_http_url(&AppSettings::global(cx).ai_chat).into()
}

fn default_http_url(settings: &AiChatSettings) -> String {
    match primary_agent(settings).transport {
        AcpAgentTransportSettings::Http { url } => url,
        AcpAgentTransportSettings::Stdio { .. } => String::new(),
    }
}

fn primary_stdio_command(cx: &App) -> SharedString {
    default_stdio_command(&AppSettings::global(cx).ai_chat).into()
}

fn default_stdio_command(settings: &AiChatSettings) -> String {
    match primary_agent(settings).transport {
        AcpAgentTransportSettings::Stdio { command, .. } => command,
        AcpAgentTransportSettings::Http { .. } => String::new(),
    }
}

fn primary_stdio_args_json(cx: &App) -> SharedString {
    default_stdio_args_json(&AppSettings::global(cx).ai_chat).into()
}

fn default_stdio_args_json(settings: &AiChatSettings) -> String {
    match primary_agent(settings).transport {
        AcpAgentTransportSettings::Stdio { args, .. } => format_json_array(&args),
        AcpAgentTransportSettings::Http { .. } => "[]".to_string(),
    }
}

fn primary_stdio_env_json(cx: &App) -> SharedString {
    default_stdio_env_json(&AppSettings::global(cx).ai_chat).into()
}

fn default_stdio_env_json(settings: &AiChatSettings) -> String {
    match primary_agent(settings).transport {
        AcpAgentTransportSettings::Stdio { env, .. } => format_json_object(&env),
        AcpAgentTransportSettings::Http { .. } => "{}".to_string(),
    }
}

#[derive(Default)]
struct StdioTransport {
    command: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

fn update_primary_stdio(cx: &mut App, update: impl FnOnce(&mut StdioTransport)) {
    AppSettings::update_and_save(cx, |settings| {
        let agent = primary_agent_mut(settings);
        let mut transport = stdio_transport(&agent.transport);
        update(&mut transport);
        agent.transport = AcpAgentTransportSettings::Stdio {
            command: transport.command,
            args: transport.args,
            env: transport.env,
        };
    });
}

fn stdio_transport(transport: &AcpAgentTransportSettings) -> StdioTransport {
    match transport {
        AcpAgentTransportSettings::Stdio { command, args, env } => StdioTransport {
            command: command.clone(),
            args: args.clone(),
            env: env.clone(),
        },
        AcpAgentTransportSettings::Http { .. } => StdioTransport::default(),
    }
}

fn parse_json_string_array(value: &str) -> Option<Vec<String>> {
    serde_json::from_str::<Vec<String>>(value).ok()
}

fn parse_json_string_object(value: &str) -> Option<Vec<(String, String)>> {
    let parsed = serde_json::from_str::<serde_json::Map<String, Value>>(value).ok()?;
    Some(
        parsed
            .into_iter()
            .filter_map(|(name, value)| value.as_str().map(|value| (name, value.to_string())))
            .collect(),
    )
}

fn format_json_array(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string())
}

fn format_json_object(values: &[(String, String)]) -> String {
    let object = values.iter().cloned().collect::<BTreeMap<_, _>>();
    serde_json::to_string(&object).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
pub(super) fn ai_chat_setting_item_ids() -> Vec<&'static str> {
    vec![
        "acp_enabled",
        "acp_name",
        "acp_transport",
        "acp_http_url",
        "acp_stdio_command",
        "acp_stdio_args",
        "acp_stdio_env",
        "acp_agents_json",
    ]
}

#[cfg(test)]
pub(super) fn ai_chat_transport_option_values() -> Vec<String> {
    transport_options()
        .into_iter()
        .map(|(value, _)| value.to_string())
        .collect()
}

#[cfg(test)]
pub(super) fn parse_stdio_env_json_for_test(value: &str) -> Option<Vec<(String, String)>> {
    parse_json_string_object(value)
}
