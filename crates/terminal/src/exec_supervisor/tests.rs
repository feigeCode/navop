use super::{
    ExecEffect, ExecPhase, ExecSupervisor, ShellCommandReadiness, TerminalExecError,
    TerminalInputSource,
};
use crate::osc::OscEvent;
use crate::{
    TerminalControlError, TerminalControlReadiness, TerminalExecCompletion, TerminalExecObserver,
    TerminalExecRequest,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod ready_wait;

fn request(command: &str) -> TerminalExecRequest {
    TerminalExecRequest {
        command: command.to_string(),
        submit: true,
        wait_for_output: true,
        ready_timeout: Duration::ZERO,
        timeout: Duration::from_secs(30),
        observer: None,
    }
}

fn ready_supervisor() -> ExecSupervisor {
    let mut supervisor = ExecSupervisor::new();
    assert!(supervisor.on_osc(&OscEvent::InputStart).is_empty());
    supervisor
}

fn submit(supervisor: &mut ExecSupervisor, id: u64, command: &str) {
    assert!(matches!(
        supervisor.start(id, request(command)).as_slice(),
        [ExecEffect::Write {
            source: TerminalInputSource::AgentCommand,
            data,
        }, ExecEffect::ArmTimeout {
            id: timeout_id,
            phase: ExecPhase::Observing,
            ..
        }] if data == format!("{command}\n").as_bytes() && *timeout_id == id
    ));
}

#[test]
fn ready_exec_submits_without_ctrl_c_when_prompt_is_empty() {
    let mut supervisor = ready_supervisor();
    submit(&mut supervisor, 11, "df -h");
    assert_eq!(
        ShellCommandReadiness::SubmissionPending { command_epoch: 11 },
        supervisor.readiness()
    );
}

#[test]
fn submitted_only_exec_becomes_busy_before_command_start_arrives() {
    let mut supervisor = ready_supervisor();
    let mut submitted_only = request("sleep 1");
    submitted_only.wait_for_output = false;
    assert!(matches!(
        supervisor.start(34, submitted_only).as_slice(),
        [
            ExecEffect::Write {
                source: TerminalInputSource::AgentCommand,
                ..
            },
            ExecEffect::Complete { id: 34, .. }
        ]
    ));
    assert_eq!(
        ShellCommandReadiness::SubmissionPending { command_epoch: 34 },
        supervisor.readiness()
    );
    assert_eq!(
        vec![ExecEffect::Fail {
            id: 35,
            error: TerminalExecError::Busy,
        }],
        supervisor.start(35, request("pwd"))
    );
}

#[test]
fn running_terminal_rejects_without_writing() {
    let mut supervisor = ready_supervisor();
    assert!(
        supervisor
            .on_input(TerminalInputSource::User, b"sleep 300\n")
            .is_empty()
    );

    assert_eq!(
        vec![ExecEffect::Fail {
            id: 12,
            error: TerminalExecError::Busy,
        }],
        supervisor.start(12, request("pwd"))
    );
}

#[test]
fn init_command_submission_makes_terminal_busy() {
    let mut supervisor = ready_supervisor();
    assert!(
        supervisor
            .on_input(TerminalInputSource::InitCommand, b"cd /workspace\n")
            .is_empty()
    );

    assert_eq!(
        vec![ExecEffect::Fail {
            id: 30,
            error: TerminalExecError::Busy,
        }],
        supervisor.start(30, request("pwd"))
    );
}

#[test]
fn partial_user_input_is_cleared_before_agent_command() {
    let mut supervisor = ready_supervisor();
    assert!(
        supervisor
            .on_input(TerminalInputSource::User, b"git sta")
            .is_empty()
    );

    assert!(matches!(
        supervisor.start(13, request("pwd")).first(),
        Some(ExecEffect::Write {
            source: TerminalInputSource::AgentPreflight,
            data,
        }) if data == &[0x03]
    ));
}

#[test]
fn unsubmitted_agent_input_is_cleared_before_next_command() {
    let mut supervisor = ready_supervisor();
    let mut insert_only = request("git status");
    insert_only.submit = false;
    insert_only.wait_for_output = false;

    assert!(matches!(
        supervisor.start(130, insert_only).as_slice(),
        [ExecEffect::Write {
            source: TerminalInputSource::AgentCommand,
            data,
        }, ExecEffect::Complete { id: 130, .. }] if data == b"git status"
    ));
    assert!(matches!(
        supervisor.start(131, request("pwd")).first(),
        Some(ExecEffect::Write {
            source: TerminalInputSource::AgentPreflight,
            data,
        }) if data == &[0x03]
    ));
}

#[test]
fn concurrent_user_input_aborts_clear_without_submitting() {
    let mut supervisor = ready_supervisor();
    supervisor.on_input(TerminalInputSource::User, b"partial");
    supervisor.start(14, request("pwd"));

    assert_eq!(
        vec![ExecEffect::Fail {
            id: 14,
            error: TerminalExecError::ConcurrentUserInput,
        }],
        supervisor.on_input(TerminalInputSource::User, b"x")
    );
    assert!(supervisor.on_osc(&OscEvent::InputStart).is_empty());
}

#[test]
fn background_command_finishes_on_command_finished_without_eof() {
    let mut supervisor = ready_supervisor();
    submit(&mut supervisor, 21, "npm run dev &");
    assert!(supervisor.on_osc(&OscEvent::CommandStart).is_empty());

    let effects = supervisor.on_osc(&OscEvent::CommandFinished { exit_code: 0 });
    assert!(matches!(
        effects.as_slice(),
        [ExecEffect::Complete { id: 21, output }]
            if output.completion == TerminalExecCompletion::ShellIntegrationExit
                && output.exit_code == Some(0)
    ));
}

#[test]
fn cancel_after_submit_detaches_without_control_write() {
    let mut supervisor = ready_supervisor();
    submit(&mut supervisor, 22, "sleep 300");

    assert!(supervisor.cancel(22).is_empty());
    assert!(supervisor.on_osc(&OscEvent::CommandStart).is_empty());
    assert!(
        supervisor
            .on_osc(&OscEvent::CommandFinished { exit_code: 0 })
            .is_empty()
    );
    assert_eq!(
        ShellCommandReadiness::AwaitingPrompt { command_epoch: 22 },
        supervisor.readiness()
    );
}

#[test]
fn detached_observer_discards_late_output_instead_of_buffering_it() {
    let mut supervisor = ready_supervisor();
    submit(&mut supervisor, 36, "yes");
    supervisor.on_terminal_chunk(b"yes\r\nbefore-cancel\r\n", &[OscEvent::CommandStart]);
    let buffered_before_cancel = supervisor.active.as_ref().unwrap().raw.len();

    supervisor.cancel(36);
    supervisor.on_terminal_chunk(b"late-output\r\nlate-output\r\n", &[]);

    assert_eq!(
        buffered_before_cancel,
        supervisor.active.as_ref().unwrap().raw.len()
    );
}

#[test]
fn command_finished_before_command_start_does_not_complete_new_operation() {
    let mut supervisor = ready_supervisor();
    let effects = supervisor.start(23, request("pwd"));
    assert!(matches!(
        effects.as_slice(),
        [ExecEffect::Write { .. }, ExecEffect::ArmTimeout { .. }]
    ));

    assert!(
        supervisor
            .on_osc(&OscEvent::CommandFinished { exit_code: 0 })
            .is_empty()
    );
    assert_eq!(
        ShellCommandReadiness::SubmissionPending { command_epoch: 23 },
        supervisor.readiness()
    );
    supervisor.on_osc(&OscEvent::CommandStart);
    assert!(matches!(
        supervisor
            .on_osc(&OscEvent::CommandFinished { exit_code: 0 })
            .as_slice(),
        [ExecEffect::Complete { id: 23, .. }]
    ));
}

#[test]
fn captured_output_finishes_at_command_boundary() {
    let mut supervisor = ready_supervisor();
    submit(&mut supervisor, 24, "printf hello");
    assert!(
        supervisor
            .on_terminal_chunk(b"printf hello\r\nhello\r\n", &[OscEvent::CommandStart])
            .is_empty()
    );

    let effects = supervisor.on_terminal_chunk(
        b"\x1b]133;D;0\x07late-background-output\r\n",
        &[OscEvent::CommandFinished { exit_code: 0 }],
    );
    assert!(matches!(
        effects.as_slice(),
        [ExecEffect::Complete { output, .. }] if output.output == "hello"
    ));
}

#[test]
fn fresh_input_start_completes_when_finish_marker_is_missing() {
    let mut supervisor = ready_supervisor();
    submit(&mut supervisor, 25, "echo hello");
    supervisor.on_terminal_chunk(b"echo hello\r\nhello\r\n", &[OscEvent::CommandStart]);

    let effects = supervisor.on_osc(&OscEvent::InputStart);
    assert!(matches!(
        effects.as_slice(),
        [ExecEffect::Complete { output, .. }]
            if output.completion == TerminalExecCompletion::ObservedOutput
                && output.output == "hello"
    ));
}

#[test]
fn observing_timeout_returns_bounded_partial_output() {
    let mut supervisor = ready_supervisor();
    submit(&mut supervisor, 26, "long-command");
    supervisor.on_terminal_chunk(b"long-command\r\npartial\r\n", &[OscEvent::CommandStart]);

    let effects = supervisor.timeout(26, ExecPhase::Observing);
    assert!(matches!(
        effects.as_slice(),
        [ExecEffect::Complete { output, .. }]
            if output.completion == TerminalExecCompletion::TimedOut
                && output.output == "partial"
    ));
}

#[test]
fn observing_timeout_keeps_progress_and_rejects_replacement_command() {
    let progress = Arc::new(Mutex::new(Vec::new()));
    let sink = progress.clone();
    let mut tracked = request("long-command");
    tracked.observer = Some(TerminalExecObserver::new(move |update| {
        sink.lock().expect("progress lock").push(update);
    }));
    let mut supervisor = ready_supervisor();
    assert!(matches!(
        supervisor.start(260, tracked).as_slice(),
        [ExecEffect::Write { .. }, ExecEffect::ArmTimeout { .. }]
    ));
    supervisor.on_osc(&OscEvent::InputStart);
    supervisor.on_terminal_chunk(b"long-command\r\npartial\r\n", &[OscEvent::CommandStart]);

    assert!(matches!(
        supervisor.timeout(260, ExecPhase::Observing).as_slice(),
        [ExecEffect::Complete { output, .. }]
            if output.completion == TerminalExecCompletion::TimedOut
    ));
    assert_eq!(
        vec![ExecEffect::Fail {
            id: 261,
            error: TerminalExecError::Busy,
        }],
        supervisor.start(261, request("echo replacement"))
    );

    supervisor.on_terminal_chunk(b"done\r\n", &[]);
    supervisor.on_terminal_chunk(
        b"\x1b]133;D;0\x07",
        &[OscEvent::CommandFinished { exit_code: 0 }],
    );
    let progress = progress.lock().expect("progress lock");
    assert!(progress.iter().any(|update| {
        update.completion == TerminalExecCompletion::TimedOut && !update.is_final
    }));
    assert!(progress.iter().any(|update| {
        update.completion == TerminalExecCompletion::ShellIntegrationExit
            && update.is_final
            && update.output.contains("done")
    }));
}

#[test]
fn disconnect_fails_pre_submit_and_finishes_detached_observer() {
    let mut clearing = ready_supervisor();
    clearing.start(27, request("pwd"));
    assert_eq!(
        vec![ExecEffect::Fail {
            id: 27,
            error: TerminalExecError::Disconnected,
        }],
        clearing.disconnect()
    );

    let mut detached = ready_supervisor();
    submit(&mut detached, 28, "sleep 300");
    detached.cancel(28);
    assert!(detached.disconnect().is_empty());
    assert_eq!(ShellCommandReadiness::Disconnected, detached.readiness());
}

#[test]
fn cancel_before_submit_never_writes_agent_command() {
    let mut supervisor = ready_supervisor();
    supervisor.on_input(TerminalInputSource::User, b"partial");
    supervisor.start(29, request("pwd"));

    assert_eq!(
        vec![ExecEffect::Fail {
            id: 29,
            error: TerminalExecError::CancelledBeforeSubmit,
        }],
        supervisor.cancel(29)
    );
    assert!(supervisor.on_osc(&OscEvent::InputStart).is_empty());
}

#[test]
fn terminal_control_allows_only_active_foreground_states() {
    let mut supervisor = ExecSupervisor::new();

    supervisor.readiness = ShellCommandReadiness::SubmissionPending { command_epoch: 41 };
    assert_eq!(
        Ok(TerminalControlReadiness::SubmissionPending),
        supervisor.interrupt_foreground()
    );

    supervisor.readiness = ShellCommandReadiness::CommandRunning { command_epoch: 41 };
    assert_eq!(
        Ok(TerminalControlReadiness::CommandRunning),
        supervisor.interrupt_foreground()
    );
}

#[test]
fn terminal_control_rejects_non_running_states() {
    let cases = [
        (
            ShellCommandReadiness::Initializing,
            TerminalControlError::Busy,
        ),
        (
            ShellCommandReadiness::PromptRendering,
            TerminalControlError::Busy,
        ),
        (
            ShellCommandReadiness::ClearingInput { command_epoch: 7 },
            TerminalControlError::Busy,
        ),
        (
            ShellCommandReadiness::Ready { prompt_epoch: 7 },
            TerminalControlError::NotRunning,
        ),
        (
            ShellCommandReadiness::AwaitingPrompt { command_epoch: 7 },
            TerminalControlError::NotRunning,
        ),
        (
            ShellCommandReadiness::Unknown,
            TerminalControlError::ReadinessUnknown,
        ),
        (
            ShellCommandReadiness::Disconnected,
            TerminalControlError::Disconnected,
        ),
    ];

    for (readiness, expected) in cases {
        let mut supervisor = ExecSupervisor::new();
        supervisor.readiness = readiness;
        assert_eq!(Err(expected), supervisor.interrupt_foreground());
    }
}

#[test]
fn terminal_control_preserves_exec_observer_until_real_completion() {
    let mut supervisor = ready_supervisor();
    submit(&mut supervisor, 42, "sleep 300");
    supervisor.on_osc(&OscEvent::CommandStart);

    assert_eq!(
        Ok(TerminalControlReadiness::CommandRunning),
        supervisor.interrupt_foreground()
    );

    let effects = supervisor.on_osc(&OscEvent::CommandFinished { exit_code: 130 });
    assert!(matches!(
        effects.as_slice(),
        [ExecEffect::Complete { id: 42, output }]
            if output.exit_code == Some(130)
                && output.completion == TerminalExecCompletion::ShellIntegrationExit
    ));
}

#[test]
fn terminal_control_rejects_when_an_exec_is_waiting_for_ready() {
    let mut supervisor = ready_supervisor();
    supervisor.on_input(TerminalInputSource::User, b"sleep 300\n");
    supervisor.on_osc(&OscEvent::CommandStart);
    let mut waiting = request("echo queued");
    waiting.ready_timeout = Duration::from_secs(5);

    assert!(matches!(
        supervisor.start(43, waiting).as_slice(),
        [ExecEffect::ArmTimeout {
            id: 43,
            phase: ExecPhase::WaitingForReady,
            ..
        }]
    ));
    assert_eq!(
        Err(TerminalControlError::Busy),
        supervisor.interrupt_foreground()
    );
}
