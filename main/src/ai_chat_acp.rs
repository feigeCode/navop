use ai_chat_view::{
    AcpAgentConfig, AcpAgentEntry, AcpAgentSource, AcpAuthConfig, AcpAuthMethodConfig,
    AcpConfigDiagnostic, AcpProbeRecord, AcpTimeoutConfig, AcpTransport, probe_fingerprint,
    set_acp_agent_config_provider,
};
use anyhow::bail;
use extension_runtime::extension::{
    AcpAgentExtensionAgent, AcpAgentExtensionProvider, AcpAgentExtensionTransport, ExtensionKind,
    ExtensionRegistry,
};
use gpui::App;
use one_core::settings::AppSettings;
use std::collections::HashSet;
use std::time::Duration;

mod user_config;

use user_config::{
    AcpResolvedAgentOverride, AcpUserConfig, load_user_config, resolve_override,
    resolve_user_agent, save_user_config, user_agent_diagnostic,
};

pub fn init(cx: &mut App) {
    set_acp_agent_config_provider(cx, |_cx| current_agent_entries());
}

/// 当前生效的 agent 列表。
///
/// 聊天切换器与设置页都走这里，保证两边看到的 id、启用状态、诊断完全一致。
pub fn current_agent_entries() -> anyhow::Result<Vec<AcpAgentEntry>> {
    let mut entries = acp_agent_entries_from_registry()?;
    normalize_acp_agent_entry_ids(&mut entries);
    Ok(entries)
}

/// 设置页展示用的一行。
#[derive(Clone, Debug)]
pub struct AcpAgentRow {
    pub id: String,
    pub name: String,
    pub source: AcpAgentSource,
    pub enabled: bool,
    /// 启动命令（stdio 传输）。
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// 配置本身的问题（例如扩展声明了 http 传输）。
    pub diagnostic: Option<String>,
    /// 启动指纹；用来在落盘缓存里查这条配置的检测结论。
    pub fingerprint: Option<String>,
}

/// 读出设置页要展示的行。
pub fn settings_rows() -> anyhow::Result<Vec<AcpAgentRow>> {
    Ok(current_agent_entries()?
        .into_iter()
        .map(|entry| {
            let (command, args, env) = match entry.config.as_ref().map(|config| &config.transport) {
                Some(AcpTransport::Stdio { command, args, env }) => {
                    (command.clone(), args.clone(), env.clone())
                }
                None => (String::new(), Vec::new(), Vec::new()),
            };
            AcpAgentRow {
                id: entry.id.to_string(),
                name: entry.name.to_string(),
                source: entry.source,
                enabled: entry.enabled,
                command,
                args,
                env,
                diagnostic: entry.diagnostic.map(|diagnostic| diagnostic.message),
                fingerprint: entry.config.as_ref().map(probe_fingerprint),
            }
        })
        .collect())
}

/// 用户新增/修改一条自定义 agent。
#[derive(Clone, Debug)]
pub struct AcpUserAgentSpec {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// 新增或更新一条用户自定义 agent。
pub fn upsert_user_agent(spec: AcpUserAgentSpec) -> anyhow::Result<()> {
    let id = spec.id.trim();
    if id.is_empty() {
        bail!("agent id 不能为空");
    }
    if spec.command.trim().is_empty() {
        bail!("启动命令不能为空");
    }
    let mut config = load_user_config()?;
    config.agents.insert(
        id.to_string(),
        user_config::AcpUserAgentConfig {
            name: Some(spec.name.trim().to_string()).filter(|name| !name.is_empty()),
            command: Some(spec.command.trim().to_string()),
            args: Some(spec.args.clone()),
            env: spec.env.iter().cloned().collect(),
            enabled: None,
            auth_method: None,
            timeouts: Default::default(),
        },
    );
    save_user_config(&config)
}

/// 删除一条**用户自定义**的 agent；扩展提供的 agent 走 [`reset_agent_override`]。
pub fn remove_user_agent(id: &str) -> anyhow::Result<()> {
    let mut config = load_user_config()?;
    let is_user_defined = config
        .agents
        .get(id)
        .is_some_and(|agent| agent.command.is_some());
    if !is_user_defined {
        bail!("{id} 不是用户自定义的 agent");
    }
    config.agents.remove(id);
    save_user_config(&config)
}

/// 保存扩展提供的 agent 的参数/环境变量覆盖。
pub fn save_agent_override(
    id: &str,
    args: Vec<String>,
    env: Vec<(String, String)>,
) -> anyhow::Result<()> {
    let mut config = load_user_config()?;
    let entry = config.agents.entry(id.to_string()).or_default();
    if entry.command.is_some() {
        bail!("{id} 是用户自定义的 agent，请用 upsert_user_agent 保存");
    }
    entry.args = Some(args);
    entry.env = env.into_iter().collect();
    if entry.is_default_entry() {
        config.agents.remove(id);
    }
    save_user_config(&config)
}

/// 清掉某条目的覆盖/启用记录，回到扩展声明的默认状态。
pub fn reset_agent_override(id: &str) -> anyhow::Result<()> {
    let mut config = load_user_config()?;
    config.agents.remove(id);
    save_user_config(&config)
}

/// 启用/停用某个 agent。停用后聊天切换器不再展示它。
pub fn set_agent_enabled(id: &str, enabled: bool) -> anyhow::Result<()> {
    let mut config = load_user_config()?;
    let entry = config.agents.entry(id.to_string()).or_default();
    if entry.command.is_none() && enabled {
        // 扩展 agent 默认就是启用的，不需要留一条空记录。
        entry.enabled = None;
    } else {
        entry.enabled = Some(enabled);
    }
    if entry.is_default_entry() {
        config.agents.remove(id);
    }
    save_user_config(&config)
}

/// 记录「当前使用」的 agent（设置页直接切换）。
pub fn set_active_agent(cx: &mut App, id: Option<&str>) {
    let id = id.map(ToString::to_string);
    AppSettings::update_and_save(cx, |settings| {
        settings.ai_chat.last_acp_agent_id = id;
    });
}

/// 同步执行一次探测并写入落盘缓存（在后台线程调用）。
pub fn probe_and_cache(config: &AcpAgentConfig, handle: tokio::runtime::Handle) -> AcpProbeRecord {
    let probe = ai_chat_view::probe_agent_blocking(config, handle);
    AcpProbeRecord::new(probe_fingerprint(config), probe)
}

fn acp_agent_entries_from_registry() -> anyhow::Result<Vec<AcpAgentEntry>> {
    let user_config = load_user_config()?;
    let lookup = |name: &str| std::env::var(name).ok();
    let mut entries = ExtensionRegistry::global()
        .map(|registry| {
            let registry = registry
                .read()
                .map_err(|err| anyhow::anyhow!("extension registry lock poisoned: {err}"))?;
            let root = registry.root_for(ExtensionKind::AcpAgent);
            let agents = AcpAgentExtensionProvider::load_agents_from_root(&root)?;
            Ok::<_, anyhow::Error>(acp_agent_entries_from_agents(
                &agents,
                &user_config,
                &lookup,
            ))
        })
        .transpose()?
        .unwrap_or_default();
    entries.extend(user_defined_agent_entries(&user_config, &lookup));
    Ok(dedupe_agent_entries(entries))
}

/// 用户自己定义的 agent（配置里带 `command` 的条目）。
fn user_defined_agent_entries(
    config: &AcpUserConfig,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Vec<AcpAgentEntry> {
    config
        .agents
        .iter()
        // 没有 command 的条目只是对扩展 agent 的覆盖，不在这里生成新 agent。
        .filter(|(_, agent)| agent.command.is_some())
        .map(|(id, agent)| {
            let enabled = agent.is_enabled();
            match resolve_user_agent(id, agent, lookup) {
                Ok(config) => AcpAgentEntry::ready(config)
                    .with_source(AcpAgentSource::User)
                    .with_enabled(enabled),
                Err(error) => AcpAgentEntry::invalid(
                    id.clone(),
                    user_agent_display_name(id, agent),
                    user_agent_diagnostic(&error),
                )
                .with_source(AcpAgentSource::User)
                .with_enabled(enabled),
            }
        })
        .collect()
}

fn user_agent_display_name(id: &str, agent: &user_config::AcpUserAgentConfig) -> String {
    agent
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(id)
        .to_string()
}

fn dedupe_agent_entries(entries: Vec<AcpAgentEntry>) -> Vec<AcpAgentEntry> {
    let mut used = HashSet::new();
    entries
        .into_iter()
        .map(|mut entry| {
            let id = unique_id(entry.id.to_string(), &mut used);
            entry.id = id.clone().into();
            if let Some(config) = &mut entry.config {
                config.id = id.into();
            }
            entry
        })
        .collect()
}

fn acp_agent_entries_from_agents(
    agents: &[AcpAgentExtensionAgent],
    user_config: &AcpUserConfig,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Vec<AcpAgentEntry> {
    agents
        .iter()
        .filter_map(|agent| {
            let id = extension_agent_config_id(agent)?;
            let name = non_empty_or_else(&agent.name, || "ACP Agent".to_string());
            let mut config = match acp_agent_config_from_extension_agent(agent) {
                Ok(config) => config,
                Err(diagnostic) => return Some(AcpAgentEntry::invalid(id, name, diagnostic)),
            };
            let Some(override_config) = user_config.agents.get(config.id.as_ref()) else {
                return Some(AcpAgentEntry::ready(config));
            };
            let enabled = override_config.is_enabled();
            Some(match resolve_override(override_config, lookup) {
                Ok(resolved) => {
                    apply_user_override(&mut config, resolved);
                    AcpAgentEntry::ready(config).with_enabled(enabled)
                }
                Err(error) => AcpAgentEntry::invalid(
                    config.id,
                    config.name,
                    AcpConfigDiagnostic::new(error.to_string()),
                )
                .with_enabled(enabled),
            })
        })
        .collect()
}

/// 把扩展声明的 agent 转成运行期配置。
///
/// 只接受 stdio：协议 SDK 2.x 的 `AcpAgent` 只负责拉起子进程，HTTP MCP 形态已从
/// SDK 移除。声明 http 的扩展在这里就被判为不可用并带出诊断，而不是等到连接时才失败。
fn acp_agent_config_from_extension_agent(
    agent: &AcpAgentExtensionAgent,
) -> Result<AcpAgentConfig, AcpConfigDiagnostic> {
    let AcpAgentExtensionTransport::Stdio { command, args, env } = &agent.transport else {
        return Err(AcpConfigDiagnostic::new(
            "HTTP transport is no longer supported; declare a stdio command instead".to_string(),
        ));
    };
    let command = non_empty_trimmed(command)
        .ok_or_else(|| AcpConfigDiagnostic::new("stdio command must not be empty".to_string()))?;
    let id = extension_agent_config_id(agent)
        .ok_or_else(|| AcpConfigDiagnostic::new("agent id must not be empty".to_string()))?;
    let name = non_empty_or_else(&agent.name, || "ACP Agent".to_string());
    let config = AcpAgentConfig::new(
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
    .with_env(env.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    Ok(config
        .with_auth(extension_auth_config(agent))
        .with_timeouts(extension_timeout_config(agent)))
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
    // 传输只剩 stdio 一种形态，直接解构即可（留着 `if let` 只会被警告说恒真）。
    let AcpTransport::Stdio { env, .. } = &mut config.transport;
    merge_env(env, user.env);
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
