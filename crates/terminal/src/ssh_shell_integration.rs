const MAX_SUPPRESSED_OUTPUT_BYTES: usize = 64 * 1024;
pub const SHELL_INTEGRATION_READY_MARKER: &[u8] = b"\x1b]1337;ShellIntegrationReady=1\x07";
pub const SHELL_INTEGRATION_FAILED_MARKER: &[u8] = b"\x1b]1337;ShellIntegrationFailed=1\x07";
const SHELL_INTEGRATION_BOOTSTRAP_MARKER: &[u8] = b"\x1b]1337;ShellIntegrationBootstrap=1\x07";
const SHELL_INTEGRATION_BOOTSTRAP_COMMAND: &[u8] =
    b" _ONETCLI_RUNTIME_SETUP=1; : __ONETCLI_RUNTIME_SETUP_1; printf '\\033]1337;ShellIntegrationBootstrap=1\\007'\r";

/// 注入命令回显的开头（[`runtime_shell_integration_command`] 以空格开头）。
///
/// 握手静默期里，PTY 会先把我们注入的那整行命令回显出来，然后才执行它。回显
/// 前面的内容才是真正的首屏输出（pam_motd 打印的登录 banner / 欢迎信息），
/// 它只是在静默期里被连带吞掉了；握手完成时可以按这个前缀切出来还回去。
const INJECTION_ECHO_PREFIX: &[u8] = b" _ONETCLI_RUNTIME_SETUP=1";

/// 最后一个完整行的结束位置；末尾没有换行的半行（注入前的旧提示符）会被丢掉。
fn last_complete_line_end(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn ansi_c_quote(input: &str) -> String {
    let mut quoted = String::from("$'");
    for character in input.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '\'' => quoted.push_str("\\'"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            character => quoted.push(character),
        }
    }
    quoted.push('\'');
    quoted
}

/// 兼容各平台终端的行内输入上限：N_TTY 的输入队列只有 4 KiB，一次写入超过它
/// 的部分会被内核直接丢弃（表现为远端 bash 停在 `>` 续行提示符上、Ready 标记
/// 永不出现，5 秒后降级）。留出余量取 4000 字节。
const MAX_INJECTION_COMMAND_BYTES: usize = 4000;

/// 去掉缩进、空行与整行注释，把注入脚本压到 PTY 单次输入队列能装下的长度。
///
/// 脚本里没有续行符、heredoc 或多行字符串，因此按行去缩进是安全的。
fn minified_shell_integration_body(body: &str) -> String {
    body.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            (!trimmed.is_empty() && !trimmed.starts_with('#')).then_some(trimmed)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn runtime_shell_integration_body() -> String {
    let script = crate::shell_integration::embedded_shell_integration_script();
    let body =
        minified_shell_integration_body(&script.lines().skip(3).collect::<Vec<_>>().join("\n"));
    format!(
        "{body}\nif [[ -n \"${{BASH_VERSION:-}}\" ]]; then\n    __onetcli_setup_history=\"$(history 2>/dev/null | while read -r __onetcli_history_number __onetcli_history_rest; do case \"$__onetcli_history_rest\" in *__ONETCLI_RUNTIME_SETUP_1*) printf '%s ' \"${{__onetcli_history_number%\\*}}\";; esac; done)\"\n    for __onetcli_history_number in $__onetcli_setup_history; do history -d \"$__onetcli_history_number\" 2>/dev/null; done\n    unset __onetcli_setup_history __onetcli_history_number __onetcli_history_rest\nfi",
        body = body,
    )
}

pub(crate) fn runtime_shell_integration_command() -> Vec<u8> {
    let command = runtime_shell_integration_command_for_body(&runtime_shell_integration_body());
    // 超出 PTY 输入队列的命令会被内核截断：远端停在 `>` 续行提示符上，Ready 标记
    // 永不出现，连接要卡满 5 秒兜底才降级。脚本变长时这里会先报警。
    if command.len() > MAX_INJECTION_COMMAND_BYTES {
        tracing::warn!(
            target: "terminal.ssh.setup",
            len = command.len(),
            limit = MAX_INJECTION_COMMAND_BYTES,
            "运行时 Shell Integration 注入命令超过 PTY 输入队列上限，远端可能截断"
        );
    }
    command
}

fn runtime_shell_integration_command_for_body(body: &str) -> Vec<u8> {
    let body = ansi_c_quote(body);
    format!(
        " _ONETCLI_RUNTIME_SETUP=1; : __ONETCLI_RUNTIME_SETUP_1; \
         if eval {body}; then \
             printf '\\033]1337;ShellIntegrationReady=1\\007'; \
         else \
             printf '\\033]1337;ShellIntegrationFailed=1\\007'; \
         fi; \
         :\r"
    )
    .into_bytes()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellIntegrationReady {
    None,
    Integrated,
    Failed,
    Plain,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FilteredShellOutput {
    Suppressed,
    BootstrapReady,
    Forward {
        data: Vec<u8>,
        ready: ShellIntegrationReady,
    },
}

enum RuntimeShellIntegrationPhase {
    Disabled,
    WaitingForFirstOutput,
    Bootstrapping {
        suppressed: Vec<u8>,
    },
    BootstrapReady,
    Injecting {
        suppressed: Vec<u8>,
        /// 静默缓冲超过上限被截断时，首屏已被切掉一段，不再尝试还原。
        overflowed: bool,
    },
    Integrated,
    Plain,
}

/// 供真机集成测试（tests/ 目录）复用的运行时注入状态机入口。
pub mod test_support {
    pub use super::{
        FilteredShellOutput, RuntimeShellIntegration, SHELL_INTEGRATION_READY_MARKER,
        ShellIntegrationReady,
    };
}

pub struct RuntimeShellIntegration {
    phase: RuntimeShellIntegrationPhase,
    command: Vec<u8>,
}

impl RuntimeShellIntegration {
    pub fn new(requested: bool) -> Self {
        Self {
            phase: if requested {
                RuntimeShellIntegrationPhase::WaitingForFirstOutput
            } else {
                RuntimeShellIntegrationPhase::Disabled
            },
            command: runtime_shell_integration_command(),
        }
    }

    pub fn injection_command(&self) -> &[u8] {
        &self.command
    }

    pub fn bootstrap_command(&self) -> &'static [u8] {
        SHELL_INTEGRATION_BOOTSTRAP_COMMAND
    }

    pub fn should_inject(&self, data: &[u8], login_complete: bool, expect_responded: bool) -> bool {
        !data.is_empty()
            && login_complete
            && !expect_responded
            && matches!(
                self.phase,
                RuntimeShellIntegrationPhase::WaitingForFirstOutput
            )
    }

    pub fn begin_injection(&mut self) {
        if matches!(
            self.phase,
            RuntimeShellIntegrationPhase::WaitingForFirstOutput
                | RuntimeShellIntegrationPhase::BootstrapReady
        ) {
            self.phase = RuntimeShellIntegrationPhase::Injecting {
                suppressed: Vec::new(),
                overflowed: false,
            };
        }
    }

    pub fn begin_bootstrap(&mut self) {
        if matches!(
            self.phase,
            RuntimeShellIntegrationPhase::WaitingForFirstOutput
        ) {
            self.phase = RuntimeShellIntegrationPhase::Bootstrapping {
                suppressed: Vec::new(),
            };
        }
    }

    pub fn mark_existing_ready(&mut self) {
        if matches!(
            self.phase,
            RuntimeShellIntegrationPhase::WaitingForFirstOutput
        ) {
            self.phase = RuntimeShellIntegrationPhase::Integrated;
        }
    }

    pub fn is_injecting(&self) -> bool {
        matches!(self.phase, RuntimeShellIntegrationPhase::Injecting { .. })
    }

    /// 是否已确认进入集成态（含远端已有 rc 注入的老 session 场景）。
    pub fn is_integrated(&self) -> bool {
        matches!(self.phase, RuntimeShellIntegrationPhase::Integrated)
    }

    pub fn filter_output(&mut self, data: Vec<u8>) -> FilteredShellOutput {
        match &mut self.phase {
            RuntimeShellIntegrationPhase::Bootstrapping { suppressed } => {
                suppressed.extend_from_slice(&data);
                if suppressed.len() > MAX_SUPPRESSED_OUTPUT_BYTES {
                    let retain_from = suppressed.len() - MAX_SUPPRESSED_OUTPUT_BYTES;
                    suppressed.drain(..retain_from);
                }
                if find_subslice(suppressed, SHELL_INTEGRATION_BOOTSTRAP_MARKER).is_some() {
                    self.phase = RuntimeShellIntegrationPhase::BootstrapReady;
                    return FilteredShellOutput::BootstrapReady;
                }
                FilteredShellOutput::Suppressed
            }
            RuntimeShellIntegrationPhase::Injecting {
                suppressed,
                overflowed,
            } => {
                suppressed.extend_from_slice(&data);
                if suppressed.len() > MAX_SUPPRESSED_OUTPUT_BYTES {
                    let retain_from = suppressed.len() - MAX_SUPPRESSED_OUTPUT_BYTES;
                    suppressed.drain(..retain_from);
                    *overflowed = true;
                }
                let ready_marker = find_subslice(suppressed, SHELL_INTEGRATION_READY_MARKER)
                    .map(|start| (start, SHELL_INTEGRATION_READY_MARKER.len()));
                let failed_marker = find_subslice(suppressed, SHELL_INTEGRATION_FAILED_MARKER)
                    .map(|start| (start, SHELL_INTEGRATION_FAILED_MARKER.len()));
                let Some((marker_start, marker_len, ready)) = (match (ready_marker, failed_marker) {
                    (Some(ready), Some(failed)) if ready.0 <= failed.0 => {
                        Some((ready.0, ready.1, ShellIntegrationReady::Integrated))
                    }
                    (Some(_), Some(failed)) => {
                        Some((failed.0, failed.1, ShellIntegrationReady::Failed))
                    }
                    (Some(ready), None) => {
                        Some((ready.0, ready.1, ShellIntegrationReady::Integrated))
                    }
                    (None, Some(failed)) => {
                        Some((failed.0, failed.1, ShellIntegrationReady::Failed))
                    }
                    (None, None) => None,
                }) else {
                    return FilteredShellOutput::Suppressed;
                };
                // 注入回显之前的内容是真实首屏（登录 banner 等），只是被静默期连带
                // 吞掉了：这里原样还回去，同时丢掉回显本身、以及注入前那半截旧
                // 提示符行，避免多出一行残留 prompt。
                let restored = if *overflowed {
                    Vec::new()
                } else {
                    find_subslice(&suppressed[..marker_start], INJECTION_ECHO_PREFIX)
                        .map(|echo_start| {
                            let keep = last_complete_line_end(&suppressed[..echo_start]);
                            suppressed[..keep].to_vec()
                        })
                        .unwrap_or_default()
                };
                let suffix_start = marker_start + marker_len;
                let suffix = suppressed.split_off(suffix_start);
                suppressed.clear();
                self.phase = match ready {
                    // ready marker 由集成脚本在所有 hook 安装完成后主动发出。
                    // 此时脚本已经执行成功，不应再等未必出现的 OSC 133;B。
                    ShellIntegrationReady::Integrated => RuntimeShellIntegrationPhase::Integrated,
                    // 注入命令返回失败时直接降级为裸终端，不能继续扣住输入等 5s。
                    ShellIntegrationReady::Failed => RuntimeShellIntegrationPhase::Plain,
                    ShellIntegrationReady::None | ShellIntegrationReady::Plain => unreachable!(),
                };
                FilteredShellOutput::Forward {
                    data: [restored.as_slice(), suffix.as_slice()].concat(),
                    ready,
                }
            }
            RuntimeShellIntegrationPhase::WaitingForFirstOutput => FilteredShellOutput::Forward {
                data,
                ready: ShellIntegrationReady::None,
            },
            RuntimeShellIntegrationPhase::BootstrapReady => FilteredShellOutput::Suppressed,
            RuntimeShellIntegrationPhase::Disabled | RuntimeShellIntegrationPhase::Integrated => {
                FilteredShellOutput::Forward {
                    data,
                    ready: ShellIntegrationReady::None,
                }
            }
            RuntimeShellIntegrationPhase::Plain => FilteredShellOutput::Forward {
                data,
                ready: ShellIntegrationReady::None,
            },
        }
    }

    pub fn on_input_start(&mut self) {
        if matches!(
            self.phase,
            RuntimeShellIntegrationPhase::WaitingForFirstOutput
        ) {
            self.phase = RuntimeShellIntegrationPhase::Integrated;
        }
    }

    pub fn on_timeout(&mut self) -> bool {
        if matches!(
            self.phase,
            RuntimeShellIntegrationPhase::Bootstrapping { .. }
                | RuntimeShellIntegrationPhase::BootstrapReady
                | RuntimeShellIntegrationPhase::Injecting { .. }
        ) {
            self.phase = RuntimeShellIntegrationPhase::Plain;
            return true;
        }
        false
    }

    pub fn accepts_terminal_input(&self) -> bool {
        !matches!(
            self.phase,
            RuntimeShellIntegrationPhase::WaitingForFirstOutput
                | RuntimeShellIntegrationPhase::Bootstrapping { .. }
                | RuntimeShellIntegrationPhase::BootstrapReady
                | RuntimeShellIntegrationPhase::Injecting { .. }
        )
    }

    /// 当前阶段名，用于日志定位「输入被暂存」的原因。
    pub fn phase_name(&self) -> &'static str {
        match self.phase {
            RuntimeShellIntegrationPhase::Disabled => "disabled",
            RuntimeShellIntegrationPhase::WaitingForFirstOutput => "waiting-for-first-output",
            RuntimeShellIntegrationPhase::Bootstrapping { .. } => "bootstrapping",
            RuntimeShellIntegrationPhase::BootstrapReady => "bootstrap-ready",
            RuntimeShellIntegrationPhase::Injecting { .. } => "injecting",
            RuntimeShellIntegrationPhase::Integrated => "integrated",
            RuntimeShellIntegrationPhase::Plain => "plain",
        }
    }

    /// 注入静默期内的原始输出，仅供真机诊断测试定位缺失 marker 的原因。
    pub fn suppressed_output(&self) -> &[u8] {
        match &self.phase {
            RuntimeShellIntegrationPhase::Bootstrapping { suppressed } => suppressed,
            RuntimeShellIntegrationPhase::Injecting { suppressed, .. } => suppressed,
            _ => &[],
        }
    }

    pub fn saw_injection_echo(&self) -> bool {
        find_subslice(self.suppressed_output(), INJECTION_ECHO_PREFIX).is_some()
    }

    /// 看门狗兜底：把仍会暂存用户输入的握手态强制推进到裸终端。
    ///
    /// 握手态只能靠远端输出推进（`should_inject` / ready marker / OSC 133;B）。
    /// 远端不再产生可解除握手的输出时（登录 expect 永不完成、prompt 结束标记
    /// 缺失等），用户输入会被永久暂存，正是 Issue #206 描述的现象。返回是否
    /// 改变了状态；已可输入的阶段不会被降级。
    pub fn force_release_input(&mut self) -> bool {
        match self.phase {
            RuntimeShellIntegrationPhase::WaitingForFirstOutput
            | RuntimeShellIntegrationPhase::Bootstrapping { .. }
            | RuntimeShellIntegrationPhase::BootstrapReady
            | RuntimeShellIntegrationPhase::Injecting { .. } => {
                self.phase = RuntimeShellIntegrationPhase::Plain;
                true
            }
            RuntimeShellIntegrationPhase::Disabled
            | RuntimeShellIntegrationPhase::Integrated
            | RuntimeShellIntegrationPhase::Plain => false,
        }
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_command_is_one_line_and_keeps_navop_osc_protocol() {
        let integration = RuntimeShellIntegration::new(true);
        let bootstrap = String::from_utf8(integration.bootstrap_command().to_vec())
            .expect("bootstrap command must be UTF-8 shell text");
        assert!(bootstrap.starts_with(' '));
        assert!(bootstrap.ends_with('\r'));
        assert!(!bootstrap[..bootstrap.len() - 1].contains('\n'));
        assert!(bootstrap.contains("ShellIntegrationBootstrap=1"));

        let command = String::from_utf8(integration.injection_command().to_vec())
            .expect("runtime command must be UTF-8 shell text");

        assert!(command.starts_with(' '));
        assert!(command.ends_with('\r'));
        assert!(!command[..command.len() - 1].contains('\n'));
        assert!(command.contains("__ONETCLI_RUNTIME_SETUP_1"));
        assert!(command.contains("eval $'"));
        assert!(command.contains("133;A"));
        assert!(command.contains("1337;Command="));
        assert!(command.contains("ShellIntegrationReady=1"));
        assert!(command.contains("ShellIntegrationFailed=1"));
        assert!(command.contains("if eval"));
        assert!(command.contains("_ONETCLI_SHELL_INTEGRATED=1"));
        assert!(!command.contains(".config/onetcli"));
        assert!(!command.contains(".bashrc"));
        assert!(!command.contains(".zshrc"));
        assert!(!command.contains("mkdir"));
    }

    #[test]
    fn runtime_command_fits_in_the_pty_input_queue() {
        // 真机（CentOS 7 上的 bash 4.2）复现过：注入命令 6285 字节，超过 N_TTY
        // 的 4 KiB 输入队列后被内核截断，远端停在 `>` 续行提示符，Ready 标记
        // 永不出现 → 连接卡满 5 秒兜底。
        let command = runtime_shell_integration_command();
        assert!(
            command.len() <= MAX_INJECTION_COMMAND_BYTES,
            "注入命令 {} 字节，超过 PTY 输入队列安全上限 {} 字节",
            command.len(),
            MAX_INJECTION_COMMAND_BYTES
        );
        let command = String::from_utf8(command).expect("runtime command must be UTF-8 shell text");
        // 压缩脚本只去掉缩进/空行/整行注释，钩子与 trap 必须原样保留。
        assert!(command.contains("__onetcli_preexec_bash"));
        assert!(command.contains("precmd_functions+=(__onetcli_precmd_zsh)"));
        assert!(command.contains("__onetcli_update_cwd"));
        assert!(command.contains("trap "));
    }

    #[test]
    fn split_runtime_echo_is_hidden_until_completion_marker() {
        let mut integration = RuntimeShellIntegration::new(true);
        assert!(integration.should_inject(b"welcome\r\nuser@host$ ", true, false));
        integration.begin_injection();

        assert_eq!(
            FilteredShellOutput::Suppressed,
            integration.filter_output(b" echoed internal setup\r\n".to_vec())
        );
        assert_eq!(
            FilteredShellOutput::Suppressed,
            integration.filter_output(b"\x1b]133;A\x07user@host$ \x1b]1337;Shell".to_vec())
        );

        assert_eq!(
            FilteredShellOutput::Forward {
                data: b"\x1b]133;B\x07tail".to_vec(),
                ready: ShellIntegrationReady::Integrated,
            },
            integration.filter_output(b"IntegrationReady=1\x07\x1b]133;B\x07tail".to_vec())
        );
        assert!(integration.accepts_terminal_input());
        assert!(integration.is_integrated());
        integration.on_input_start();
        assert!(integration.accepts_terminal_input());
        assert!(integration.is_integrated());
    }

    #[test]
    fn bootstrap_marker_gates_full_injection_until_shell_is_ready() {
        let mut integration = RuntimeShellIntegration::new(true);
        assert!(integration.should_inject(b"login banner", true, false));
        integration.begin_bootstrap();
        assert_eq!("bootstrapping", integration.phase_name());
        assert!(!integration.accepts_terminal_input());
        assert_eq!(
            FilteredShellOutput::Suppressed,
            integration.filter_output(b"bootstrapped setup echo".to_vec())
        );

        let split = SHELL_INTEGRATION_BOOTSTRAP_MARKER.len() / 2;
        assert_eq!(
            FilteredShellOutput::Suppressed,
            integration.filter_output(SHELL_INTEGRATION_BOOTSTRAP_MARKER[..split].to_vec())
        );
        assert_eq!(
            FilteredShellOutput::BootstrapReady,
            integration.filter_output(SHELL_INTEGRATION_BOOTSTRAP_MARKER[split..].to_vec())
        );
        assert_eq!("bootstrap-ready", integration.phase_name());
        assert!(!integration.accepts_terminal_input());

        integration.begin_injection();
        assert_eq!("injecting", integration.phase_name());
        assert!(!integration.should_inject(b"prompt", true, false));
    }

    /// 握手静默期里被吞掉的登录 banner（pam_motd 在注入前打印的那几行）必须还回去，
    /// 同时不能把注入命令回显和注入前的旧提示符残行带到屏幕上。
    #[test]
    fn ready_marker_restores_login_banner_swallowed_during_handshake() {
        let mut integration = RuntimeShellIntegration::new(true);
        assert!(integration.should_inject(b"Welcome\r\n", true, false));
        integration.begin_injection();

        assert_eq!(
            FilteredShellOutput::Suppressed,
            integration
                .filter_output(b"Last login: Sun Nov  9 09:50:00 2026 from 10.0.0.2\r\n".to_vec())
        );
        // 注入行回显 + 注入前那半截旧提示符（没有换行，必须丢掉）
        assert_eq!(
            FilteredShellOutput::Suppressed,
            integration.filter_output(
                b"user@host:~$  _ONETCLI_RUNTIME_SETUP=1; : __ONETCLI_RUNTIME_SETUP_1; eval $'x'\r\n"
                    .to_vec()
            )
        );

        assert_eq!(
            FilteredShellOutput::Forward {
                data: b"Last login: Sun Nov  9 09:50:00 2026 from 10.0.0.2\r\n\
                        \x1b]133;A\x07user@host:~$ "
                    .to_vec(),
                ready: ShellIntegrationReady::Integrated,
            },
            integration.filter_output(
                b"\x1b]1337;ShellIntegrationReady=1\x07\x1b]133;A\x07user@host:~$ ".to_vec()
            )
        );
        assert_eq!("integrated", integration.phase_name());
        assert!(integration.accepts_terminal_input());
    }

    /// 静默缓冲被截断时首屏已经不完整，宁可不还原，也不要吐半截内容。
    #[test]
    fn overflowed_suppression_window_skips_banner_restore() {
        let mut integration = RuntimeShellIntegration::new(true);
        integration.begin_injection();

        let mut chunk = vec![b'x'; MAX_SUPPRESSED_OUTPUT_BYTES + 4096];
        chunk.extend_from_slice(b"\n _ONETCLI_RUNTIME_SETUP=1; eval $'x'\r\n");
        chunk.extend_from_slice(SHELL_INTEGRATION_READY_MARKER);
        chunk.extend_from_slice(b"prompt");
        assert_eq!(
            FilteredShellOutput::Forward {
                data: b"prompt".to_vec(),
                ready: ShellIntegrationReady::Integrated,
            },
            integration.filter_output(chunk)
        );
    }

    /// ready marker 已经证明集成脚本执行完成，不能再等未必出现的 133;B。
    /// 提示符框架可以把 133;B 冲掉，若握手仍依赖它，用户每次连接都会白等宽限期。
    #[test]
    fn ready_marker_releases_input_without_waiting_for_prompt_marker() {
        let mut integration = RuntimeShellIntegration::new(true);
        integration.begin_injection();
        // ready marker 与回显同批到达时，本批就只剩 marker 之后的空后缀。
        assert_eq!(
            FilteredShellOutput::Forward {
                data: Vec::new(),
                ready: ShellIntegrationReady::Integrated,
            },
            integration.filter_output(b"echoed\r\n\x1b]1337;ShellIntegrationReady=1\x07".to_vec())
        );
        assert_eq!("integrated", integration.phase_name());
        assert!(integration.accepts_terminal_input());
        assert!(integration.is_integrated());

        // 之后真正到达的 133;B 只用于命令生命周期事件，不会把状态降级回握手态。
        integration.on_input_start();
        assert!(integration.is_integrated());
        assert!(!integration.force_release_input());
    }

    #[test]
    fn injection_failure_marker_releases_input_and_falls_back_to_plain() {
        let mut integration = RuntimeShellIntegration::new(true);
        integration.begin_injection();
        assert_eq!(
            FilteredShellOutput::Suppressed,
            integration.filter_output(b" _ONETCLI_RUNTIME_SETUP=1; echoed setup".to_vec())
        );
        assert!(integration.saw_injection_echo());
        assert_eq!(
            b" _ONETCLI_RUNTIME_SETUP=1; echoed setup".len(),
            integration.suppressed_output().len()
        );
        assert_eq!(
            FilteredShellOutput::Forward {
                data: b"prompt".to_vec(),
                ready: ShellIntegrationReady::Failed,
            },
            integration.filter_output(b"\x1b]1337;ShellIntegrationFailed=1\x07prompt".to_vec())
        );
        assert_eq!("plain", integration.phase_name());
        assert!(integration.accepts_terminal_input());
        assert!(!integration.is_integrated());
    }

    #[test]
    fn timeout_releases_suppressed_output_and_falls_back_to_plain_shell() {
        let mut integration = RuntimeShellIntegration::new(true);
        integration.begin_injection();
        assert_eq!(
            FilteredShellOutput::Suppressed,
            integration.filter_output(b"echoed prompt".to_vec())
        );

        assert!(integration.on_timeout());
        assert!(integration.accepts_terminal_input());
        assert_eq!("plain", integration.phase_name());
        assert_eq!(
            FilteredShellOutput::Forward {
                data: b"next prompt".to_vec(),
                ready: ShellIntegrationReady::None,
            },
            integration.filter_output(b"next prompt".to_vec())
        );
        assert!(integration.accepts_terminal_input());
    }

    #[test]
    fn disabled_integration_forwards_everything_and_accepts_input_immediately() {
        let mut integration = RuntimeShellIntegration::new(false);
        assert!(!integration.should_inject(b"prompt", true, false));
        assert!(integration.accepts_terminal_input());
        assert_eq!(
            FilteredShellOutput::Forward {
                data: b"prompt".to_vec(),
                ready: ShellIntegrationReady::None,
            },
            integration.filter_output(b"prompt".to_vec())
        );
    }

    #[test]
    fn handshake_watchdog_releases_stalled_waiting_for_first_output() {
        // 登录 expect 未完成时 should_inject 永不触发，输入必须能被兜底放行。
        let mut integration = RuntimeShellIntegration::new(true);
        assert_eq!("waiting-for-first-output", integration.phase_name());
        assert!(!integration.accepts_terminal_input());

        assert!(integration.force_release_input());
        assert!(integration.accepts_terminal_input());
        assert_eq!("plain", integration.phase_name());
        assert!(
            !integration.should_inject(b"prompt", true, false),
            "强制放行后不应再注入，否则暂存的用户输入会与注入命令混在一起"
        );
    }

    #[test]
    fn handshake_watchdog_releases_stalled_injection() {
        // 注入回显未出现完成标记：握手停在 Injecting。
        let mut integration = RuntimeShellIntegration::new(true);
        integration.begin_injection();
        assert_eq!("injecting", integration.phase_name());
        assert!(integration.force_release_input());
        assert!(integration.accepts_terminal_input());

        // 注入超时直接进入 Plain，不能继续等下一段输出才释放输入。
        let mut integration = RuntimeShellIntegration::new(true);
        integration.begin_injection();
        assert!(integration.on_timeout());
        assert_eq!("plain", integration.phase_name());
        assert!(integration.accepts_terminal_input());
    }

    #[test]
    fn handshake_watchdog_keeps_already_usable_phases_untouched() {
        let mut integration = RuntimeShellIntegration::new(false);
        assert!(
            !integration.force_release_input(),
            "未请求注入的会话无需降级"
        );
        assert!(integration.accepts_terminal_input());

        let mut integration = RuntimeShellIntegration::new(true);
        integration.on_input_start();
        assert!(integration.is_integrated());
        assert!(
            !integration.force_release_input(),
            "已集成会话不应被看门狗降级"
        );
        assert!(integration.is_integrated());
        assert!(integration.accepts_terminal_input());
    }

    #[test]
    fn login_incomplete_or_expect_round_defers_injection() {
        let integration = RuntimeShellIntegration::new(true);
        assert!(!integration.should_inject(b"login:", false, false));
        assert!(!integration.should_inject(b"welcome", true, true));
        assert!(integration.should_inject(b"prompt", true, false));
    }

    #[test]
    fn existing_integration_from_legacy_rc_block_is_detected_via_input_start() {
        // 远端 rc 已带旧版持久注入的 session：首个 prompt 就会发出 OSC 133;B。
        let mut integration = RuntimeShellIntegration::new(true);
        integration.on_input_start();
        assert!(integration.is_integrated());
        assert!(!integration.should_inject(b"prompt", true, false));
    }

    #[cfg(unix)]
    #[test]
    fn runtime_command_parses_and_reports_ready_in_real_bash_and_zsh() {
        let integration = RuntimeShellIntegration::new(true);
        let command = String::from_utf8(integration.injection_command().to_vec()).unwrap();
        let command = command.trim_end_matches('\r');

        let shells: [(&str, &[&str]); 2] = [
            ("/bin/bash", &["--noprofile", "--norc"]),
            ("/bin/zsh", &["--no-rcs"]),
        ];
        for (shell, extra_flags) in shells {
            if !std::path::Path::new(shell).exists() {
                continue;
            }
            let mut command_runner = std::process::Command::new(shell);
            command_runner
                .args(extra_flags)
                .arg("-c")
                .arg(command)
                .env("HISTFILE", "/dev/null");
            let output = command_runner
                .output()
                .expect("应能启动真实 shell 执行注入命令");
            assert!(
                output.status.success(),
                "{shell} 应接受注入命令: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                stdout.contains("1337;ShellIntegrationReady=1"),
                "{shell} 应输出完成标记，实际: {stdout:?}"
            );

            let mut interactive_runner = std::process::Command::new(shell);
            interactive_runner
                .args(extra_flags)
                .args(["-i", "-c", command])
                .env("HISTFILE", "/dev/null")
                .stdin(std::process::Stdio::null());
            let output = interactive_runner
                .output()
                .expect("应能在交互模式下启动真实 shell 执行注入命令");
            assert!(
                output.status.success(),
                "{shell} 交互模式应接受注入命令: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                stdout.contains("1337;ShellIntegrationReady=1"),
                "{shell} 交互模式应输出完成标记，实际: {stdout:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn runtime_command_emits_failure_marker_when_eval_fails() {
        let shell = "/bin/bash";
        if !std::path::Path::new(shell).exists() {
            return;
        }
        let command = runtime_shell_integration_command_for_body("false");
        let command = String::from_utf8(command).unwrap();
        let command = command.trim_end_matches('\r');
        let output = std::process::Command::new(shell)
            .args(["--noprofile", "--norc", "-c", command])
            .output()
            .expect("应能启动真实 bash 执行失败注入命令");
        assert!(
            output.status.success(),
            "外层命令应正常处理 eval 失败: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("1337;ShellIntegrationFailed=1"),
            "eval 失败时必须输出失败标记，实际: {stdout:?}"
        );
        assert!(
            !stdout.contains("1337;ShellIntegrationReady=1"),
            "eval 失败时不能误报 ready，实际: {stdout:?}"
        );
    }
}
