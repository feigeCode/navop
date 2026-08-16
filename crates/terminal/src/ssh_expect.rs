//! SSH 交互终端中的 expect/send 自动应答。
//!
//! 这里处理的是 SSH shell/channel 已打开后的设备 CLI 提示，不参与 SSH 协议认证。
//! 用户名步骤固定先于密码步骤，每个步骤最多执行一次。

use anyhow::{Context as _, Result, bail};
use one_core::storage::{SshAccountExpect, TerminalExpectSend};
use regex::bytes::Regex;

const MAX_PENDING_BYTES: usize = 16 * 1024;
const FEED_CHUNK_BYTES: usize = 8 * 1024;

#[derive(Clone)]
struct CompiledSshExpectStep {
    expect: Regex,
    send: Vec<u8>,
}

#[derive(Clone)]
pub(crate) struct SshLoginExpect {
    steps: Vec<CompiledSshExpectStep>,
    next_step: usize,
    pending: Vec<u8>,
}

impl SshLoginExpect {
    pub(crate) fn new(
        config: &SshAccountExpect,
        username: &str,
        password: Option<&str>,
    ) -> Result<Self> {
        let mut steps = Vec::with_capacity(2);
        compile_step(
            &mut steps,
            "username",
            &config.username,
            (!username.is_empty()).then_some(username),
        )?;
        compile_step(&mut steps, "password", &config.password, password)?;

        Ok(Self {
            steps,
            next_step: 0,
            pending: Vec::new(),
        })
    }

    pub(crate) fn advance(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        if self.is_complete() || data.is_empty() {
            return Vec::new();
        }

        let mut sends = Vec::new();
        for chunk in data.chunks(FEED_CHUNK_BYTES) {
            self.pending.extend_from_slice(chunk);
            self.match_pending(&mut sends);
            if self.is_complete() {
                self.pending.clear();
                break;
            }
            self.trim_pending();
        }
        sends
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.next_step >= self.steps.len()
    }

    fn match_pending(&mut self, sends: &mut Vec<Vec<u8>>) {
        while let Some(step) = self.steps.get(self.next_step) {
            let Some(found) = step.expect.find(&self.pending) else {
                break;
            };
            sends.push(step.send.clone());
            self.next_step += 1;
            self.pending.drain(..found.end());
        }
    }

    fn trim_pending(&mut self) {
        let overflow = self.pending.len().saturating_sub(MAX_PENDING_BYTES);
        if overflow > 0 {
            self.pending.drain(..overflow);
        }
    }
}

fn compile_step(
    steps: &mut Vec<CompiledSshExpectStep>,
    name: &str,
    step: &TerminalExpectSend,
    fallback: Option<&str>,
) -> Result<()> {
    if step.expect.is_empty() {
        if step.send.is_empty() {
            return Ok(());
        }
        bail!("SSH {name} expect is required when send is configured");
    }

    let expect = Regex::new(&step.expect)
        .with_context(|| format!("invalid SSH {name} expect regular expression"))?;
    if expect.is_match(b"") {
        bail!("SSH {name} expect regular expression cannot match empty input");
    }

    let send = if step.send.is_empty() {
        fallback
            .filter(|value| !value.is_empty())
            .with_context(|| format!("SSH {name} expect has no send value or runtime fallback"))?
    } else {
        step.send.as_str()
    };
    steps.push(CompiledSshExpectStep {
        expect,
        send: compile_login_send(send),
    });
    Ok(())
}

/// 解析 send 文本中的转义：`\r`、`\n`、`\t`、`\0`、`\\`、`\xNN`。
fn parse_send_escapes(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            let start = index;
            while index < bytes.len() && bytes[index] != b'\\' {
                index += 1;
            }
            output.extend_from_slice(&bytes[start..index]);
            continue;
        }
        parse_escape(bytes, &mut index, &mut output);
    }
    output
}

fn parse_escape(bytes: &[u8], index: &mut usize, output: &mut Vec<u8>) {
    let Some(escaped) = bytes.get(*index + 1).copied() else {
        output.push(b'\\');
        *index += 1;
        return;
    };
    match escaped {
        b'r' => {
            output.push(b'\r');
            *index += 2;
        }
        b'n' => {
            output.push(b'\n');
            *index += 2;
        }
        b't' => {
            output.push(b'\t');
            *index += 2;
        }
        b'0' => {
            output.push(0);
            *index += 2;
        }
        b'\\' => {
            output.push(b'\\');
            *index += 2;
        }
        b'x' => {
            if let Some(value) = parse_hex_byte(&bytes[*index + 2..]) {
                output.push(value);
                *index += 4;
            } else {
                output.push(b'\\');
                *index += 1;
            }
        }
        _ => {
            output.push(b'\\');
            *index += 1;
        }
    }
}

fn parse_hex_byte(bytes: &[u8]) -> Option<u8> {
    let high = hex_value(*bytes.first()?)?;
    let low = hex_value(*bytes.get(1)?)?;
    Some((high << 4) | low)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn compile_login_send(send: &str) -> Vec<u8> {
    let mut bytes = parse_send_escapes(send);
    if !bytes.ends_with(b"\r") && !bytes.ends_with(b"\n") {
        bytes.push(b'\r');
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(username: (&str, &str), password: (&str, &str)) -> SshAccountExpect {
        SshAccountExpect {
            username: TerminalExpectSend {
                expect: username.0.to_string(),
                send: username.1.to_string(),
            },
            password: TerminalExpectSend {
                expect: password.0.to_string(),
                send: password.1.to_string(),
            },
        }
    }

    #[test]
    fn runs_username_then_password_once() {
        let mut script = SshLoginExpect::new(
            &config(
                (r"(?i)(?:login|username)\s*:", "admin"),
                (r"(?i)password\s*:", "secret"),
            ),
            "fallback-user",
            Some("fallback-password"),
        )
        .unwrap();

        assert_eq!(
            script.advance(b"Username: Password: Password:"),
            vec![b"admin\r".to_vec(), b"secret\r".to_vec()]
        );
        assert!(script.advance(b"Username: Password:").is_empty());
        assert!(script.is_complete());
    }

    #[test]
    fn supports_password_only_expect() {
        let mut script = SshLoginExpect::new(
            &config(("", ""), (r"(?i)password\s*:", "")),
            "admin",
            Some("secret"),
        )
        .unwrap();

        assert_eq!(script.advance(b"Password: "), vec![b"secret\r".to_vec()]);
    }

    #[test]
    fn empty_send_uses_runtime_credentials() {
        let mut script = SshLoginExpect::new(
            &config(("Username:", ""), ("Password:", "")),
            "runtime-user",
            Some("runtime-password"),
        )
        .unwrap();

        assert_eq!(
            script.advance(b"Username: Password:"),
            vec![b"runtime-user\r".to_vec(), b"runtime-password\r".to_vec()]
        );
    }

    #[test]
    fn explicit_send_overrides_runtime_credentials_and_supports_escapes() {
        let mut script = SshLoginExpect::new(
            &config(("Username:", r"operator\r"), ("Password:", r"secret\x21")),
            "runtime-user",
            Some("runtime-password"),
        )
        .unwrap();

        assert_eq!(
            script.advance(b"Username: Password:"),
            vec![b"operator\r".to_vec(), b"secret!\r".to_vec()]
        );
    }

    #[test]
    fn matches_across_output_reads() {
        let mut script = SshLoginExpect::new(
            &config(("", ""), (r"(?i)password\s*:", "secret")),
            "admin",
            None,
        )
        .unwrap();

        assert!(script.advance(b"Pass").is_empty());
        assert_eq!(script.advance(b"word: "), vec![b"secret\r".to_vec()]);
    }

    #[test]
    fn rejects_missing_runtime_fallback() {
        let error = SshLoginExpect::new(&config(("", ""), ("Password:", "")), "admin", None)
            .err()
            .expect("missing password fallback should fail");
        assert!(error.to_string().contains("runtime fallback"));
    }

    #[test]
    fn rejects_send_without_expect_and_invalid_or_empty_regex() {
        assert!(
            SshLoginExpect::new(&config(("", "admin"), ("", "")), "admin", None)
                .err()
                .unwrap()
                .to_string()
                .contains("expect is required")
        );
        assert!(SshLoginExpect::new(&config(("(", "admin"), ("", "")), "admin", None).is_err());
        assert!(
            SshLoginExpect::new(&config((".*", "admin"), ("", "")), "admin", None)
                .err()
                .unwrap()
                .to_string()
                .contains("cannot match empty input")
        );
    }

    #[test]
    fn pending_buffer_is_bounded() {
        let mut script =
            SshLoginExpect::new(&config(("", ""), ("Password:", "secret")), "admin", None).unwrap();
        script.advance(&vec![b'x'; MAX_PENDING_BYTES * 3]);
        assert!(script.pending.len() <= MAX_PENDING_BYTES);
    }
}
