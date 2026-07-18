use crate::osc::OscEvent;
use crate::{
    TerminalControlError, TerminalControlReadiness, TerminalExecCompletion, TerminalExecObserver,
    TerminalExecProgress, TerminalExecRequest,
};
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
    timed_out: bool,
    observer: Option<TerminalExecObserver>,
}

pub(crate) struct ExecSupervisor {
    readiness: ShellCommandReadiness,
    prompt_epoch: u64,
    command_epoch: u64,
    input_dirty: bool,
    active: Option<ActiveExec>,
}

impl ExecSupervisor {
    pub(crate) fn new() -> Self {
        Self {
            readiness: ShellCommandReadiness::Initializing,
            prompt_epoch: 0,
            command_epoch: 0,
            input_dirty: false,
            active: None,
        }
    }

    pub(crate) fn readiness(&self) -> ShellCommandReadiness {
        self.readiness
    }

    pub(crate) fn captures_terminal_output(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.phase == ExecPhase::Observing && !active.detached)
    }

    pub(crate) fn interrupt_foreground(
        &self,
    ) -> Result<TerminalControlReadiness, TerminalControlError> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.phase == ExecPhase::WaitingForReady)
        {
            return Err(TerminalControlError::Busy);
        }
        match self.readiness {
            ShellCommandReadiness::SubmissionPending { .. } => {
                Ok(TerminalControlReadiness::SubmissionPending)
            }
            ShellCommandReadiness::CommandRunning { .. } => {
                Ok(TerminalControlReadiness::CommandRunning)
            }
            ShellCommandReadiness::Ready { .. } | ShellCommandReadiness::AwaitingPrompt { .. } => {
                Err(TerminalControlError::NotRunning)
            }
            ShellCommandReadiness::Initializing
            | ShellCommandReadiness::PromptRendering
            | ShellCommandReadiness::ClearingInput { .. } => Err(TerminalControlError::Busy),
            ShellCommandReadiness::Unknown => Err(TerminalControlError::ReadinessUnknown),
            ShellCommandReadiness::Disconnected => Err(TerminalControlError::Disconnected),
        }
    }

    pub(crate) fn start(&mut self, id: u64, request: TerminalExecRequest) -> Vec<ExecEffect> {
        if self.active.is_some() {
            return fail(id, TerminalExecError::Busy);
        }
        if matches!(self.readiness, ShellCommandReadiness::Ready { .. }) {
            return if self.input_dirty {
                self.start_clear(id, request)
            } else {
                self.start_submit(id, request)
            };
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
        ) && matches!(self.readiness, ShellCommandReadiness::Ready { .. })
        {
            if data.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
                self.input_dirty = false;
                self.command_epoch = self.command_epoch.saturating_add(1);
                self.readiness = ShellCommandReadiness::SubmissionPending {
                    command_epoch: self.command_epoch,
                };
            } else if !data.is_empty() {
                // Conservatively remember any unsubmitted input. Backspace or
                // shell editing may make the line empty again, but an extra
                // Ctrl+C is safer than appending an Agent command to a line
                // whose exact editing state is unknown.
                self.input_dirty = true;
            }
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
        let effects: Vec<_> = events.iter().flat_map(|event| self.on_osc(event)).collect();
        if let Some(active) = self
            .active
            .as_ref()
            .filter(|active| active.phase == ExecPhase::Observing && !active.detached)
        {
            publish_progress(active, TerminalExecCompletion::ObservedOutput, None, false);
        }
        effects
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
                active.observer = None;
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
        let active = self.active.as_mut().expect("timed out exec exists");
        if active.detached || active.timed_out {
            return Vec::new();
        }
        active.timed_out = true;
        publish_progress(active, TerminalExecCompletion::TimedOut, None, false);
        let output = output_with_completion(active, TerminalExecCompletion::TimedOut, None);
        vec![ExecEffect::Complete { id, output }]
    }

    pub(crate) fn disconnect(&mut self) -> Vec<ExecEffect> {
        self.readiness = ShellCommandReadiness::Disconnected;
        let Some(active) = self.active.take() else {
            return Vec::new();
        };
        if active.detached {
            Vec::new()
        } else if active.timed_out {
            publish_progress(&active, TerminalExecCompletion::TimedOut, None, true);
            Vec::new()
        } else {
            fail(active.id, TerminalExecError::Disconnected)
        }
    }
}

fn fail(id: u64, error: TerminalExecError) -> Vec<ExecEffect> {
    vec![ExecEffect::Fail { id, error }]
}

fn publish_progress(
    active: &ActiveExec,
    completion: TerminalExecCompletion,
    exit_code: Option<i32>,
    is_final: bool,
) {
    let Some(observer) = active.observer.clone() else {
        return;
    };
    let output = output_with_completion(active, completion, exit_code);
    observer.publish(TerminalExecProgress {
        output: output.output,
        completion,
        exit_code,
        duration_ms: output.duration_ms,
        is_final,
    });
}

#[cfg(test)]
mod tests;
