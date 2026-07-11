use std::fmt;

use serde_json::Value;

const MAX_DETAIL_CHARS: usize = 8 * 1024;
const SENSITIVE_SUFFIXES: [&str; 5] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcpErrorKind {
    CommandNotFound,
    CommandNotExecutable,
    ProcessExited,
    ConnectTimeout,
    InitializeFailed,
    ProtocolVersionMismatch,
    UnsupportedAuthMethod,
    MissingCredentials,
    AuthenticationFailed,
    AuthenticationTimeout,
    SessionCreationFailed,
    TurnAlreadyRunning,
    PromptFailed,
    PromptTimeout,
    EmptyResponse,
    ConnectionClosed,
    InvalidUserConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcpRecoveryAction {
    Retry,
    Authenticate,
    Configure { path: String },
    SelectAnotherAgent,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpError {
    pub kind: AcpErrorKind,
    pub agent_id: String,
    pub agent_name: String,
    pub phase: String,
    pub summary: String,
    pub detail: String,
    pub recovery: AcpRecoveryAction,
}

impl AcpError {
    pub fn new(
        kind: AcpErrorKind,
        agent_id: impl Into<String>,
        agent_name: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            agent_id: agent_id.into(),
            agent_name: agent_name.into(),
            phase: String::new(),
            summary: summary.into(),
            detail: String::new(),
            recovery: AcpRecoveryAction::None,
        }
    }

    pub fn with_phase(mut self, phase: impl Into<String>) -> Self {
        self.phase = phase.into();
        self
    }

    pub fn with_detail(mut self, detail: impl AsRef<str>) -> Self {
        self.detail = sanitize_detail(detail.as_ref());
        self
    }

    pub fn with_recovery(mut self, recovery: AcpRecoveryAction) -> Self {
        self.recovery = recovery;
        self
    }

    pub fn empty_response(agent_id: impl Into<String>, agent_name: impl Into<String>) -> Self {
        Self::new(
            AcpErrorKind::EmptyResponse,
            agent_id,
            agent_name,
            "ACP Agent 没有返回任何内容",
        )
        .with_detail("请求正常结束，但没有收到文本、推理、工具调用或计划更新")
        .with_recovery(AcpRecoveryAction::Authenticate)
    }
}

impl fmt::Display for AcpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.summary)?;
        if !self.detail.is_empty() {
            write!(formatter, ": {}", self.detail)?;
        }
        Ok(())
    }
}

impl std::error::Error for AcpError {}

pub(crate) fn extract_rpc_error_detail(message: &str, data: Option<&Value>) -> String {
    let candidate = data
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
        .or_else(|| {
            data.and_then(|value| value.get("additionalDetails"))
                .and_then(Value::as_str)
        })
        .unwrap_or(message);
    let status = data.and_then(find_http_status);
    let detail = match status {
        Some(status) if !candidate.contains(&status.to_string()) => {
            format!("HTTP {status}: {candidate}")
        }
        _ => candidate.to_string(),
    };
    sanitize_detail(&detail)
}

fn find_http_status(value: &Value) -> Option<u64> {
    match value {
        Value::Object(object) => object.iter().find_map(|(key, value)| {
            if key == "httpStatusCode" {
                value.as_u64()
            } else {
                find_http_status(value)
            }
        }),
        Value::Array(values) => values.iter().find_map(find_http_status),
        _ => None,
    }
}

pub(crate) fn sanitize_detail(value: &str) -> String {
    let without_ansi = strip_ansi_escapes(value);
    redact_secrets(&without_ansi)
        .chars()
        .take(MAX_DETAIL_CHARS)
        .collect()
}

pub(crate) fn redact_secrets(value: &str) -> String {
    value
        .lines()
        .map(redact_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_line(line: &str) -> String {
    let assignments = line
        .split(' ')
        .map(redact_assignment)
        .collect::<Vec<_>>()
        .join(" ");
    let lower = assignments.to_ascii_lowercase();
    let Some(index) = lower.find("authorization:") else {
        return assignments;
    };
    format!("{}Authorization: [REDACTED]", &assignments[..index])
}

fn redact_assignment(token: &str) -> String {
    let Some((name, _)) = token.split_once('=') else {
        return token.to_string();
    };
    let upper = name.to_ascii_uppercase();
    if SENSITIVE_SUFFIXES
        .iter()
        .any(|suffix| upper.ends_with(suffix))
    {
        format!("{name}=[REDACTED]")
    } else {
        token.to_string()
    }
}

fn strip_ansi_escapes(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut output = String::with_capacity(line.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b {
            index = skip_ansi(bytes, index + 1);
        } else if let Some(character) = line[index..].chars().next() {
            output.push(character);
            index += character.len_utf8();
        } else {
            break;
        }
    }
    output
}

fn skip_ansi(bytes: &[u8], index: usize) -> usize {
    if bytes.get(index) != Some(&b'[') {
        return index.saturating_add(1);
    }
    let mut cursor = index + 1;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        cursor += 1;
        if (0x40..=0x7e).contains(&byte) {
            break;
        }
    }
    cursor
}
