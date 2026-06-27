use ai_chat_view::{AcpAgentConfig, set_acp_agent_config_provider};
use gpui::App;
use one_core::settings::{AcpAgentSettings, AcpAgentTransportSettings, AppSettings};
use serde_json::Value;
use std::collections::HashSet;

const HTTP_URL_ENV: &str = "ONETCLI_ACP_HTTP_URL";
const STDIO_COMMAND_ENV: &str = "ONETCLI_ACP_STDIO_COMMAND";
const STDIO_ARGS_ENV: &str = "ONETCLI_ACP_STDIO_ARGS";
const STDIO_ENV_ENV: &str = "ONETCLI_ACP_STDIO_ENV";

pub fn init(cx: &mut App) {
    set_acp_agent_config_provider(cx, |cx| {
        let mut configs = acp_agent_configs_from_settings(AppSettings::global(cx));
        configs.extend(acp_agent_configs_from_env());
        normalize_acp_agent_config_ids(&mut configs);
        Ok(configs)
    });
}

fn acp_agent_configs_from_settings(settings: &AppSettings) -> Vec<AcpAgentConfig> {
    settings
        .ai_chat
        .acp_agents
        .iter()
        .enumerate()
        .filter_map(|(index, agent)| acp_agent_config_from_settings(index, agent))
        .collect()
}

fn acp_agent_config_from_settings(
    index: usize,
    agent: &AcpAgentSettings,
) -> Option<AcpAgentConfig> {
    if !agent.enabled {
        return None;
    }

    let id = non_empty_or_else(&agent.id, || format!("settings-acp-{}", index + 1));
    let name = non_empty_or_else(&agent.name, || "ACP Agent".to_string());
    match &agent.transport {
        AcpAgentTransportSettings::Http { url } => {
            non_empty_trimmed(url).map(|url| AcpAgentConfig::new_http(id, name, url.to_string()))
        }
        AcpAgentTransportSettings::Stdio { command, args, env } => {
            non_empty_trimmed(command).map(|command| {
                AcpAgentConfig::new(id, name, command.to_string())
                    .with_args(args.clone())
                    .with_env(env.clone())
            })
        }
    }
}

fn non_empty_or_else(value: &str, fallback: impl FnOnce() -> String) -> String {
    non_empty_trimmed(value)
        .map(ToString::to_string)
        .unwrap_or_else(fallback)
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn acp_agent_configs_from_env() -> Vec<AcpAgentConfig> {
    let mut configs = Vec::new();
    if let Ok(url) = std::env::var(HTTP_URL_ENV)
        && !url.trim().is_empty()
    {
        configs.push(AcpAgentConfig::new_http(
            "env-http",
            "ACP HTTP",
            url.trim().to_string(),
        ));
    }
    if let Ok(command) = std::env::var(STDIO_COMMAND_ENV)
        && !command.trim().is_empty()
    {
        configs.push(
            AcpAgentConfig::new("env-stdio", "ACP Stdio", command.trim().to_string())
                .with_args(parse_json_string_array(STDIO_ARGS_ENV))
                .with_env(parse_json_string_object(STDIO_ENV_ENV)),
        );
    }
    configs
}

fn normalize_acp_agent_config_ids(configs: &mut [AcpAgentConfig]) {
    let mut used = HashSet::new();
    for (index, config) in configs.iter_mut().enumerate() {
        let base = non_empty_or_else(config.id.as_ref(), || format!("acp-agent-{}", index + 1));
        config.id = unique_id(base, &mut used).into();
    }
}

fn unique_id(base: String, used: &mut HashSet<String>) -> String {
    if used.insert(base.clone()) {
        return base;
    }
    let mut suffix = 2;
    loop {
        let candidate = format!("{base}-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn parse_json_string_array(name: &str) -> Vec<String> {
    let Ok(value) = std::env::var(name) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&value).unwrap_or_else(|error| {
        tracing::warn!(env = name, %error, "Invalid ACP JSON array env var");
        Vec::new()
    })
}

fn parse_json_string_object(name: &str) -> Vec<(String, String)> {
    let Ok(value) = std::env::var(name) else {
        return Vec::new();
    };
    let parsed = serde_json::from_str::<serde_json::Map<String, Value>>(&value);
    match parsed {
        Ok(object) => object
            .into_iter()
            .filter_map(|(name, value)| value.as_str().map(|value| (name, value.to_string())))
            .collect(),
        Err(error) => {
            tracing::warn!(env = name, %error, "Invalid ACP JSON object env var");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests;
