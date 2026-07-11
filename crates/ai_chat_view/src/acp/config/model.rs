use std::time::Duration;

use gpui::SharedString;

use super::AcpAgentConfig;

const DEFAULT_CONNECT_SECONDS: u64 = 30;
const DEFAULT_AUTHENTICATE_SECONDS: u64 = 120;
const DEFAULT_PROMPT_SECONDS: u64 = 600;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpAuthConfig {
    pub requested_method: Option<String>,
    pub preferred_method: Option<String>,
    pub allow_unauthenticated_fallback: bool,
    pub methods: Vec<AcpAuthMethodConfig>,
}

impl Default for AcpAuthConfig {
    fn default() -> Self {
        Self {
            requested_method: None,
            preferred_method: None,
            allow_unauthenticated_fallback: true,
            methods: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpAuthMethodConfig {
    pub id: String,
    pub env_any: Vec<String>,
    pub env_all: Vec<String>,
    pub interactive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcpTimeoutConfig {
    pub connect: Duration,
    pub authenticate: Duration,
    pub prompt: Duration,
}

impl Default for AcpTimeoutConfig {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(DEFAULT_CONNECT_SECONDS),
            authenticate: Duration::from_secs(DEFAULT_AUTHENTICATE_SECONDS),
            prompt: Duration::from_secs(DEFAULT_PROMPT_SECONDS),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpConfigDiagnostic {
    pub message: String,
}

impl AcpConfigDiagnostic {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AcpAgentEntry {
    pub id: SharedString,
    pub name: SharedString,
    pub config: Option<AcpAgentConfig>,
    pub diagnostic: Option<AcpConfigDiagnostic>,
}

impl AcpAgentEntry {
    pub fn ready(config: AcpAgentConfig) -> Self {
        Self {
            id: config.id.clone(),
            name: config.name.clone(),
            config: Some(config),
            diagnostic: None,
        }
    }

    pub fn invalid(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        diagnostic: AcpConfigDiagnostic,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            config: None,
            diagnostic: Some(diagnostic),
        }
    }
}
