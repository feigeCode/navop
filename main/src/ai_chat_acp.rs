use ai_chat_view::{
    AcpAgentConfig, AcpAgentEntry, AcpAuthConfig, AcpAuthMethodConfig, AcpConfigDiagnostic,
    AcpTimeoutConfig, AcpTransport, set_acp_agent_config_provider,
};
use extension_runtime::extension::{
    AcpAgentExtensionAgent, AcpAgentExtensionProvider, AcpAgentExtensionTransport, ExtensionKind,
    ExtensionRegistry,
};
use gpui::App;
use std::collections::HashSet;
use std::time::Duration;

mod user_config;

use user_config::{AcpResolvedAgentOverride, load_user_config, resolve_override};

pub fn init(cx: &mut App) {
    set_acp_agent_config_provider(cx, |_cx| {
        let mut entries = acp_agent_entries_from_registry()?;
        normalize_acp_agent_entry_ids(&mut entries);
        Ok(entries)
    });
}

fn acp_agent_entries_from_registry() -> anyhow::Result<Vec<AcpAgentEntry>> {
    let Some(registry) = ExtensionRegistry::global() else {
        return Ok(Vec::new());
    };
    let registry = registry
        .read()
        .map_err(|err| anyhow::anyhow!("extension registry lock poisoned: {err}"))?;
    let root = registry.root_for(ExtensionKind::AcpAgent);
    let user_config = load_user_config()?;
    let agents = AcpAgentExtensionProvider::load_agents_from_root(&root)?;
    Ok(acp_agent_entries_from_agents(
        &agents,
        &user_config,
        |name| std::env::var(name).ok(),
    ))
}

fn acp_agent_entries_from_agents(
    agents: &[AcpAgentExtensionAgent],
    user_config: &user_config::AcpUserConfig,
    lookup: impl Fn(&str) -> Option<String>,
) -> Vec<AcpAgentEntry> {
    agents
        .iter()
        .filter_map(|agent| {
            let mut config = acp_agent_config_from_extension_agent(agent)?;
            let Some(override_config) = user_config.agents.get(config.id.as_ref()) else {
                return Some(AcpAgentEntry::ready(config));
            };
            Some(match resolve_override(override_config, &lookup) {
                Ok(resolved) => {
                    apply_user_override(&mut config, resolved);
                    AcpAgentEntry::ready(config)
                }
                Err(error) => AcpAgentEntry::invalid(
                    config.id,
                    config.name,
                    AcpConfigDiagnostic::new(error.to_string()),
                ),
            })
        })
        .collect()
}

fn acp_agent_config_from_extension_agent(agent: &AcpAgentExtensionAgent) -> Option<AcpAgentConfig> {
    let id = extension_agent_config_id(agent)?;
    let name = non_empty_or_else(&agent.name, || "ACP Agent".to_string());
    match &agent.transport {
        AcpAgentExtensionTransport::Http { url } => {
            non_empty_trimmed(url).map(|url| AcpAgentConfig::new_http(id, name, url.to_string()))
        }
        AcpAgentExtensionTransport::Stdio { command, args, env } => {
            non_empty_trimmed(command).map(|command| {
                AcpAgentConfig::new(
                    id,
                    name,
                    agent
                        .manifest_dir
                        .join(command)
                        .components()
                        .collect::<std::path::PathBuf>()
                        .display()
                        .to_string(),
                )
                .with_args(args.clone())
                .with_env(env.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            })
        }
    }
    .map(|config| {
        config
            .with_auth(extension_auth_config(agent))
            .with_timeouts(extension_timeout_config(agent))
    })
}

fn extension_auth_config(agent: &AcpAgentExtensionAgent) -> AcpAuthConfig {
    AcpAuthConfig {
        requested_method: None,
        preferred_method: agent.auth.preferred_method.clone(),
        allow_unauthenticated_fallback: agent.auth.allow_unauthenticated_fallback,
        methods: agent
            .auth
            .methods
            .iter()
            .map(|method| AcpAuthMethodConfig {
                id: method.id.clone(),
                env_any: method.env_any.clone(),
                env_all: method.env_all.clone(),
                interactive: method.interactive,
            })
            .collect(),
    }
}

fn extension_timeout_config(agent: &AcpAgentExtensionAgent) -> AcpTimeoutConfig {
    AcpTimeoutConfig {
        connect: Duration::from_secs(agent.timeouts.connect_seconds),
        authenticate: Duration::from_secs(agent.timeouts.authenticate_seconds),
        prompt: Duration::from_secs(agent.timeouts.prompt_seconds),
    }
}

fn apply_user_override(config: &mut AcpAgentConfig, user: AcpResolvedAgentOverride) {
    config.auth.requested_method = user.auth_method;
    if let Some(args) = user.args
        && let AcpTransport::Stdio {
            args: current_args, ..
        } = &mut config.transport
    {
        *current_args = args;
    }
    if let AcpTransport::Stdio { env, .. } = &mut config.transport {
        merge_env(env, user.env);
    }
    user.timeouts.apply(&mut config.timeouts);
}

fn merge_env(
    env: &mut Vec<(String, String)>,
    additions: std::collections::BTreeMap<String, String>,
) {
    for (name, value) in additions {
        if let Some((_, existing)) = env.iter_mut().find(|(key, _)| key == &name) {
            *existing = value;
        } else {
            env.push((name, value));
        }
    }
}

fn extension_agent_config_id(agent: &AcpAgentExtensionAgent) -> Option<String> {
    let extension_id = non_empty_trimmed(&agent.extension_id)?;
    let agent_id = non_empty_trimmed(&agent.id)?;
    Some(format!("{extension_id}.{agent_id}"))
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

#[cfg(test)]
fn normalize_acp_agent_config_ids(configs: &mut [AcpAgentConfig]) {
    let mut used = HashSet::new();
    for (index, config) in configs.iter_mut().enumerate() {
        let base = non_empty_or_else(config.id.as_ref(), || format!("acp-agent-{}", index + 1));
        config.id = unique_id(base, &mut used).into();
    }
}

fn normalize_acp_agent_entry_ids(entries: &mut [AcpAgentEntry]) {
    let mut used = HashSet::new();
    for (index, entry) in entries.iter_mut().enumerate() {
        let base = non_empty_or_else(entry.id.as_ref(), || format!("acp-agent-{}", index + 1));
        let id = unique_id(base, &mut used);
        entry.id = id.clone().into();
        if let Some(config) = &mut entry.config {
            config.id = id.into();
        }
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

#[cfg(test)]
mod tests;
