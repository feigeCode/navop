use crate::exec_capture::sanitize_captured_terminal_output;
use crate::osc::OscEvent;
use crate::{TerminalExecCompletion, TerminalExecOutput, TerminalExecRequest};
use std::time::{Duration, Instant};

mod model;
pub(crate) use model::{
    ExecEffect, ExecPhase, ShellCommandReadiness, TerminalExecError, TerminalInputSource,
};

const CLEAR_INPUT_TIMEOUT: Duration = Duration::from_secs(1);

struct ActiveExec {
    id: u64,
    request: TerminalExecRequest,
    phase: ExecPhase,
    started_at: Instant,
    raw: Vec<u8>,
    command_started: bool,
    detached: bool,
}

pub(crate) struct ExecSupervisor {
    readiness: ShellCommandReadiness,
    prompt_epoch: u64,
    command_epoch: u64,
    input_seq: u64,
    active: Option<ActiveExec>,
}

impl ExecSupervisor {
    pub(crate) fn new() -> Self {
        Self {
            readiness: ShellCommandReadiness::Initializing,
            prompt_epoch: 0,
            command_epoch: 0,
            input_seq: 0,
            active: None,
        }
    }

    pub(crate) fn readiness(&self) -> ShellCommandReadiness {
        self.readiness
    }

    pub(crate) fn start(&mut self, id: u64, request: TerminalExecRequest) -> Vec<ExecEffect> {
        if self.active.is_some() {
            return fail(id, TerminalExecError::Busy);
        }
        match self.readiness {
            ShellCommandReadiness::Ready { .. } => self.start_clear(id, request),
            ShellCommandReadiness::Disconnected => fail(id, TerminalExecError::Disconnected),
            ShellCommandReadiness::Initializing | ShellCommandReadiness::Unknown => {
                fail(id, TerminalExecError::ReadinessUnknown)
            }
            _ => fail(id, TerminalExecError::Busy),
        }
    }

    pub(crate) fn on_input(&mut self, source: TerminalInputSource, data: &[u8]) -> Vec<ExecEffect> {
        self.input_seq = self.input_seq.saturating_add(1);
        if source == TerminalInputSource::User && self.active_is_clearing() {
            let id = self.active.take().expect("active exec exists").id;
            self.readiness = ShellCommandReadiness::PromptRendering;
            return fail(id, TerminalExecError::ConcurrentUserInput);
        }
        if source == TerminalInputSource::User
            && data.iter().any(|byte| matches!(byte, b'\r' | b'\n'))
            && matches!(self.readiness, ShellCommandReadiness::Ready { .. })
        {
            self.command_epoch = self.command_epoch.saturating_add(1);
            self.readiness = ShellCommandReadiness::SubmissionPending {
                command_epoch: self.command_epoch,
            };
        }
        Vec::new()
    }

    pub(crate) fn on_osc(&mut self, event: &OscEvent) -> Vec<ExecEffect> {
        match event {
            OscEvent::PromptStart => self.on_prompt_start(),
            OscEvent::InputStart => self.on_input_start(),
            OscEvent::CommandStart => self.on_command_start(),
            OscEvent::CommandFinished { exit_code } => self.on_command_finished(*exit_code),
            _ => Vec::new(),
        }
    }

    pub(crate) fn cancel(&mut self, id: u64) -> Vec<ExecEffect> {
        let Some(active) = self.active.as_mut().filter(|active| active.id == id) else {
            return Vec::new();
        };
        match active.phase {
            ExecPhase::ClearingInput => {
                self.active = None;
                self.readiness = ShellCommandReadiness::PromptRendering;
                fail(id, TerminalExecError::CancelledBeforeSubmit)
            }
            ExecPhase::Observing => {
                active.detached = true;
                Vec::new()
            }
        }
    }

    fn start_clear(&mut self, id: u64, request: TerminalExecRequest) -> Vec<ExecEffect> {
        self.active = Some(ActiveExec {
            id,
            request,
            phase: ExecPhase::ClearingInput,
            started_at: Instant::now(),
            raw: Vec::new(),
            command_started: false,
            detached: false,
        });
        self.readiness = ShellCommandReadiness::ClearingInput { command_epoch: id };
        vec![
            ExecEffect::Write {
                source: TerminalInputSource::AgentPreflight,
                data: vec![0x03],
            },
            ExecEffect::ArmTimeout {
                id,
                phase: ExecPhase::ClearingInput,
                duration: CLEAR_INPUT_TIMEOUT,
            },
        ]
    }

    fn on_prompt_start(&mut self) -> Vec<ExecEffect> {
        if !self.active_is_clearing() {
            self.readiness = ShellCommandReadiness::PromptRendering;
        }
        Vec::new()
    }

    fn on_input_start(&mut self) -> Vec<ExecEffect> {
        self.prompt_epoch = self.prompt_epoch.saturating_add(1);
        self.readiness = ShellCommandReadiness::Ready {
            prompt_epoch: self.prompt_epoch,
        };
        if self.active_is_clearing() {
            return self.submit_active();
        }
        Vec::new()
    }

    fn submit_active(&mut self) -> Vec<ExecEffect> {
        let mut active = self.active.take().expect("clearing exec exists");
        let mut data = active.request.command.as_bytes().to_vec();
        if active.request.submit {
            data.push(b'\n');
        }
        let write = ExecEffect::Write {
            source: TerminalInputSource::AgentCommand,
            data,
        };
        if !active.request.submit || !active.request.wait_for_output {
            return vec![write, complete_submitted(&active)];
        }
        active.phase = ExecPhase::Observing;
        let id = active.id;
        let timeout = active.request.timeout;
        self.readiness = ShellCommandReadiness::SubmissionPending { command_epoch: id };
        self.active = Some(active);
        vec![
            write,
            ExecEffect::ArmTimeout {
                id,
                phase: ExecPhase::Observing,
                duration: timeout,
            },
        ]
    }

    fn on_command_start(&mut self) -> Vec<ExecEffect> {
        if let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.phase == ExecPhase::Observing)
        {
            active.command_started = true;
            self.readiness = ShellCommandReadiness::CommandRunning {
                command_epoch: active.id,
            };
        }
        Vec::new()
    }

    fn on_command_finished(&mut self, exit_code: i32) -> Vec<ExecEffect> {
        let Some(active) = self
            .active
            .as_ref()
            .filter(|active| active.phase == ExecPhase::Observing && active.command_started)
        else {
            return Vec::new();
        };
        let id = active.id;
        self.readiness = ShellCommandReadiness::AwaitingPrompt { command_epoch: id };
        let active = self.active.take().expect("observing exec exists");
        if active.detached {
            return Vec::new();
        }
        vec![ExecEffect::Complete {
            id,
            output: completed_output(&active, exit_code),
        }]
    }

    fn active_is_clearing(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.phase == ExecPhase::ClearingInput)
    }
}

fn fail(id: u64, error: TerminalExecError) -> Vec<ExecEffect> {
    vec![ExecEffect::Fail { id, error }]
}

fn complete_submitted(active: &ActiveExec) -> ExecEffect {
    ExecEffect::Complete {
        id: active.id,
        output: TerminalExecOutput {
            completion: TerminalExecCompletion::SubmittedOnly,
            exit_code: None,
            output: String::new(),
            duration_ms: elapsed_ms(active.started_at),
        },
    }
}

fn completed_output(active: &ActiveExec, exit_code: i32) -> TerminalExecOutput {
    TerminalExecOutput {
        completion: TerminalExecCompletion::ShellIntegrationExit,
        exit_code: Some(exit_code),
        output: sanitize_captured_terminal_output(&active.raw, &active.request.command),
        duration_ms: elapsed_ms(active.started_at),
    }
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
