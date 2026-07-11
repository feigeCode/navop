use std::collections::HashSet;

use anyhow::{Result, bail};
use serde::Deserialize;

const MIN_TIMEOUT_SECONDS: u64 = 1;
const MAX_TIMEOUT_SECONDS: u64 = 3600;
const DEFAULT_CONNECT_SECONDS: u64 = 30;
const DEFAULT_AUTHENTICATE_SECONDS: u64 = 120;
const DEFAULT_PROMPT_SECONDS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AcpAgentExtensionAuth {
    #[serde(default)]
    pub preferred_method: Option<String>,
    #[serde(default = "default_true")]
    pub allow_unauthenticated_fallback: bool,
    #[serde(default)]
    pub methods: Vec<AcpAgentExtensionAuthMethod>,
}

impl Default for AcpAgentExtensionAuth {
    fn default() -> Self {
        Self {
            preferred_method: None,
            allow_unauthenticated_fallback: true,
            methods: Vec::new(),
        }
    }
}

impl AcpAgentExtensionAuth {
    pub(super) fn validate(&self) -> Result<()> {
        let mut ids = HashSet::new();
        for method in &self.methods {
            method.validate()?;
            if !ids.insert(method.id.as_str()) {
                bail!("ACP auth method id must be unique: {}", method.id);
            }
        }
        if let Some(preferred) = self.preferred_method.as_deref() {
            if preferred.trim().is_empty() {
                bail!("ACP preferred auth method must not be empty");
            }
            if !ids.contains(preferred) {
                bail!("ACP preferred auth method is not configured: {preferred}");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AcpAgentExtensionAuthMethod {
    pub id: String,
    #[serde(default)]
    pub env_any: Vec<String>,
    #[serde(default)]
    pub env_all: Vec<String>,
    #[serde(default)]
    pub interactive: bool,
}

impl AcpAgentExtensionAuthMethod {
    fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            bail!("ACP auth method id must not be empty");
        }
        if self
            .env_any
            .iter()
            .chain(&self.env_all)
            .any(|name| name.trim().is_empty())
        {
            bail!("ACP auth environment variable name must not be empty");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct AcpAgentExtensionTimeouts {
    #[serde(default = "default_connect_seconds")]
    pub connect_seconds: u64,
    #[serde(default = "default_authenticate_seconds")]
    pub authenticate_seconds: u64,
    #[serde(default = "default_prompt_seconds")]
    pub prompt_seconds: u64,
}

impl Default for AcpAgentExtensionTimeouts {
    fn default() -> Self {
        Self {
            connect_seconds: DEFAULT_CONNECT_SECONDS,
            authenticate_seconds: DEFAULT_AUTHENTICATE_SECONDS,
            prompt_seconds: DEFAULT_PROMPT_SECONDS,
        }
    }
}

impl AcpAgentExtensionTimeouts {
    pub(super) fn validate(&self) -> Result<()> {
        validate_timeout("connect_seconds", self.connect_seconds)?;
        validate_timeout("authenticate_seconds", self.authenticate_seconds)?;
        validate_timeout("prompt_seconds", self.prompt_seconds)
    }
}

fn validate_timeout(name: &str, seconds: u64) -> Result<()> {
    if !(MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&seconds) {
        bail!("{name} must be between 1 and 3600");
    }
    Ok(())
}

const fn default_true() -> bool {
    true
}

const fn default_connect_seconds() -> u64 {
    DEFAULT_CONNECT_SECONDS
}

const fn default_authenticate_seconds() -> u64 {
    DEFAULT_AUTHENTICATE_SECONDS
}

const fn default_prompt_seconds() -> u64 {
    DEFAULT_PROMPT_SECONDS
}
