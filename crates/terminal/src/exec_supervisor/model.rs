use crate::TerminalExecOutput;
use std::error::Error;
use std::fmt;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShellCommandReadiness {
    Initializing,
    PromptRendering,
    Ready { prompt_epoch: u64 },
    ClearingInput { command_epoch: u64 },
    SubmissionPending { command_epoch: u64 },
    CommandRunning { command_epoch: u64 },
    AwaitingPrompt { command_epoch: u64 },
    Unknown,
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalInputSource {
    User,
    ExternalInput,
    AgentPreflight,
    AgentCommand,
    TerminalResponse,
    InitCommand,
}

impl TerminalInputSource {
    pub(crate) fn affects_interactive_input_state(self) -> bool {
        matches!(self, Self::User | Self::ExternalInput)
    }

    pub(crate) fn is_recordable_user_input(self) -> bool {
        self == Self::User
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecPhase {
    WaitingForReady,
    ClearingInput,
    Observing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalExecError {
    Busy,
    /// Busy/ReadyTimeout 的上下文：拒绝瞬间的 shell readiness 快照 + 恢复指引，
    /// 让 Agent 不必盲猜是嵌套 shell、前台 TUI 还是提交挂起。
    BusyContext {
        readiness: String,
    },
    ReadinessUnknown,
    Disconnected,
    ReadyTimeout,
    ConcurrentUserInput,
    Cancelled,
    CancelledBeforeSubmit,
    ClearInputTimeout,
}

impl TerminalExecError {
    /// Busy 的人类可读恢复指引；readiness 快照由调用方拼入。
    fn recovery_hint(readiness: &str) -> String {
        let hint = format!(
            "terminal_busy (readiness={readiness}): the visible terminal is not at an \
             idle integrated prompt. If the user entered a nested shell (sudo -i, su) or a \
             TUI is occupying the prompt, call terminal.read to inspect the live prompt. When \
             shell integration is unavailable but the prompt is idle, use terminal.write_command \
             to write one command directly to the PTY, then call terminal.read to inspect its \
             visible output; or ask the user to exit the nested shell. To interact with a TUI, \
             use terminal.write_keys."
        );
        hint
    }
}

impl fmt::Display for TerminalExecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BusyContext { readiness } => formatter.write_str(&Self::recovery_hint(readiness)),
            Self::Busy => formatter.write_str("terminal_busy"),
            Self::ReadinessUnknown => formatter.write_str("readiness_unknown"),
            Self::Disconnected => formatter.write_str("terminal_disconnected"),
            Self::ReadyTimeout => formatter.write_str("ready_timeout"),
            Self::ConcurrentUserInput => formatter.write_str("concurrent_user_input"),
            Self::Cancelled => formatter.write_str("cancelled"),
            Self::CancelledBeforeSubmit => formatter.write_str("cancelled_before_submit"),
            Self::ClearInputTimeout => formatter.write_str("clear_input_timeout"),
        }
    }
}

impl Error for TerminalExecError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExecEffect {
    Write {
        source: TerminalInputSource,
        data: Vec<u8>,
    },
    Complete {
        id: u64,
        output: TerminalExecOutput,
    },
    Fail {
        id: u64,
        error: TerminalExecError,
    },
    ArmTimeout {
        id: u64,
        phase: ExecPhase,
        duration: Duration,
    },
}
