use crate::osc::OscEvent;
use crate::{TerminalExecCompletion, TerminalExecRequest};
use std::time::{Duration, Instant};

mod model;
mod operation;
pub use model::TerminalExecError;
pub(crate) use model::{ExecEffect, ExecPhase, ShellCommandReadiness, TerminalInputSource};
use operation::output_with_completion;

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
        if matches!(self.readiness, ShellCommandReadiness::Ready { .. }) {
            return self.start_clear(id, request);
        }
        if self.readiness == ShellCommandReadiness::Disconnected {
            return fail(id, TerminalExecError::Disconnected);
        }
        if self.readiness == ShellCommandReadiness::Unknown {
            return fail(id, TerminalExecError::ReadinessUnknown);
        }
        if !request.ready_timeout.is_zero() {
            return self.start_ready_wait(id, request);
        }
        fail(id, TerminalExecError::Busy)
    }

    pub(crate) fn on_input(&mut self, source: TerminalInputSource, data: &[u8]) -> Vec<ExecEffect> {
        self.input_seq = self.input_seq.saturating_add(1);
        let pre_submit_phase = self.active.as_ref().map(|active| active.phase);
        if source == TerminalInputSource::User
            && matches!(
                pre_submit_phase,
                Some(ExecPhase::WaitingForReady | ExecPhase::ClearingInput)
            )
        {
            let active = self.active.take().expect("pre-submit exec exists");
            if active.phase == ExecPhase::ClearingInput {
                self.readiness = ShellCommandReadiness::PromptRendering;
            }
            return fail(active.id, TerminalExecError::ConcurrentUserInput);
        }
        if matches!(
            source,
            TerminalInputSource::User | TerminalInputSource::InitCommand
        ) && data.iter().any(|byte| matches!(byte, b'\r' | b'\n'))
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

    pub(crate) fn on_terminal_chunk(
        &mut self,
        data: &[u8],
        events: &[OscEvent],
    ) -> Vec<ExecEffect> {
        if let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.phase == ExecPhase::Observing && !active.detached)
        {
            active.raw.extend_from_slice(data);
        }
        events.iter().flat_map(|event| self.on_osc(event)).collect()
    }

    pub(crate) fn cancel(&mut self, id: u64) -> Vec<ExecEffect> {
        let Some(active) = self.active.as_mut().filter(|active| active.id == id) else {
            return Vec::new();
        };
        match active.phase {
            ExecPhase::WaitingForReady => {
                self.active = None;
                fail(id, TerminalExecError::CancelledBeforeSubmit)
            }
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

    pub(crate) fn timeout(&mut self, id: u64, phase: ExecPhase) -> Vec<ExecEffect> {
        let Some(_) = self
            .active
            .as_ref()
            .filter(|active| active.id == id && active.phase == phase)
        else {
            return Vec::new();
        };
        if phase == ExecPhase::WaitingForReady {
            self.active = None;
            return fail(id, TerminalExecError::ReadyTimeout);
        }
        if phase == ExecPhase::ClearingInput {
            self.active = None;
            self.readiness = ShellCommandReadiness::Unknown;
            return fail(id, TerminalExecError::ClearInputTimeout);
        }
        let active = self.active.take().expect("timed out exec exists");
        if active.detached {
            return Vec::new();
        }
        vec![ExecEffect::Complete {
            id,
            output: output_with_completion(&active, TerminalExecCompletion::TimedOut, None),
        }]
    }

    pub(crate) fn disconnect(&mut self) -> Vec<ExecEffect> {
        self.readiness = ShellCommandReadiness::Disconnected;
        let Some(active) = self.active.take() else {
            return Vec::new();
        };
        if active.detached {
            Vec::new()
        } else {
            fail(active.id, TerminalExecError::Disconnected)
        }
    }
}

fn fail(id: u64, error: TerminalExecError) -> Vec<ExecEffect> {
    vec![ExecEffect::Fail { id, error }]
}

#[cfg(test)]
mod tests;
