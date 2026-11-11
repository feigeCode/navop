//! SSH 会话内提权（`su` / `sudo`）密码提示的自动应答。
//!
//! [`SshLoginExpect`](crate::ssh_expect::SshLoginExpect) 只负责通道刚打开时的
//! 一次性登录应答；`su root`、`sudo -i` 这类提示出现在会话中途，且可能重复
//! 出现，因此这里维护一个不消耗步骤的「提示观察器」。
//!
//! 直接按「行尾出现 Password:」就去填密码太危险：`mysql -u root -p`、`read -s`
//! 脚本、甚至 `cat` 一个含 Password 字样的文件都会拿到密码。因此这里的策略是
//! **armed 窗口**：只有最近在输出里看到过 `sudo` / `su` 命令（或登录后命令里
//! 自带这两个词）才进入 armed 状态，armed 期间填写有次数上限，并且同一个提示
//! 再次出现（密码被拒）就立刻停手，把控制权交还给用户。

use std::time::{Duration, Instant};

use one_core::storage::SshElevationExpect;
use regex::Regex;
use std::sync::OnceLock;
use tracing::debug;

/// 保留的输出尾部长度上限；提示一定出现在最后几十个字节里。
const TAIL_CHARS: usize = 4 * 1024;
/// 看到 `sudo` / `su` 之后，多久内允许自动应答密码提示。
const ARM_WINDOW: Duration = Duration::from_secs(20);
/// 每次成功应答后顺延的 armed 时长，用于 `sudo su root` 这类连续提示。
const ARM_EXTENSION: Duration = Duration::from_secs(10);
/// 同一个提示在这个时间窗内再次出现，说明密码被拒，停止自动应答。
const REJECT_WINDOW: Duration = Duration::from_secs(20);
/// 单次会话最多自动应答多少次，防止误判后无限灌密码。
const MAX_FILLS: u32 = 3;

fn privilege_command_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)(?:^|[\s;&|(`$])(?:sudo|su)(?:\s|$)")
            .expect("valid privilege command regex")
    })
}

fn prompt_regexes() -> &'static [Regex] {
    static REGEXES: OnceLock<Vec<Regex>> = OnceLock::new();
    REGEXES.get_or_init(|| {
        [
            // `[sudo] password for alice:`
            r"(?i)^\s*\[sudo\]\s*password for [^:\n]{0,64}\s*:\s*$",
            // `Password:` / `alice's password:` / `Password for root:`
            r"(?i)^[^\n]{0,80}?\bpassword(?:\s+for\s+[^:\n]{0,64})?\s*:\s*$",
            // `Enter PASSCODE:` 之类只把 password 写成长一点的场景较少，这里补 passwd。
            r"(?i)^[^\n]{0,80}?\bpasswd(?:\s+for\s+[^:\n]{0,64})?\s*:\s*$",
            // 中文提示：`密码：`
            r"^\s*[^\n]{0,40}密码\s*[:：]\s*$",
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("valid password prompt regex"))
        .collect()
    })
}

/// 会话内 `su` / `sudo` 密码提示的自动应答器。
pub(crate) struct SessionPasswordExpect {
    enabled: bool,
    password: Option<String>,
    tail: String,
    /// 连接建立时登录后命令就自带 sudo/su，首段输出到达即可视为已 armed。
    pending_init_arm: bool,
    armed_until: Option<Instant>,
    fills: u32,
    last_fill: Option<Instant>,
    last_prompt: Option<String>,
}

impl SessionPasswordExpect {
    /// 按主机配置构造。
    ///
    /// `login_password` 是本次连接实际使用的登录密码；`init_commands` 里出现
    /// `sudo` / `su` 时，即使远端不回显也能在首段输出后进入 armed。
    pub(crate) fn new(
        config: &SshElevationExpect,
        login_password: Option<&str>,
        init_commands: Option<&str>,
    ) -> Self {
        let (enabled, password) = config.resolve(login_password);
        let pending_init_arm =
            init_commands.is_some_and(|commands| privilege_command_regex().is_match(commands));
        Self {
            enabled: enabled && password.is_some(),
            password,
            tail: String::new(),
            pending_init_arm,
            armed_until: None,
            fills: 0,
            last_fill: None,
            last_prompt: None,
        }
    }

    /// 是否需要对输出做观察（未启用时调用方可以跳过解码）。
    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 观察一段解码后的终端输出，命中提权密码提示时返回要写回的字节（含回车）。
    pub(crate) fn observe(&mut self, data: &str, now: Instant) -> Option<Vec<u8>> {
        if !self.enabled || data.is_empty() {
            return None;
        }
        let cleaned = strip_control_sequences(data);
        if cleaned.is_empty() {
            return None;
        }
        self.push_tail(&cleaned);

        if self.pending_init_arm {
            self.pending_init_arm = false;
            self.armed_until = Some(now + ARM_WINDOW);
        }

        // 只在「已经结束的行」里找命令，避免提示文本（`[sudo] password for u:`）
        // 本身把 armed 打开。
        let completed_end = self
            .tail
            .rfind(['\n', '\r'])
            .map(|index| index + 1)
            .unwrap_or(0);
        if completed_end > 0 {
            if privilege_command_regex().is_match(&self.tail[..completed_end]) {
                self.armed_until = Some(now + ARM_WINDOW);
            }
            // 已经结束的行只用于识别命令，之后不必再参与匹配：把尾部收窄到
            // 「最后一行未完成的输出」，既省内存，也避免旧命令反复续期 armed。
            self.tail.drain(..completed_end);
        }

        let prompt = self.current_prompt()?;
        if !self.armed_until.is_some_and(|until| now < until) {
            return None;
        }
        if self.fills >= MAX_FILLS {
            self.armed_until = None;
            debug!(
                target: "terminal.ssh.session_password",
                "session password auto-fill reached the per-session limit"
            );
            return None;
        }
        if self.last_fill.is_some_and(|filled_at| {
            now.duration_since(filled_at) < REJECT_WINDOW
                && self.last_prompt.as_deref() == Some(prompt.as_str())
        }) {
            // 同一个提示再次出现说明密码被拒：停手，别把用户的密码反复喂给远端。
            self.armed_until = None;
            debug!(
                target: "terminal.ssh.session_password",
                "session password auto-fill stopped after a rejected prompt"
            );
            return None;
        }

        let password = self.password.clone()?;
        self.fills += 1;
        self.last_fill = Some(now);
        self.last_prompt = Some(prompt);
        self.armed_until = Some(now + ARM_EXTENSION);
        debug!(
            target: "terminal.ssh.session_password",
            fills = self.fills,
            "answered a session password prompt"
        );
        let mut payload = password.into_bytes();
        payload.push(b'\r');
        Some(payload)
    }

    fn push_tail(&mut self, data: &str) {
        self.tail.push_str(data);
        let len = self.tail.len();
        if len <= TAIL_CHARS {
            return;
        }
        let mut cut = len - TAIL_CHARS;
        while cut < len && !self.tail.is_char_boundary(cut) {
            cut += 1;
        }
        self.tail.drain(..cut);
    }

    /// 当前尾部最后一行若是密码提示，返回规范化后的提示文本。
    fn current_prompt(&self) -> Option<String> {
        let line = self
            .tail
            .rsplit(['\n', '\r'])
            .next()
            .unwrap_or_default()
            .trim_end();
        if line.is_empty() {
            return None;
        }
        prompt_regexes()
            .iter()
            .any(|regex| regex.is_match(line))
            .then(|| line.trim_start().to_string())
    }
}

/// 去掉 ANSI 转义（CSI/OSC/DCS 等）与其它不可见控制字符，保留换行和回车。
fn strip_control_sequences(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => skip_escape_sequence(&mut chars),
            '\u{7}' => {}
            ch if ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t' => {}
            ch => output.push(ch),
        }
    }
    output
}

fn skip_escape_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.next() {
        // CSI: 参数中间字节 0x30-0x3F，最终字节 0x40-0x7E。
        Some('[') => {
            for ch in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&ch) {
                    break;
                }
            }
        }
        // OSC / DCS / PM / APC：以 BEL 或 ST（ESC \）结束。
        Some(']') | Some('P') | Some('^') | Some('_') => {
            while let Some(ch) = chars.next() {
                if ch == '\u{7}' {
                    break;
                }
                if ch == '\u{1b}' {
                    if chars.peek() == Some(&'\\') {
                        chars.next();
                    }
                    break;
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elevation(auto_fill: Option<bool>, password: &str) -> SshElevationExpect {
        SshElevationExpect {
            auto_fill,
            password: password.to_string(),
        }
    }

    fn responder(login_password: Option<&str>) -> SessionPasswordExpect {
        SessionPasswordExpect::new(&elevation(None, ""), login_password, None)
    }

    fn fill_text(bytes: &[u8]) -> &str {
        std::str::from_utf8(bytes)
            .expect("utf-8 payload")
            .trim_end_matches('\r')
    }

    #[test]
    fn answers_sudo_prompt_after_the_command_echo() {
        let mut expect = responder(Some("secret"));
        let now = Instant::now();
        assert!(expect.observe("alice@host:~$ sudo -i\r\n", now).is_none());
        let payload = expect
            .observe(
                "[sudo] password for alice: ",
                now + Duration::from_millis(200),
            )
            .expect("sudo prompt answered");
        assert_eq!(fill_text(&payload), "secret");
        assert!(payload.ends_with(b"\r"));
    }

    #[test]
    fn answers_su_prompt_split_across_chunks() {
        let mut expect = responder(Some("root-pass"));
        let now = Instant::now();
        assert!(
            expect
                .observe("alice@host:~$ su - root\r\nPass", now)
                .is_none()
        );
        let payload = expect
            .observe("word:", now + Duration::from_millis(120))
            .expect("su prompt answered");
        assert_eq!(fill_text(&payload), "root-pass");
    }

    #[test]
    fn ignores_prompts_without_a_privilege_command() {
        let mut expect = responder(Some("secret"));
        let now = Instant::now();
        assert!(
            expect
                .observe("alice@host:~$ mysql -u root -p\r\nEnter password: ", now)
                .is_none(),
            "没有 sudo/su 时不能把密码灌进其它程序的提示"
        );
    }

    #[test]
    fn strips_ansi_sequences_before_matching() {
        let mut expect = responder(Some("secret"));
        let now = Instant::now();
        let _ = expect.observe("alice@host:~$ \u{1b}[1msudo\u{1b}[0m -i\r\n", now);
        let payload = expect
            .observe(
                "\u{1b}]0;title\u{7}\u{1b}[33m[sudo]\u{1b}[0m password for alice:\u{1b}[0m ",
                now + Duration::from_millis(80),
            )
            .expect("ansi-wrapped prompt answered");
        assert_eq!(fill_text(&payload), "secret");
    }

    #[test]
    fn answers_chained_prompts_for_sudo_su() {
        let mut expect = responder(Some("alice-pass"));
        let now = Instant::now();
        let _ = expect.observe("alice@host:~$ sudo su root\r\n", now);
        let first = expect
            .observe(
                "[sudo] password for alice: ",
                now + Duration::from_millis(100),
            )
            .expect("sudo prompt answered");
        assert_eq!(fill_text(&first), "alice-pass");
        let second = expect
            .observe("Password: ", now + Duration::from_millis(400))
            .expect("su prompt answered");
        assert_eq!(fill_text(&second), "alice-pass");
    }

    #[test]
    fn stops_after_a_rejected_prompt() {
        let mut expect = responder(Some("wrong"));
        let now = Instant::now();
        let _ = expect.observe("alice@host:~$ sudo -i\r\n", now);
        assert!(
            expect
                .observe(
                    "[sudo] password for alice: ",
                    now + Duration::from_millis(100)
                )
                .is_some()
        );
        assert!(
            expect
                .observe(
                    "Sorry, try again.\r\n[sudo] password for alice: ",
                    now + Duration::from_millis(600)
                )
                .is_none(),
            "密码被拒后不能继续填写"
        );
        assert!(
            expect
                .observe(
                    "[sudo] password for alice: ",
                    now + Duration::from_millis(900)
                )
                .is_none()
        );
    }

    #[test]
    fn does_not_answer_when_disabled() {
        let mut expect =
            SessionPasswordExpect::new(&elevation(Some(false), ""), Some("secret"), None);
        let now = Instant::now();
        let _ = expect.observe("alice@host:~$ sudo -i\r\n", now);
        assert!(
            expect
                .observe(
                    "[sudo] password for alice: ",
                    now + Duration::from_millis(100)
                )
                .is_none()
        );
    }

    #[test]
    fn does_not_answer_without_any_password() {
        let mut expect = responder(None);
        let now = Instant::now();
        let _ = expect.observe("alice@host:~$ sudo -i\r\n", now);
        assert!(
            expect
                .observe(
                    "[sudo] password for alice: ",
                    now + Duration::from_millis(100)
                )
                .is_none()
        );
    }

    #[test]
    fn password_defaults_to_the_login_password() {
        let mut expect = responder(Some("login-pass"));
        let now = Instant::now();
        let _ = expect.observe("alice@host:~$ su\r\n", now);
        let payload = expect
            .observe("Password: ", now + Duration::from_millis(100))
            .expect("prompt answered");
        assert_eq!(fill_text(&payload), "login-pass");
    }

    #[test]
    fn dedicated_elevation_password_wins() {
        let mut expect =
            SessionPasswordExpect::new(&elevation(None, "root-pass"), Some("login-pass"), None);
        let now = Instant::now();
        let _ = expect.observe("alice@host:~$ su root\r\n", now);
        let payload = expect
            .observe("Password: ", now + Duration::from_millis(100))
            .expect("prompt answered");
        assert_eq!(fill_text(&payload), "root-pass");
    }

    #[test]
    fn answers_chinese_prompt() {
        let mut expect = responder(Some("secret"));
        let now = Instant::now();
        let _ = expect.observe("alice@host:~$ sudo -i\r\n", now);
        let payload = expect
            .observe("密码：", now + Duration::from_millis(100))
            .expect("chinese prompt answered");
        assert_eq!(fill_text(&payload), "secret");
    }

    #[test]
    fn init_commands_with_sudo_arm_the_window() {
        let mut expect =
            SessionPasswordExpect::new(&elevation(None, ""), Some("secret"), Some("sudo -i"));
        let now = Instant::now();
        assert!(
            expect.observe("[sudo] password for alice: ", now).is_some(),
            "登录后命令自带 sudo 时无需等待回显"
        );
    }

    #[test]
    fn gives_up_after_the_fill_limit() {
        let mut expect = responder(Some("secret"));
        let mut now = Instant::now();
        let mut answered = 0;
        for index in 0..5 {
            let _ = expect.observe("alice@host:~$ sudo -i\r\n", now);
            if expect
                .observe(
                    &format!("[sudo] password for user{index}: "),
                    now + Duration::from_millis(100),
                )
                .is_some()
            {
                answered += 1;
            }
            now += Duration::from_secs(1);
        }
        assert_eq!(answered, MAX_FILLS as usize);
    }
}
