use super::*;

#[test]
fn bounded_ready_wait_sends_no_bytes_until_fresh_input_start() {
    let mut supervisor = ready_supervisor();
    supervisor.on_input(TerminalInputSource::User, b"sleep 1\n");
    let mut waiting_request = request("pwd");
    waiting_request.ready_timeout = Duration::from_secs(5);

    assert_eq!(
        vec![ExecEffect::ArmTimeout {
            id: 31,
            phase: ExecPhase::WaitingForReady,
            duration: Duration::from_secs(5),
        }],
        supervisor.start(31, waiting_request)
    );
    supervisor.on_osc(&OscEvent::CommandStart);
    supervisor.on_osc(&OscEvent::CommandFinished { exit_code: 0 });
    assert!(matches!(
        supervisor.on_osc(&OscEvent::InputStart).as_slice(),
        [ExecEffect::Write {
            source: TerminalInputSource::AgentCommand,
            data,
        }, ExecEffect::ArmTimeout {
            phase: ExecPhase::Observing,
            ..
        }] if data == b"pwd\n"
    ));
}

#[test]
fn ready_wait_timeout_fails_without_terminal_write() {
    let mut supervisor = ready_supervisor();
    supervisor.on_input(TerminalInputSource::User, b"sleep 1\n");
    let mut waiting_request = request("pwd");
    waiting_request.ready_timeout = Duration::from_millis(5);
    supervisor.start(32, waiting_request);

    assert_eq!(
        vec![ExecEffect::Fail {
            id: 32,
            error: TerminalExecError::ReadyTimeout,
        }],
        supervisor.timeout(32, ExecPhase::WaitingForReady)
    );
}

#[test]
fn human_input_cancels_pending_ready_wait() {
    let mut supervisor = ready_supervisor();
    supervisor.on_input(TerminalInputSource::User, b"sleep 1\n");
    let mut waiting_request = request("pwd");
    waiting_request.ready_timeout = Duration::from_secs(5);
    supervisor.start(33, waiting_request);

    assert_eq!(
        vec![ExecEffect::Fail {
            id: 33,
            error: TerminalExecError::ConcurrentUserInput,
        }],
        supervisor.on_input(TerminalInputSource::User, b"x")
    );
}
