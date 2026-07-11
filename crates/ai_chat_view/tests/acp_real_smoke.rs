use std::time::Duration;

use agent_runtime::RuntimeEvent;
use ai_chat_view::{AcpAgentConfig, AcpConnectOutcome, AcpConnection, AcpTimeoutConfig};

#[tokio::test]
#[ignore = "uses an installed local ACP agent and its existing authentication"]
async fn installed_acp_agent_returns_output_or_actionable_failure() {
    let command = std::env::var("ACP_SMOKE_COMMAND").expect("ACP_SMOKE_COMMAND is required");
    let name = std::env::var("ACP_SMOKE_NAME").unwrap_or_else(|_| "ACP Agent".to_string());
    let config =
        AcpAgentConfig::new("real-smoke", name.clone(), command).with_timeouts(AcpTimeoutConfig {
            connect: Duration::from_secs(60),
            authenticate: Duration::from_secs(120),
            prompt: Duration::from_secs(120),
        });

    let outcome = AcpConnection::connect_with_runtime(&config, tokio::runtime::Handle::current())
        .await
        .unwrap_or_else(|error| panic!("{name} connect failed: {error:#}"));
    let connection = match outcome {
        AcpConnectOutcome::Ready(connection) => connection,
        AcpConnectOutcome::AuthenticationRequired(pending) => {
            eprintln!("{name} requires authentication: {:?}", pending.methods());
            return;
        }
    };

    let events = prompt_until_terminal(&connection, "请只回复：ACP smoke ok").await;
    let text = assistant_text(&events);
    match events.last() {
        Some(RuntimeEvent::TurnCompleted { .. }) => {
            assert!(
                !text.trim().is_empty(),
                "{name} completed with blank output"
            );
            eprintln!("{name} response: {text}");
        }
        Some(RuntimeEvent::TurnFailed { reason, .. }) => {
            assert!(!reason.trim().is_empty(), "{name} failed without details");
            eprintln!("{name} actionable failure: {reason}");
        }
        Some(RuntimeEvent::TurnCancelled { .. }) => panic!("{name} cancelled unexpectedly"),
        other => panic!("{name} missing terminal event: {other:?}"),
    }
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

fn assistant_text(events: &[RuntimeEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            RuntimeEvent::AssistantMessageDelta { delta, .. } => Some(delta.as_str()),
            RuntimeEvent::AssistantMessage { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}
