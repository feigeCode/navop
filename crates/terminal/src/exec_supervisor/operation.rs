use super::{
    ActiveExec, CLEAR_INPUT_TIMEOUT, ExecEffect, ExecPhase, ExecSupervisor, ShellCommandReadiness,
    TerminalInputSource,
};
use crate::exec_capture::sanitize_captured_terminal_output;
use crate::{TerminalExecCompletion, TerminalExecOutput, TerminalExecRequest};
use std::time::Instant;

impl ExecSupervisor {
    pub(super) fn start_submit(
        &mut self,
        id: u64,
        request: TerminalExecRequest,
    ) -> Vec<ExecEffect> {
        let observer = request.observer.clone();
        self.active = Some(ActiveExec {
            id,
            request,
            phase: ExecPhase::ClearingInput,
            started_at: Instant::now(),
            raw: Vec::new(),
            command_started: false,
            detached: false,
            timed_out: false,
            observer,
        });
        self.submit_active()
    }

    pub(super) fn start_ready_wait(
        &mut self,
        id: u64,
        request: TerminalExecRequest,
    ) -> Vec<ExecEffect> {
        let duration = request.ready_timeout;
        let observer = request.observer.clone();
        self.active = Some(ActiveExec {
            id,
            request,
            phase: ExecPhase::WaitingForReady,
            started_at: Instant::now(),
            raw: Vec::new(),
            command_started: false,
            detached: false,
            timed_out: false,
            observer,
        });
        vec![ExecEffect::ArmTimeout {
            id,
            phase: ExecPhase::WaitingForReady,
            duration,
        }]
    }

    pub(super) fn start_clear(&mut self, id: u64, request: TerminalExecRequest) -> Vec<ExecEffect> {
        let observer = request.observer.clone();
        self.active = Some(ActiveExec {
            id,
            request,
            phase: ExecPhase::ClearingInput,
            started_at: Instant::now(),
            raw: Vec::new(),
            command_started: false,
            detached: false,
            timed_out: false,
            observer,
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

    pub(super) fn on_prompt_start(&mut self) -> Vec<ExecEffect> {
        if !self.active_is_clearing() {
            self.readiness = ShellCommandReadiness::PromptRendering;
        }
        Vec::new()
    }

    pub(super) fn on_input_start(&mut self) -> Vec<ExecEffect> {
        let completed_prompt_render = self.readiness == ShellCommandReadiness::PromptRendering;
        self.prompt_epoch = self.prompt_epoch.saturating_add(1);
        self.input_dirty = false;
        self.readiness = ShellCommandReadiness::Ready {
            prompt_epoch: self.prompt_epoch,
        };
        if self.active_is_clearing() {
            return self.submit_active();
        }
        if self.active_is_waiting() {
            let active = self.active.take().expect("waiting exec exists");
            return self.start_submit(active.id, active.request);
        }
        if self.active_is_prompt_completable(completed_prompt_render) {
            let active = self.active.take().expect("started observer exists");
            if active.detached {
                return Vec::new();
            }
            let output =
                output_with_completion(&active, TerminalExecCompletion::ObservedOutput, None);
            super::publish_progress(&active, TerminalExecCompletion::ObservedOutput, None, true);
            return vec![ExecEffect::Complete {
                id: active.id,
                output,
            }];
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
        if active.request.submit {
            self.input_dirty = false;
            self.readiness = ShellCommandReadiness::SubmissionPending {
                command_epoch: active.id,
            };
        } else if !active.request.command.is_empty() {
            self.input_dirty = true;
        }
        if !active.request.submit || !active.request.wait_for_output {
            return vec![write, complete_submitted(&active)];
        }
        active.phase = ExecPhase::Observing;
        let id = active.id;
        let timeout = active.request.timeout;
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

    pub(super) fn on_command_start(&mut self) -> Vec<ExecEffect> {
        if let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.phase == ExecPhase::Observing)
        {
            active.command_started = true;
            self.readiness = ShellCommandReadiness::CommandRunning {
                command_epoch: active.id,
            };
        } else if let ShellCommandReadiness::SubmissionPending { command_epoch } = self.readiness {
            self.readiness = ShellCommandReadiness::CommandRunning { command_epoch };
        }
        Vec::new()
    }

    pub(super) fn on_command_finished(&mut self, exit_code: i32) -> Vec<ExecEffect> {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.phase == ExecPhase::Observing && !active.command_started)
        {
            // A late finish marker from the command that produced the current
            // prompt must not complete or rewind a newly submitted operation.
            return Vec::new();
        }
        let Some(active) = self
            .active
            .as_ref()
            .filter(|active| active.phase == ExecPhase::Observing && active.command_started)
        else {
            self.finish_unobserved_command();
            return Vec::new();
        };
        let id = active.id;
        self.readiness = ShellCommandReadiness::AwaitingPrompt { command_epoch: id };
        let active = self.active.take().expect("observing exec exists");
        if active.detached {
            return Vec::new();
        }
        let output = output_with_completion(
            &active,
            TerminalExecCompletion::ShellIntegrationExit,
            Some(exit_code),
        );
        super::publish_progress(
            &active,
            TerminalExecCompletion::ShellIntegrationExit,
            Some(exit_code),
            true,
        );
        vec![ExecEffect::Complete { id, output }]
    }

    pub(super) fn active_is_clearing(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.phase == ExecPhase::ClearingInput)
    }

    fn active_is_waiting(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.phase == ExecPhase::WaitingForReady)
    }

    fn active_is_prompt_completable(&self, completed_prompt_render: bool) -> bool {
        self.active.as_ref().is_some_and(|active| {
            active.phase == ExecPhase::Observing
                && (active.command_started || completed_prompt_render)
        })
    }

    fn finish_unobserved_command(&mut self) {
        if let ShellCommandReadiness::CommandRunning { command_epoch }
        | ShellCommandReadiness::SubmissionPending { command_epoch } = self.readiness
        {
            self.readiness = ShellCommandReadiness::AwaitingPrompt { command_epoch };
        }
    }
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

pub(super) fn output_with_completion(
    active: &ActiveExec,
    completion: TerminalExecCompletion,
    exit_code: Option<i32>,
) -> TerminalExecOutput {
    TerminalExecOutput {
        completion,
        exit_code,
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
