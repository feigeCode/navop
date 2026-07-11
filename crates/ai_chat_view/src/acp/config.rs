//! ACP agent 配置:通用「自定义命令」形态(path + args + env)。
//!
//! 不写厂商特定逻辑:用户配置一条可执行命令(如 `npx -y @zed-industries/claude-code-acp`
//! 或 `gemini`),onetcli 作为 ACP 客户端把它当 stdio 子进程拉起。

use agent_client_protocol::schema::{EnvVariable, McpServer, McpServerHttp, McpServerStdio};
use agent_client_protocol::{AcpAgent, LineDirection};
use agent_runtime::SkillContext;
use gpui::SharedString;
use std::path::PathBuf;

mod model;

pub use model::{
    AcpAgentEntry, AcpAuthConfig, AcpAuthMethodConfig, AcpConfigDiagnostic, AcpTimeoutConfig,
};

/// ACP agent 传输类型。
#[derive(Clone, Debug)]
pub enum AcpTransport {
    /// stdio 子进程(默认)。
    Stdio {
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
    },
    /// 进程内 HTTP MCP 服务(streamable-http)。
    Http { url: String },
}

/// 一个可接入的 ACP agent 配置(自定义命令或进程内 HTTP)。
#[derive(Clone, Debug)]
pub struct AcpAgentConfig {
    /// 唯一标识(用于头部切换控件选中态)。
    pub id: SharedString,
    /// 展示名(头部按钮文案)。
    pub name: SharedString,
    /// 传输配置(stdio 子进程或 HTTP)。
    pub transport: AcpTransport,
    /// 鉴权选择与凭证要求。
    pub auth: AcpAuthConfig,
    /// 连接、鉴权和单轮请求超时。
    pub timeouts: AcpTimeoutConfig,
}

impl AcpAgentConfig {
    /// 构造 stdio 子进程模式的 ACP agent 配置。
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        command: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            transport: AcpTransport::Stdio {
                command: command.into(),
                args: Vec::new(),
                env: Vec::new(),
            },
            auth: AcpAuthConfig::default(),
            timeouts: AcpTimeoutConfig::default(),
        }
    }

    /// 构造进程内 HTTP 模式的 ACP agent 配置(用于连接 onetcli MCP HTTP 宿主)。
    pub fn new_http(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        url: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            transport: AcpTransport::Http { url: url.into() },
            auth: AcpAuthConfig::default(),
            timeouts: AcpTimeoutConfig::default(),
        }
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        if let AcpTransport::Stdio {
            args: ref mut a, ..
        } = self.transport
        {
            *a = args;
        }
        self
    }

    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        if let AcpTransport::Stdio { env: ref mut e, .. } = self.transport {
            *e = env;
        }
        self
    }

    pub fn with_auth(mut self, auth: AcpAuthConfig) -> Self {
        self.auth = auth;
        self
    }

    pub fn with_timeouts(mut self, timeouts: AcpTimeoutConfig) -> Self {
        self.timeouts = timeouts;
        self
    }

    pub fn with_skill_context(mut self, skills: &SkillContext) -> Self {
        if skills.is_empty() {
            return self;
        }
        if let AcpTransport::Stdio { env, .. } = &mut self.transport {
            env.push(("ONETCLI_SKILLS".to_string(), skills.describe()));
            env.push((
                "ONETCLI_SELECTED_SKILLS".to_string(),
                skills.selected_names_csv(),
            ));
        }
        self
    }

    /// 转为 `agent-client-protocol` 的 agent 传输配置。
    pub(crate) fn to_acp_agent(&self) -> AcpAgent {
        match &self.transport {
            AcpTransport::Stdio { command, args, env } => {
                // McpServerStdio 是 #[non_exhaustive],用 `new` + 公有字段赋值构造。
                let mut stdio = McpServerStdio::new(self.name.to_string(), PathBuf::from(command));
                stdio.args = args.clone();
                stdio.env = env
                    .iter()
                    .map(|(name, value)| EnvVariable::new(name.clone(), value.clone()))
                    .collect();
                // 把子进程的协议 I/O 接到 tracing。很多 ACP agent 会把 DEBUG/INFO
                // 正常日志写到 stderr,因此 stderr 需要按内容分级,不能一律 warn。
                let agent_name = self.name.to_string();
                AcpAgent::new(McpServer::Stdio(stdio)).with_debug(move |line, direction| {
                    match direction {
                        LineDirection::Stderr => log_acp_stderr(&agent_name, line),
                        LineDirection::Stdout => {
                            let line = strip_ansi_escapes(line);
                            tracing::debug!(agent = %agent_name, "acp recv: {line}");
                        }
                        LineDirection::Stdin => {
                            let line = strip_ansi_escapes(line);
                            tracing::debug!(agent = %agent_name, "acp send: {line}");
                        }
                    }
                })
            }
            AcpTransport::Http { url } => {
                let http = McpServerHttp::new(self.name.to_string(), url);
                AcpAgent::new(McpServer::Http(http))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AcpStderrLevel {
    Debug,
    Warn,
    Error,
}

fn classify_acp_stderr(line: &str) -> AcpStderrLevel {
    let lower = line.to_ascii_lowercase();
    if lower.contains("error") || lower.contains("panic") || lower.contains("fatal") {
        AcpStderrLevel::Error
    } else if lower.contains("warn") {
        AcpStderrLevel::Warn
    } else {
        AcpStderrLevel::Debug
    }
}

fn log_acp_stderr(agent_name: &str, line: &str) {
    let line = strip_ansi_escapes(line);
    match classify_acp_stderr(&line) {
        AcpStderrLevel::Debug => tracing::debug!(agent = %agent_name, "acp stderr: {line}"),
        AcpStderrLevel::Warn => tracing::warn!(agent = %agent_name, "acp stderr: {line}"),
        AcpStderrLevel::Error => tracing::error!(agent = %agent_name, "acp stderr: {line}"),
    }
}

fn strip_ansi_escapes(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            i = skip_ansi_escape(bytes, i + 1);
            continue;
        }
        let Some(ch) = line[i..].chars().next() else {
            break;
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn skip_ansi_escape(bytes: &[u8], i: usize) -> usize {
    if i >= bytes.len() {
        return i;
    }
    match bytes[i] {
        b'[' => skip_csi_escape(bytes, i + 1),
        b']' => skip_osc_escape(bytes, i + 1),
        _ => i + 1,
    }
}

fn skip_csi_escape(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        let b = bytes[i];
        i += 1;
        if (0x40..=0x7e).contains(&b) {
            break;
        }
    }
    i
}

fn skip_osc_escape(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        if bytes[i] == 0x07 {
            return i + 1;
        }
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
            return i + 2;
        }
        i += 1;
    }
    i
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
