use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use ai_chat_view::{AcpAgentConfig, AcpAuthConfig, AcpConfigDiagnostic, AcpTimeoutConfig};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// 当前写出的配置版本。
///
/// - v1：只支持对**扩展提供的** ACP agent 做覆盖（args/env/超时/鉴权方式）。
/// - v2：在 v1 基础上允许用户**自己定义** agent（写入 `command`），并记录 `active`
///   与 `enabled`。v1 文件按原语义继续读，保存时升到 v2。
const CONFIG_VERSION: u32 = 2;
const MIN_TIMEOUT_SECONDS: u64 = 1;
const MAX_TIMEOUT_SECONDS: u64 = 3600;
const SENSITIVE_SUFFIXES: [&str; 5] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"];

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct AcpUserConfig {
    pub version: u32,
    #[serde(default)]
    pub agents: BTreeMap<String, AcpUserAgentConfig>,
}

impl AcpUserConfig {
    pub(crate) fn empty() -> Self {
        Self {
            version: CONFIG_VERSION,
            agents: BTreeMap::new(),
        }
    }

    /// 用户自定义的 agent id（带 `command` 的那些）。
    #[cfg(test)]
    pub(crate) fn user_defined_ids(&self) -> Vec<String> {
        self.agents
            .iter()
            .filter(|(_, agent)| agent.command.is_some())
            .map(|(id, _)| id.clone())
            .collect()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct AcpUserAgentConfig {
    /// 展示名；缺省用 id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 启动命令。**存在即表示这是一条用户自定义 agent**，而不只是对扩展 agent 的覆盖。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// 是否启用；缺省视为启用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    #[serde(default, skip_serializing_if = "AcpUserTimeoutOverride::is_empty")]
    pub timeouts: AcpUserTimeoutOverride,
}

impl AcpUserAgentConfig {
    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    /// 完全空白的条目不必落盘。
    pub(crate) fn is_default_entry(&self) -> bool {
        self.name.is_none()
            && self.command.is_none()
            && self.args.is_none()
            && self.env.is_empty()
            && self.enabled.is_none()
            && self.auth_method.is_none()
            && self.timeouts.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub(crate) struct AcpUserTimeoutOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticate_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_seconds: Option<u64>,
}

impl AcpUserTimeoutOverride {
    fn is_empty(&self) -> bool {
        self.connect_seconds.is_none()
            && self.authenticate_seconds.is_none()
            && self.prompt_seconds.is_none()
    }

    pub(crate) fn apply(self, target: &mut AcpTimeoutConfig) {
        if let Some(seconds) = self.connect_seconds {
            target.connect = Duration::from_secs(seconds);
        }
        if let Some(seconds) = self.authenticate_seconds {
            target.authenticate = Duration::from_secs(seconds);
        }
        if let Some(seconds) = self.prompt_seconds {
            target.prompt = Duration::from_secs(seconds);
        }
    }
}

/// 解开环境变量引用后的覆盖值。
#[derive(Debug)]
pub(crate) struct AcpResolvedAgentOverride {
    pub auth_method: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: BTreeMap<String, String>,
    pub timeouts: AcpUserTimeoutOverride,
}

pub(crate) fn config_path() -> Result<std::path::PathBuf> {
    Ok(one_core::storage::manager::get_config_dir()?.join("acp-agents.json"))
}

pub(crate) fn load_user_config() -> Result<AcpUserConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(AcpUserConfig::empty());
    }
    let content = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    parse_user_config(&content)
}

/// 保存配置。写入前做一次校验，保证磁盘上的配置永远能被下次读取接受。
pub(crate) fn save_user_config(config: &AcpUserConfig) -> Result<()> {
    for agent in config.agents.values() {
        validate_agent_config(agent)?;
    }
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut normalized = AcpUserConfig {
        version: CONFIG_VERSION,
        agents: config.agents.clone(),
    };
    normalized
        .agents
        .retain(|_, agent| !agent.is_default_entry());
    let content = serde_json::to_string_pretty(&normalized)?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, content)?;
    fs::rename(&temporary, &path)?;
    Ok(())
}

pub(crate) fn parse_user_config(content: &str) -> Result<AcpUserConfig> {
    let config: AcpUserConfig = serde_json::from_str(content).context("parse acp-agents.json")?;
    if !(1..=CONFIG_VERSION).contains(&config.version) {
        bail!("unsupported ACP user config version: {}", config.version);
    }
    for agent in config.agents.values() {
        validate_agent_config(agent)?;
    }
    Ok(config)
}

pub(crate) fn resolve_override(
    value: &AcpUserAgentConfig,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<AcpResolvedAgentOverride> {
    let mut env = BTreeMap::new();
    for (name, value) in &value.env {
        env.insert(name.clone(), resolve_env_value(name, value, &lookup)?);
    }
    Ok(AcpResolvedAgentOverride {
        auth_method: value.auth_method.clone(),
        args: value.args.clone(),
        env,
        timeouts: value.timeouts,
    })
}

/// 把用户自定义的一条 agent 配置解开成运行期配置。
pub(crate) fn resolve_user_agent(
    id: &str,
    value: &AcpUserAgentConfig,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<AcpAgentConfig> {
    let command = value
        .command
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .ok_or_else(|| anyhow::anyhow!("agent {id} 缺少 command"))?;
    let name = value
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(id);
    let mut env = BTreeMap::new();
    for (name, value) in &value.env {
        env.insert(name.clone(), resolve_env_value(name, value, &lookup)?);
    }
    let mut config = AcpAgentConfig::new(id, name, command)
        .with_args(value.args.clone().unwrap_or_default())
        .with_env(env.into_iter().collect());
    config.auth = AcpAuthConfig {
        requested_method: value.auth_method.clone(),
        ..AcpAuthConfig::default()
    };
    value.timeouts.apply(&mut config.timeouts);
    Ok(config)
}

/// 把一条用户配置转成设置页展示用的诊断（配置不合法时用）。
pub(crate) fn user_agent_diagnostic(error: &anyhow::Error) -> AcpConfigDiagnostic {
    AcpConfigDiagnostic::new(error.to_string())
}

fn validate_agent_config(value: &AcpUserAgentConfig) -> Result<()> {
    if let Some(command) = value.command.as_deref()
        && command.trim().is_empty()
    {
        bail!("command must not be empty");
    }
    for (name, value) in &value.env {
        if is_sensitive(name) && env_reference(value).is_none() {
            bail!("{name} must use ${{env:NAME}}");
        }
    }
    validate_timeout("connect_seconds", value.timeouts.connect_seconds)?;
    validate_timeout("authenticate_seconds", value.timeouts.authenticate_seconds)?;
    validate_timeout("prompt_seconds", value.timeouts.prompt_seconds)
}

fn validate_timeout(name: &str, value: Option<u64>) -> Result<()> {
    if let Some(seconds) = value
        && !(MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&seconds)
    {
        bail!("{name} must be between 1 and 3600");
    }
    Ok(())
}

fn resolve_env_value(
    name: &str,
    value: &str,
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<String> {
    let Some(reference) = env_reference(value) else {
        return Ok(value.to_string());
    };
    lookup(reference)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing environment variable {reference} for {name}"))
}

fn env_reference(value: &str) -> Option<&str> {
    value
        .strip_prefix("${env:")
        .and_then(|value| value.strip_suffix('}'))
        .filter(|name| !name.is_empty() && !name.contains(['{', '}']))
}

fn is_sensitive(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SENSITIVE_SUFFIXES
        .iter()
        .any(|suffix| upper.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_one_files_stay_readable_and_default_to_enabled() {
        let parsed = parse_user_config(
            r#"{
                "version": 1,
                "agents": {
                    "codex.codex": {"args": ["--stdio"]}
                }
            }"#,
        )
        .unwrap();

        let agent = &parsed.agents["codex.codex"];
        assert!(agent.command.is_none());
        assert!(agent.is_enabled());
        assert_eq!(Some(vec!["--stdio".to_string()]), agent.args);
    }

    #[test]
    fn version_three_is_rejected() {
        let error = parse_user_config(r#"{"version": 3, "agents": {}}"#).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unsupported ACP user config version")
        );
    }

    #[test]
    fn blank_command_is_rejected() {
        let error = parse_user_config(r#"{"version": 2, "agents": {"mine": {"command": "   "}}}"#)
            .unwrap_err();

        assert!(error.to_string().contains("command must not be empty"));
    }

    #[test]
    fn user_defined_agents_resolve_to_a_runnable_config() {
        let parsed = parse_user_config(
            r#"{
                "version": 2,
                "agents": {
                    "mine": {
                        "name": "My Agent",
                        "command": "my-acp",
                        "args": ["--stdio"],
                        "env": {"LOG": "debug"}
                    }
                }
            }"#,
        )
        .unwrap();

        let config = resolve_user_agent("mine", &parsed.agents["mine"], |_| None).unwrap();

        assert_eq!("mine", config.id.as_ref());
        assert_eq!("My Agent", config.name.as_ref());
        let ai_chat_view::AcpTransport::Stdio { command, args, env } = &config.transport;
        assert_eq!("my-acp", command);
        assert_eq!(&vec!["--stdio".to_string()], args);
        assert_eq!(&vec![("LOG".to_string(), "debug".to_string())], env);
    }

    #[test]
    fn user_defined_ids_only_lists_entries_with_a_command() {
        let parsed = parse_user_config(
            r#"{
                "version": 2,
                "agents": {
                    "mine": {"command": "my-acp"},
                    "codex.codex": {"enabled": false}
                }
            }"#,
        )
        .unwrap();

        assert_eq!(vec!["mine".to_string()], parsed.user_defined_ids());
    }
}
