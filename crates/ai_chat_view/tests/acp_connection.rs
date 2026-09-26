use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1::{ContentBlock, SessionId, TextContent};
use agent_runtime::RuntimeEvent;
use ai_chat_view::{
    AcpAgentConfig, AcpConnectOutcome, AcpConnection, AcpConnectionPhase, AcpPermissionFuture,
    AcpPermissionOutcome, AcpPermissionProvider, AcpPromptStartError, AcpSessionContinuity,
    AcpTimeoutConfig,
};

#[derive(Clone, Copy)]
enum Mode {
    Text,
    Empty,
    AuthRequired,
    PromptError,
    PromptHang,
    Permission,
    ExitAfterInitialize,
}

/// 重连必须复用上次的 ACP 会话，而不是一路 `session/new`。
///
/// 两处证据互相印证：agent 侧记下的方法序列里有 `session/load`，且会话 id 没变——
/// fake agent 的 `session/new` 按进程给不同 id，走 new 一定看得出来。
#[tokio::test]
async fn reconnect_reuses_the_remembered_acp_session() {
    let log = temp_method_log("reuse");
    let first = connect_reuse(&log, None).await;
    let remembered = first.protocol_session_id();
    assert!(
        remembered.starts_with("fake-new-"),
        "first connect should create a session, got {remembered}"
    );
    assert_eq!(
        Some(AcpSessionContinuity::StartedFresh),
        first.session_continuity(),
        "first connect has nothing to reuse"
    );
    drop(first);

    let second = connect_reuse(&log, Some(&remembered)).await;

    assert_eq!(
        remembered,
        second.protocol_session_id(),
        "reconnect must land on the remembered ACP session"
    );
    assert_eq!(
        vec!["initialize", "session/new", "initialize", "session/load"],
        read_method_log(&log),
        "reconnect must ask the agent to load the remembered session"
    );
    assert_eq!(
        Some(AcpSessionContinuity::ReusedWithHistory),
        second.session_continuity(),
        "reuse must be reported as such, or the view would warn about lost context"
    );
}

/// 记住的会话已经不存在时，要退回新建，而不是把整次连接判死。
#[tokio::test]
async fn a_vanished_remembered_session_falls_back_to_a_new_one() {
    let log = temp_method_log("fallback");

    let connection = connect_reuse(&log, Some("fake-missing-session")).await;

    assert!(
        connection.protocol_session_id().starts_with("fake-new-"),
        "a failed load must fall back to a freshly created session, got {}",
        connection.protocol_session_id()
    );
    assert_eq!(
        vec!["initialize", "session/load", "session/new"],
        read_method_log(&log),
        "the fallback is a real new session, not a silent stop"
    );
    assert_eq!(
        Some(AcpSessionContinuity::RestartedAfterReuseFailure),
        connection.session_continuity(),
        "a failed reopen must be reported as a lost context, not as a fresh start"
    );
}

/// 只声明 `session/resume` 的 agent：接上了会话，但 agent 不回放历史。
///
/// 这一档必须与「干净的新会话」区分开——否则屏幕上空空如也时，
/// 用户没法判断是上下文断了还是只是没显示。
#[tokio::test]
async fn an_agent_that_only_resumes_is_reported_as_no_history_replay() {
    let log = temp_method_log("resume-only");
    let executable =
        std::env::var("CARGO_BIN_EXE_fake_acp_agent").expect("fake ACP executable path");
    let config = AcpAgentConfig::new("fake", "Fake ACP", executable)
        .with_args(vec![
            "session-resume-only".to_string(),
            log.display().to_string(),
        ])
        .with_timeouts(short_timeouts(Duration::from_secs(2)));

    let connection = match AcpConnection::connect_with_runtime_and_resume(
        &config,
        std::env::current_dir().expect("current directory"),
        tokio::runtime::Handle::current(),
        Some(SessionId::new("fake-remembered")),
    )
    .await
    .expect("resume-only agent should connect")
    {
        AcpConnectOutcome::Ready(connection) => *connection,
        AcpConnectOutcome::AuthenticationRequired(_) => panic!("unexpected authentication"),
    };

    assert_eq!("fake-remembered", connection.protocol_session_id());
    assert_eq!(
        vec!["initialize", "session/resume"],
        read_method_log(&log),
        "without load capability the only way back is session/resume"
    );
    assert_eq!(
        Some(AcpSessionContinuity::ReusedWithoutHistory),
        connection.session_continuity(),
        "context is back but nothing will be replayed on screen"
    );
}

/// agent 不声明 load/resume 能力时不该硬试：直接新建。
#[tokio::test]
async fn an_agent_without_reopen_capability_creates_a_new_session() {
    let log = temp_method_log("no-capability");
    let executable =
        std::env::var("CARGO_BIN_EXE_fake_acp_agent").expect("fake ACP executable path");
    let config = AcpAgentConfig::new("fake", "Fake ACP", executable)
        .with_args(vec!["text".to_string(), log.display().to_string()])
        .with_timeouts(short_timeouts(Duration::from_secs(2)));

    let connection = match AcpConnection::connect_with_runtime_and_resume(
        &config,
        std::env::current_dir().expect("current directory"),
        tokio::runtime::Handle::current(),
        Some(SessionId::new("fake-remembered")),
    )
    .await
    .expect("connect should succeed without reopen capability")
    {
        AcpConnectOutcome::Ready(connection) => *connection,
        AcpConnectOutcome::AuthenticationRequired(_) => panic!("unexpected authentication"),
    };

    assert_eq!("fake-session", connection.protocol_session_id());
    assert_eq!(vec!["initialize", "session/new"], read_method_log(&log));
    assert_eq!(
        Some(AcpSessionContinuity::StartedFresh),
        connection.session_continuity(),
        "an agent without reopen capability was never going to continue the context"
    );
}

async fn connect_reuse(log: &std::path::Path, resume: Option<&str>) -> AcpConnection {
    let executable =
        std::env::var("CARGO_BIN_EXE_fake_acp_agent").expect("fake ACP executable path");
    let config = AcpAgentConfig::new("fake", "Fake ACP", executable)
        .with_args(vec!["session-reuse".to_string(), log.display().to_string()])
        .with_timeouts(short_timeouts(Duration::from_secs(2)));

    match AcpConnection::connect_with_runtime_and_resume(
        &config,
        std::env::current_dir().expect("current directory"),
        tokio::runtime::Handle::current(),
        resume.map(|id| SessionId::new(id.to_string())),
    )
    .await
    .expect("fake session-reuse agent should connect")
    {
        AcpConnectOutcome::Ready(connection) => *connection,
        AcpConnectOutcome::AuthenticationRequired(_) => panic!("unexpected authentication"),
    }
}

fn short_timeouts(prompt: Duration) -> AcpTimeoutConfig {
    AcpTimeoutConfig {
        connect: Duration::from_secs(2),
        authenticate: Duration::from_secs(2),
        prompt,
    }
}

fn temp_method_log(tag: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "fake-acp-methods-{tag}-{}-{unique}.log",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn read_method_log(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn text_response_emits_output_and_completes() {
    let connection = ready_connection(Mode::Text, Duration::from_secs(2)).await;
    let events = prompt_until_terminal(&connection, "hello").await;

    let text = events
        .iter()
        .filter_map(|event| match event {
            RuntimeEvent::AssistantMessageDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!("fake response", text, "unexpected events: {events:?}");
    assert!(matches!(
        events.last(),
        Some(RuntimeEvent::TurnCompleted { .. })
    ));
}

#[tokio::test]
async fn empty_agent_response_becomes_turn_failure() {
    let connection = ready_connection(Mode::Empty, Duration::from_secs(2)).await;
    let events = prompt_until_terminal(&connection, "hello").await;

    assert!(matches!(
        events.last(),
        Some(RuntimeEvent::TurnFailed { reason, .. }) if reason.contains("returned no content")
    ));
}

#[tokio::test]
async fn interactive_authentication_can_complete_connection() {
    let outcome = connect_fake(Mode::AuthRequired, Duration::from_secs(2))
        .await
        .unwrap();
    let AcpConnectOutcome::AuthenticationRequired(pending) = outcome else {
        panic!("fake auth agent should require explicit authentication");
    };
    assert_eq!(vec!["fake-login"], pending.methods());

    let connection = (*pending)
        .authenticate("fake-login".to_string())
        .await
        .expect("fake authentication should succeed");
    let events = prompt_until_terminal(&connection, "hello").await;

    assert!(matches!(
        events.last(),
        Some(RuntimeEvent::TurnCompleted { .. })
    ));
}

#[tokio::test]
async fn nested_provider_401_is_preserved() {
    let connection = ready_connection(Mode::PromptError, Duration::from_secs(2)).await;
    let events = prompt_until_terminal(&connection, "hello").await;

    assert!(matches!(
        events.last(),
        Some(RuntimeEvent::TurnFailed { reason, .. })
            if reason.contains("HTTP 401") && reason.contains("Invalid API key")
    ));
}

#[tokio::test]
async fn prompt_timeout_sends_cancel_and_returns_to_ready() {
    let connection = ready_connection(Mode::PromptHang, Duration::from_millis(100)).await;
    let events = prompt_until_terminal(&connection, "hello").await;

    assert!(matches!(
        events.last(),
        Some(RuntimeEvent::TurnFailed { reason, .. }) if reason.contains("timed out")
    ));
    // The phase returns to Ready on a separate task after the failed turn
    // settles; poll with a deadline so the assertion does not race that async
    // transition (the timeout->cancel handshake can outlive the emitted event).
    let ready_deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if connection.phase() == AcpConnectionPhase::Ready {
            break;
        }
        if tokio::time::Instant::now() >= ready_deadline {
            panic!(
                "connection did not return to Ready after prompt timeout, phase={:?}",
                connection.phase()
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_second_prompt_is_rejected_without_emitting_a_fake_terminal_event() {
    let connection = ready_connection(Mode::PromptHang, Duration::from_secs(2)).await;
    let mut receiver = connection.subscribe();
    let first_turn = connection
        .try_prompt(text_prompt("first"))
        .expect("first prompt should start");

    for _ in 0..2 {
        receiver
            .recv()
            .await
            .expect("turn start events should be emitted");
    }

    assert_eq!(
        Err(AcpPromptStartError::AlreadyRunning),
        connection.try_prompt(text_prompt("second"))
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), receiver.recv())
            .await
            .is_err(),
        "rejected prompt must not emit a synthetic TurnFailed event"
    );
    assert_eq!(
        AcpConnectionPhase::RunningTurn {
            turn_id: first_turn.clone(),
        },
        connection.phase()
    );

    connection.cancel();
    loop {
        let event = receiver
            .recv()
            .await
            .expect("cancelled prompt should finish normally");
        if let RuntimeEvent::TurnCancelled { turn_id, .. } = event {
            assert_eq!(first_turn, turn_id);
            break;
        }
    }
    tokio::task::yield_now().await;
    assert_eq!(AcpConnectionPhase::Ready, connection.phase());
}

#[tokio::test]
async fn process_exit_after_initialize_fails_connect() {
    let error = match connect_fake(Mode::ExitAfterInitialize, Duration::from_secs(10)).await {
        Ok(_) => panic!("exited fake agent must not produce a ready connection"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("not ready")
            || error.to_string().contains("Failed to connect ACP Agent")
            || error.to_string().contains("connection timed out"),
        "unexpected error: {error:#}"
    );
}

#[tokio::test]
async fn permission_request_reaches_connection_provider_and_returns_original_option() {
    let (request_tx, mut request_rx) = tokio::sync::mpsc::unbounded_channel();
    let provider: AcpPermissionProvider = Arc::new(move |request| {
        request_tx.send(request).expect("permission observer");
        Box::pin(async {
            AcpPermissionOutcome::Selected {
                option_id: "allow-once".to_string(),
            }
        }) as AcpPermissionFuture
    });
    let connection = match AcpConnection::connect_with_runtime_and_permission_provider(
        &fake_config(Mode::Permission, Duration::from_secs(2)),
        std::env::current_dir().expect("current directory"),
        tokio::runtime::Handle::current(),
        provider,
    )
    .await
    .expect("fake permission agent should connect")
    {
        AcpConnectOutcome::Ready(connection) => *connection,
        AcpConnectOutcome::AuthenticationRequired(_) => panic!("unexpected authentication"),
    };

    let events = prompt_until_terminal(&connection, "write file").await;
    let request = request_rx.recv().await.expect("ACP permission request");

    assert_eq!("fake-session", request.session_id);
    assert_eq!("fake-call", request.tool_call_id);
    assert_eq!("Write file", request.tool_name);
    assert_eq!(2, request.options.len());
    assert_eq!("allow-once", request.options[1].option_id);
    let text = events
        .iter()
        .filter_map(|event| match event {
            RuntimeEvent::AssistantMessageDelta { delta, .. } => Some(delta.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(text.contains("allow-once"), "unexpected events: {events:?}");
    assert!(matches!(
        events.last(),
        Some(RuntimeEvent::TurnCompleted { .. })
    ));
}

async fn ready_connection(mode: Mode, prompt_timeout: Duration) -> AcpConnection {
    match connect_fake(mode, prompt_timeout)
        .await
        .expect("fake agent should connect")
    {
        AcpConnectOutcome::Ready(connection) => *connection,
        AcpConnectOutcome::AuthenticationRequired(_) => panic!("unexpected authentication"),
    }
}

async fn connect_fake(mode: Mode, prompt_timeout: Duration) -> anyhow::Result<AcpConnectOutcome> {
    AcpConnection::connect_with_runtime(
        &fake_config(mode, prompt_timeout),
        std::env::current_dir().expect("current directory"),
        tokio::runtime::Handle::current(),
    )
    .await
}

fn fake_config(mode: Mode, prompt_timeout: Duration) -> AcpAgentConfig {
    let executable = std::env::var("CARGO_BIN_EXE_fake_acp_agent")
        .expect("Cargo should expose the fake ACP executable");
    let mut config = AcpAgentConfig::new("fake", "Fake ACP", executable)
        .with_args(vec![mode_name(mode).to_string()])
        .with_timeouts(AcpTimeoutConfig {
            connect: Duration::from_secs(2),
            authenticate: Duration::from_secs(2),
            prompt: prompt_timeout,
        });
    if matches!(mode, Mode::AuthRequired) {
        config.auth.requested_method = Some("fake-login".to_string());
    }
    config
}

async fn prompt_until_terminal(connection: &AcpConnection, prompt: &str) -> Vec<RuntimeEvent> {
    let mut receiver = connection.subscribe();
    connection
        .try_prompt(text_prompt(prompt))
        .expect("prompt should start");
    let mut events = Vec::new();
    loop {
        let event = receiver
            .recv()
            .await
            .expect("ACP event channel should stay open");
        let terminal = matches!(
            event,
            RuntimeEvent::TurnCompleted { .. }
                | RuntimeEvent::TurnCancelled { .. }
                | RuntimeEvent::TurnFailed { .. }
        );
        events.push(event);
        if terminal {
            return events;
        }
    }
}

fn text_prompt(prompt: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text(TextContent::new(prompt))]
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Text => "text",
        Mode::Empty => "empty",
        Mode::AuthRequired => "auth-required",
        Mode::PromptError => "prompt-error",
        Mode::PromptHang => "prompt-hang",
        Mode::Permission => "permission",
        Mode::ExitAfterInitialize => "exit-after-initialize",
    }
}
