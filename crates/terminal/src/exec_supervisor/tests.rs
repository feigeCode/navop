use super::{
    ExecEffect, ExecPhase, ExecSupervisor, ShellCommandReadiness, TerminalExecError,
    TerminalInputSource,
};
use crate::osc::OscEvent;
use crate::{TerminalExecCompletion, TerminalExecRequest};
use std::time::Duration;

mod ready_wait;

fn request(command: &str) -> TerminalExecRequest {
    TerminalExecRequest {
        command: command.to_string(),
        submit: true,
        wait_for_output: true,
        ready_timeout: Duration::ZERO,
        timeout: Duration::from_secs(30),
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
            source: TerminalInputSource::AgentPreflight,
            data,
        }, ExecEffect::ArmTimeout {
            id: timeout_id,
            phase: ExecPhase::ClearingInput,
            ..
        }] if data == &[0x03] && *timeout_id == id
    ));
    assert!(matches!(
        supervisor.on_osc(&OscEvent::InputStart).as_slice(),
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
fn ready_exec_clears_then_submits_after_fresh_input_start() {
    let mut supervisor = ready_supervisor();
    submit(&mut supervisor, 11, "df -h");
    assert_eq!(
        ShellCommandReadiness::SubmissionPending { command_epoch: 11 },
        supervisor.readiness()
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
fn concurrent_user_input_aborts_clear_without_submitting() {
    let mut supervisor = ready_supervisor();
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
fn command_finished_before_command_start_does_not_complete_new_operation() {
    let mut supervisor = ready_supervisor();
    supervisor.start(23, request("pwd"));

    assert!(
        supervisor
            .on_osc(&OscEvent::CommandFinished { exit_code: 0 })
            .is_empty()
    );
    assert!(matches!(
        supervisor.on_osc(&OscEvent::InputStart).first(),
        Some(ExecEffect::Write {
            source: TerminalInputSource::AgentCommand,
            ..
        })
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
