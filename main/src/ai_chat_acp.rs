use ai_chat_view::{AcpAgentConfig, set_acp_agent_config_provider};
use extension_runtime::extension::{
    AcpAgentExtensionAgent, AcpAgentExtensionProvider, AcpAgentExtensionTransport, ExtensionKind,
    ExtensionRegistry,
};
use gpui::App;
use std::collections::HashSet;

pub fn init(cx: &mut App) {
    set_acp_agent_config_provider(cx, |_cx| {
        let mut configs = acp_agent_configs_from_registry()?;
        normalize_acp_agent_config_ids(&mut configs);
        Ok(configs)
    });
}

fn acp_agent_configs_from_registry() -> anyhow::Result<Vec<AcpAgentConfig>> {
    let Some(registry) = ExtensionRegistry::global() else {
        return Ok(Vec::new());
    };
    let registry = registry
        .read()
        .map_err(|err| anyhow::anyhow!("extension registry lock poisoned: {err}"))?;
    let root = registry.root_for(ExtensionKind::AcpAgent);
    Ok(AcpAgentExtensionProvider::load_agents_from_root(&root)?
        .iter()
        .filter_map(acp_agent_config_from_extension_agent)
        .collect())
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

#[cfg(test)]
mod tests;
