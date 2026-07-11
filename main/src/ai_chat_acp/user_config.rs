use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use ai_chat_view::AcpTimeoutConfig;
use anyhow::{Context, Result, bail};
use serde::Deserialize;

const CONFIG_VERSION: u32 = 1;
const MIN_TIMEOUT_SECONDS: u64 = 1;
const MAX_TIMEOUT_SECONDS: u64 = 3600;
const SENSITIVE_SUFFIXES: [&str; 5] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"];

#[derive(Debug, Default, Deserialize)]
pub(super) struct AcpUserConfig {
    pub version: u32,
    #[serde(default)]
    pub agents: BTreeMap<String, AcpUserAgentOverride>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct AcpUserAgentOverride {
    #[serde(default)]
    pub auth_method: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub timeouts: AcpUserTimeoutOverride,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub(super) struct AcpUserTimeoutOverride {
    pub connect_seconds: Option<u64>,
    pub authenticate_seconds: Option<u64>,
    pub prompt_seconds: Option<u64>,
}

#[derive(Debug)]
pub(super) struct AcpResolvedAgentOverride {
    pub auth_method: Option<String>,
    pub args: Option<Vec<String>>,
    pub env: BTreeMap<String, String>,
    pub timeouts: AcpUserTimeoutOverride,
}

pub(super) fn load_user_config() -> Result<AcpUserConfig> {
    let path = one_core::storage::manager::get_config_dir()?.join("acp-agents.json");
    if !path.exists() {
        return Ok(AcpUserConfig {
            version: CONFIG_VERSION,
            agents: BTreeMap::new(),
        });
    }
    let content = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    parse_user_config(&content)
}

pub(super) fn parse_user_config(content: &str) -> Result<AcpUserConfig> {
    let config: AcpUserConfig = serde_json::from_str(content).context("parse acp-agents.json")?;
    if config.version != CONFIG_VERSION {
        bail!("unsupported ACP user config version: {}", config.version);
    }
    for agent in config.agents.values() {
        validate_override(agent)?;
    }
    Ok(config)
}

pub(super) fn resolve_override(
    value: &AcpUserAgentOverride,
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

impl AcpUserTimeoutOverride {
    pub(super) fn apply(self, target: &mut AcpTimeoutConfig) {
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

fn validate_override(value: &AcpUserAgentOverride) -> Result<()> {
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
