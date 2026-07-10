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
    AgentPreflight,
    AgentCommand,
    TerminalResponse,
    InitCommand,
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
    ReadinessUnknown,
    Disconnected,
    ReadyTimeout,
    ConcurrentUserInput,
    Cancelled,
    CancelledBeforeSubmit,
    ClearInputTimeout,
}

impl fmt::Display for TerminalExecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Busy => "terminal_busy",
            Self::ReadinessUnknown => "readiness_unknown",
            Self::Disconnected => "terminal_disconnected",
            Self::ReadyTimeout => "ready_timeout",
            Self::ConcurrentUserInput => "concurrent_user_input",
            Self::Cancelled => "cancelled",
            Self::CancelledBeforeSubmit => "cancelled_before_submit",
            Self::ClearInputTimeout => "clear_input_timeout",
        })
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
