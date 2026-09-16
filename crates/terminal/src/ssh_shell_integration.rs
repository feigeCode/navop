const MAX_SUPPRESSED_OUTPUT_BYTES: usize = 64 * 1024;
pub const SHELL_INTEGRATION_READY_MARKER: &[u8] = b"\x1b]1337;ShellIntegrationReady=1\x07";

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

fn runtime_shell_integration_body() -> String {
    let script = crate::shell_integration::embedded_shell_integration_script();
    let body = script.lines().skip(3).collect::<Vec<_>>().join("\n");
    format!(
        "{body}\nif [[ -n \"${{BASH_VERSION:-}}\" ]]; then\n    __onetcli_setup_history=\"$(history 2>/dev/null | while read -r __onetcli_history_number __onetcli_history_rest; do case \"$__onetcli_history_rest\" in *__ONETCLI_RUNTIME_SETUP_1*) printf '%s ' \"${{__onetcli_history_number%\\*}}\";; esac; done)\"\n    for __onetcli_history_number in $__onetcli_setup_history; do history -d \"$__onetcli_history_number\" 2>/dev/null; done\n    unset __onetcli_setup_history __onetcli_history_number __onetcli_history_rest\nfi\n__onetcli_emit_osc '1337;ShellIntegrationReady=1'",
        body = body,
    )
}

fn runtime_shell_integration_command() -> Vec<u8> {
    let body = ansi_c_quote(&runtime_shell_integration_body());
    format!(" _ONETCLI_RUNTIME_SETUP=1; : __ONETCLI_RUNTIME_SETUP_1; eval {body}\r").into_bytes()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellIntegrationReady {
    None,
    Integrated,
    Plain,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FilteredShellOutput {
    Suppressed,
    Forward {
        data: Vec<u8>,
        ready: ShellIntegrationReady,
    },
}

enum RuntimeShellIntegrationPhase {
    Disabled,
    WaitingForFirstOutput,
    Injecting { suppressed: Vec<u8> },
    AwaitingPrompt,
    Integrated,
    PlainAwaitingOutput,
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
        ) {
            self.phase = RuntimeShellIntegrationPhase::Injecting {
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
            RuntimeShellIntegrationPhase::Injecting { suppressed } => {
                suppressed.extend_from_slice(&data);
                if suppressed.len() > MAX_SUPPRESSED_OUTPUT_BYTES {
                    let retain_from = suppressed.len() - MAX_SUPPRESSED_OUTPUT_BYTES;
                    suppressed.drain(..retain_from);
                }
                let Some(marker_start) = find_subslice(suppressed, SHELL_INTEGRATION_READY_MARKER)
                else {
                    return FilteredShellOutput::Suppressed;
                };
                let suffix_start = marker_start + SHELL_INTEGRATION_READY_MARKER.len();
                let suffix = suppressed.split_off(suffix_start);
                suppressed.clear();
                self.phase = RuntimeShellIntegrationPhase::AwaitingPrompt;
                FilteredShellOutput::Forward {
                    data: suffix,
                    ready: ShellIntegrationReady::None,
                }
            }
            RuntimeShellIntegrationPhase::WaitingForFirstOutput => FilteredShellOutput::Forward {
                data,
                ready: ShellIntegrationReady::None,
            },
            RuntimeShellIntegrationPhase::Disabled
            | RuntimeShellIntegrationPhase::AwaitingPrompt
            | RuntimeShellIntegrationPhase::Integrated => FilteredShellOutput::Forward {
                data,
                ready: ShellIntegrationReady::None,
            },
            RuntimeShellIntegrationPhase::PlainAwaitingOutput => {
                self.phase = RuntimeShellIntegrationPhase::Plain;
                FilteredShellOutput::Forward {
                    data,
                    ready: ShellIntegrationReady::Plain,
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
                | RuntimeShellIntegrationPhase::AwaitingPrompt
        ) {
            self.phase = RuntimeShellIntegrationPhase::Integrated;
        }
    }

    pub fn on_timeout(&mut self) -> bool {
        if matches!(self.phase, RuntimeShellIntegrationPhase::Injecting { .. }) {
            self.phase = RuntimeShellIntegrationPhase::PlainAwaitingOutput;
            return true;
        }
        false
    }

    pub fn accepts_terminal_input(&self) -> bool {
        !matches!(
            self.phase,
            RuntimeShellIntegrationPhase::WaitingForFirstOutput
                | RuntimeShellIntegrationPhase::Injecting { .. }
                | RuntimeShellIntegrationPhase::AwaitingPrompt
                | RuntimeShellIntegrationPhase::PlainAwaitingOutput
        )
    }

    /// 当前阶段名，用于日志定位「输入被暂存」的原因。
    pub fn phase_name(&self) -> &'static str {
        match self.phase {
            RuntimeShellIntegrationPhase::Disabled => "disabled",
            RuntimeShellIntegrationPhase::WaitingForFirstOutput => "waiting-for-first-output",
            RuntimeShellIntegrationPhase::Injecting { .. } => "injecting",
            RuntimeShellIntegrationPhase::AwaitingPrompt => "awaiting-prompt",
            RuntimeShellIntegrationPhase::Integrated => "integrated",
            RuntimeShellIntegrationPhase::PlainAwaitingOutput => "plain-awaiting-output",
            RuntimeShellIntegrationPhase::Plain => "plain",
        }
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
            | RuntimeShellIntegrationPhase::Injecting { .. }
            | RuntimeShellIntegrationPhase::AwaitingPrompt
            | RuntimeShellIntegrationPhase::PlainAwaitingOutput => {
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
        assert!(command.contains("_ONETCLI_SHELL_INTEGRATED=1"));
        assert!(!command.contains(".config/onetcli"));
        assert!(!command.contains(".bashrc"));
        assert!(!command.contains(".zshrc"));
        assert!(!command.contains("mkdir"));
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
                ready: ShellIntegrationReady::None,
            },
            integration.filter_output(b"IntegrationReady=1\x07\x1b]133;B\x07tail".to_vec())
        );
        assert!(!integration.accepts_terminal_input());
        integration.on_input_start();
        assert!(integration.accepts_terminal_input());
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
        assert!(!integration.accepts_terminal_input());
        assert_eq!(
            FilteredShellOutput::Forward {
                data: b"next prompt".to_vec(),
                ready: ShellIntegrationReady::Plain,
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
    fn handshake_watchdog_releases_stalled_injection_and_prompt_wait() {
        // 注入回显未出现完成标记：握手停在 Injecting。
        let mut integration = RuntimeShellIntegration::new(true);
        integration.begin_injection();
        assert_eq!("injecting", integration.phase_name());
        assert!(integration.force_release_input());
        assert!(integration.accepts_terminal_input());

        // 找到完成标记后远端迟迟不给 OSC 133;B：握手停在 AwaitingPrompt。
        let mut integration = RuntimeShellIntegration::new(true);
        integration.begin_injection();
        assert_eq!(
            FilteredShellOutput::Suppressed,
            integration.filter_output(b"echoed setup without marker".to_vec())
        );
        let _ = integration.filter_output(
            b"\x1b]1337;ShellIntegrationReady=1\x07prompt".to_vec(),
        );
        assert_eq!("awaiting-prompt", integration.phase_name());
        assert!(!integration.accepts_terminal_input());
        assert!(integration.force_release_input());
        assert!(integration.accepts_terminal_input());
        assert_eq!("plain", integration.phase_name());

        // 注入超时后等待下一段输出：远端不再产出时同样需要兜底。
        let mut integration = RuntimeShellIntegration::new(true);
        integration.begin_injection();
        assert!(integration.on_timeout());
        assert_eq!("plain-awaiting-output", integration.phase_name());
        assert!(!integration.accepts_terminal_input());
        assert!(integration.force_release_input());
        assert!(integration.accepts_terminal_input());
    }

    #[test]
    fn handshake_watchdog_keeps_already_usable_phases_untouched() {
        let mut integration = RuntimeShellIntegration::new(false);
        assert!(!integration.force_release_input(), "未请求注入的会话无需降级");
        assert!(integration.accepts_terminal_input());

        let mut integration = RuntimeShellIntegration::new(true);
        integration.on_input_start();
        assert!(integration.is_integrated());
        assert!(!integration.force_release_input(), "已集成会话不应被看门狗降级");
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
        }
    }
}
