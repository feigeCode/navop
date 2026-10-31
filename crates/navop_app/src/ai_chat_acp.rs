use ai_chat_view::{
    AcpAgentConfig, AcpAgentEntry, AcpAgentSource, AcpConfigDiagnostic, AcpProbeRecord,
    AcpTransport, builtin_acp_agent_preset, builtin_acp_agent_presets, probe_fingerprint,
    set_acp_agent_config_provider,
};
use anyhow::bail;
use gpui::App;
use one_core::settings::AppSettings;
use std::collections::HashSet;
use std::path::Path;

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
    let mut entries = builtin_agent_entries()?;
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

/// 新增或更新一条用户自定义 agent（不依赖预设，自带启动命令）。
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

/// 删除一条**用户自定义**的 agent；内置预设走 [`reset_agent_override`]。
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

/// 保存内置预设的参数/环境变量覆盖。
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

/// 清掉某条目的覆盖/启用记录，回到内置预设的默认状态。
pub fn reset_agent_override(id: &str) -> anyhow::Result<()> {
    let mut config = load_user_config()?;
    config.agents.remove(id);
    save_user_config(&config)
}

/// 启用/停用某个 agent。停用后聊天切换器不再展示它。
pub fn set_agent_enabled(id: &str, enabled: bool) -> anyhow::Result<()> {
    let mut config = load_user_config()?;
    let entry = config.agents.entry(id.to_string()).or_default();
    if entry.command.is_none() {
        entry.enabled = preset_enabled_override(id, enabled);
    } else {
        entry.enabled = Some(enabled);
    }
    if entry.is_default_entry() {
        config.agents.remove(id);
    }
    save_user_config(&config)
}

/// 打开/关掉一条内置预设时该写下的 `enabled` 值。
///
/// 默认就启用的预设，打开它不需要留记录（读配置时没有覆盖就是启用）；
/// 默认关闭的预设必须写 `Some(true)`，否则下一次读回来还是关着的。
fn preset_enabled_override(id: &str, enabled: bool) -> Option<bool> {
    let enabled_by_default =
        builtin_acp_agent_preset(id).is_some_and(|preset| preset.enabled_by_default());
    if enabled_by_default && enabled {
        None
    } else {
        Some(enabled)
    }
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

/// 内置预设 + 用户自定义，按「预设在前」的顺序拼起来。
///
/// 预设条目的命令在这里就解析成绝对路径：GUI 进程（Finder/Dock 启动）继承不到
/// shell 的 PATH，`npm` / `gemini` / `opencode` 这类命令不解析就会直接
/// `command not found`；解析成功的同时把程序所在目录补进子进程 PATH，
/// `npm exec` 再拉起的子进程才能找到 node。
fn builtin_agent_entries() -> anyhow::Result<Vec<AcpAgentEntry>> {
    let user_config = load_user_config()?;
    let lookup = |name: &str| std::env::var(name).ok();
    let mut entries = preset_agent_entries(&user_config, &lookup);
    entries.extend(user_defined_agent_entries(&user_config, &lookup));
    Ok(dedupe_agent_entries(entries))
}

fn preset_agent_entries(
    user_config: &AcpUserConfig,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Vec<AcpAgentEntry> {
    preset_agent_entries_with(user_config, lookup, process_util::resolve_program)
}

fn preset_agent_entries_with(
    user_config: &AcpUserConfig,
    lookup: &impl Fn(&str) -> Option<String>,
    resolve: impl Fn(&str) -> Option<std::path::PathBuf>,
) -> Vec<AcpAgentEntry> {
    builtin_acp_agent_presets()
        .iter()
        .map(|preset| {
            // 预设条目默认只开常用几个：全量列表留给设置页，切换器不塞满。
            let enabled_by_default = preset.enabled_by_default();
            let mut config = preset.config();
            if let Err(diagnostic) = resolve_preset_command(&mut config, &resolve) {
                return AcpAgentEntry::invalid(preset.id, preset.name, diagnostic)
                    .with_source(AcpAgentSource::Preset)
                    .with_enabled(enabled_by_default);
            }
            let Some(override_config) = user_config.agents.get(preset.id) else {
                return AcpAgentEntry::ready(config)
                    .with_source(AcpAgentSource::Preset)
                    .with_enabled(enabled_by_default);
            };
            let enabled = override_config.is_enabled();
            match resolve_override(override_config, lookup) {
                Ok(resolved) => {
                    apply_user_override(&mut config, resolved);
                    AcpAgentEntry::ready(config)
                        .with_source(AcpAgentSource::Preset)
                        .with_enabled(enabled)
                }
                Err(error) => AcpAgentEntry::invalid(
                    preset.id,
                    preset.name,
                    AcpConfigDiagnostic::new(error.to_string()),
                )
                .with_source(AcpAgentSource::Preset)
                .with_enabled(enabled),
            }
        })
        .collect()
}

/// 把预设命令解析成可执行文件：绝对路径原样校验，其余按 PATH 查找。
fn resolve_preset_command(
    config: &mut AcpAgentConfig,
    resolve: impl Fn(&str) -> Option<std::path::PathBuf>,
) -> Result<(), AcpConfigDiagnostic> {
    let AcpTransport::Stdio { command, env, .. } = &mut config.transport;
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err(AcpConfigDiagnostic::new(
            "agent 启动命令不能为空".to_string(),
        ));
    }
    let path = Path::new(trimmed);
    let resolved = if path.components().count() > 1 {
        if path.is_file() {
            path.to_path_buf()
        } else {
            return Err(AcpConfigDiagnostic::new(format!(
                "找不到命令 `{trimmed}`：请把它改成可执行文件的绝对路径，或安装后再试"
            )));
        }
    } else {
        resolve(trimmed).ok_or_else(|| {
            AcpConfigDiagnostic::new(format!(
                "找不到命令 `{trimmed}`：请先安装对应 CLI（或在设置里把启动命令改成绝对路径）"
            ))
        })?
    };
    // 把程序目录补进子进程 PATH：npm/npx 需要找到同目录的 node，
    // 否则 GUI 里拉起时会以 `env: node: No such file or directory` 失败。
    // 只补目录还不够：适配器会再 spawn 兄弟 CLI（`pi-acp` 找 `pi`），所以补全逻辑
    // 还要带上 GUI 进程看不到的工具链目录（nvm / fnm / volta 等）。
    extend_child_path(env, &resolved);
    *command = resolved.to_string_lossy().to_string();
    Ok(())
}

/// 给 ACP 子进程的 PATH 补上程序目录与 GUI 进程看不到的工具链目录。
///
/// 以配置里已写的 PATH（若有）为底：用户显式配置的目录要排在兜底目录之前，
/// 不能因为补全逻辑把整条 PATH 换掉。
fn extend_child_path(env: &mut Vec<(String, String)>, program: &Path) {
    let configured = env
        .iter()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| value.clone());
    let path = process_util::child_path_with_program_dir_and_path(
        program,
        configured.as_deref().map(std::ffi::OsStr::new),
    );
    let path = path.to_string_lossy().to_string();
    env.retain(|(name, _)| name != "PATH");
    env.push(("PATH".to_string(), path));
}

/// 用户自己定义的 agent（配置里带 `command` 的条目）。
fn user_defined_agent_entries(
    config: &AcpUserConfig,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Vec<AcpAgentEntry> {
    config
        .agents
        .iter()
        // 没有 command 的条目只是对内置预设的覆盖，不在这里生成新 agent。
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
    let AcpTransport::Stdio { command, env, .. } = &mut config.transport;
    merge_env(env, user.env);
    // 覆盖项里重写了 PATH 时，用户目录要留，但程序目录与工具链目录也不能丢：
    // 适配器再 spawn 兄弟 CLI 时仍然依赖它们。
    if env.iter().any(|(name, _)| name == "PATH") {
        let command = command.clone();
        extend_child_path(env, Path::new(&command));
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
