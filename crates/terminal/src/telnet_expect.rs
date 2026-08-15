//! Telnet expect/send 自动登录脚本。
//!
//! `expect` 按字节正则表达式匹配解码后的 Telnet 服务端输出。状态机会保留
//! 有界窗口以支持跨 TCP read 分片匹配，并且严格按配置顺序、每步只执行一次。

use one_core::storage::models::TelnetLoginStep;
use regex::bytes::Regex;

const MAX_PENDING_BYTES: usize = 16 * 1024;
const FEED_CHUNK_BYTES: usize = 8 * 1024;

#[derive(Clone)]
struct CompiledTelnetLoginStep {
    expect: Regex,
    send: Vec<u8>,
}

#[derive(Clone)]
pub(crate) struct TelnetLoginScript {
    steps: Vec<CompiledTelnetLoginStep>,
    next_step: usize,
    pending: Vec<u8>,
}

impl TelnetLoginScript {
    pub(crate) fn new(steps: &[TelnetLoginStep]) -> Result<Self, regex::Error> {
        let mut compiled = Vec::with_capacity(steps.len());
        for step in steps {
            if step.expect.is_empty() {
                tracing::warn!("忽略 expect 为空的 Telnet 登录脚本步骤");
                continue;
            }
            let expect = Regex::new(&step.expect)?;
            if expect.is_match(b"") {
                return Err(regex::Error::Syntax(
                    "Telnet expect 正则不能匹配空内容".to_string(),
                ));
            }
            compiled.push(CompiledTelnetLoginStep {
                expect,
                send: compile_login_send(&step.send),
            });
        }
        Ok(Self {
            steps: compiled,
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
            let consumed = found.end();
            sends.push(step.send.clone());
            self.next_step += 1;
            self.pending.drain(..consumed);
        }
    }

    fn trim_pending(&mut self) {
        let overflow = self.pending.len().saturating_sub(MAX_PENDING_BYTES);
        if overflow > 0 {
            self.pending.drain(..overflow);
        }
    }
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

    fn script(steps: &[TelnetLoginStep]) -> TelnetLoginScript {
        TelnetLoginScript::new(steps).expect("valid Telnet expect regex")
    }
    #[test]
    fn matches_case_insensitive_regex_across_reads() {
        let mut script = script(&[TelnetLoginStep {
            expect: r"(?i)(?:login|username)\s*:".to_string(),
            send: "admin".to_string(),
        }]);

        assert!(script.advance(b"Welcome\r\nUser").is_empty());
        assert_eq!(script.advance(b"name : "), vec![b"admin\r".to_vec()]);
        assert!(script.is_complete());
    }

    #[test]
    fn runs_steps_in_order_within_one_chunk_once_each() {
        let mut script = script(&[
            TelnetLoginStep {
                expect: r"(?i)(?:login|username)\s*:".to_string(),
                send: "admin".to_string(),
            },
            TelnetLoginStep {
                expect: r"(?i)password\s*:".to_string(),
                send: "secret".to_string(),
            },
        ]);

        assert_eq!(
            script.advance(b"Username: Password: Password: "),
            vec![b"admin\r".to_vec(), b"secret\r".to_vec()]
        );
        assert!(script.advance(b"Username: Password: ").is_empty());
    }

    #[test]
    fn consumes_output_through_regex_match_end() {
        let mut script = script(&[
            TelnetLoginStep {
                expect: r"user(name)?\s*:".to_string(),
                send: "admin".to_string(),
            },
            TelnetLoginStep {
                expect: r"password\s*:".to_string(),
                send: "secret".to_string(),
            },
        ]);

        assert_eq!(
            script.advance(b"username: password: "),
            vec![b"admin\r".to_vec(), b"secret\r".to_vec()]
        );
    }

    #[test]
    fn supports_send_escapes_and_does_not_duplicate_enter() {
        let mut script = script(&[
            TelnetLoginStep {
                expect: r"login:\r\n".to_string(),
                send: r"admin\r".to_string(),
            },
            TelnetLoginStep {
                expect: "Password:".to_string(),
                send: r"secret\x21".to_string(),
            },
        ]);

        assert_eq!(script.advance(b"login:\r\n"), vec![b"admin\r".to_vec()]);
        assert_eq!(script.advance(b"Password: "), vec![b"secret!\r".to_vec()]);
    }

    #[test]
    fn pending_buffer_is_bounded_and_still_matches_across_reads() {
        let mut script = script(&[TelnetLoginStep {
            expect: "password:".to_string(),
            send: "secret".to_string(),
        }]);

        script.advance(&vec![b'x'; MAX_PENDING_BYTES * 3]);
        assert!(script.pending.len() <= MAX_PENDING_BYTES);
        assert!(script.advance(b"pass").is_empty());
        assert_eq!(script.advance(b"word: "), vec![b"secret\r".to_vec()]);
    }

    #[test]
    fn invalid_regex_returns_error_without_panicking() {
        let result = TelnetLoginScript::new(&[TelnetLoginStep {
            expect: "(?i)(login".to_string(),
            send: "admin".to_string(),
        }]);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_regex_that_matches_empty_input() {
        let result = TelnetLoginScript::new(&[TelnetLoginStep {
            expect: "a*".to_string(),
            send: "admin".to_string(),
        }]);
        assert!(result.is_err());
    }

    #[test]
    fn skips_empty_expect_steps() {
        let mut script = script(&[
            TelnetLoginStep {
                expect: String::new(),
                send: "skip-me".to_string(),
            },
            TelnetLoginStep {
                expect: "ready>".to_string(),
                send: "go".to_string(),
            },
        ]);

        assert_eq!(script.advance(b"ready> "), vec![b"go\r".to_vec()]);
        assert!(script.is_complete());
    }
}
