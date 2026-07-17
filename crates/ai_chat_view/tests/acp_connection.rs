use std::sync::Arc;
use std::time::Duration;

use agent_runtime::RuntimeEvent;
use ai_chat_view::{
    AcpAgentConfig, AcpConnectOutcome, AcpConnection, AcpConnectionPhase, AcpPermissionFuture,
    AcpPermissionOutcome, AcpPermissionProvider, AcpTimeoutConfig,
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
        Some(RuntimeEvent::TurnFailed { reason, .. }) if reason.contains("没有返回任何内容")
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
        Some(RuntimeEvent::TurnFailed { reason, .. }) if reason.contains("超时")
    ));
    assert_eq!(AcpConnectionPhase::Ready, connection.phase());
}

#[tokio::test]
async fn process_exit_after_initialize_fails_connect() {
    let error = match connect_fake(Mode::ExitAfterInitialize, Duration::from_secs(10)).await {
        Ok(_) => panic!("exited fake agent must not produce a ready connection"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("未就绪") || error.to_string().contains("初始化失败"),
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
    connection.prompt(prompt.to_string());
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
