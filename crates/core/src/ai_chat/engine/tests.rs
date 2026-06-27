use super::ChatEngine;
use crate::ai_chat::types::AiChatMode;
use crate::storage::StorageManager;
use crate::storage::connection::SqliteConnection;
use agent_runtime::{Plan, PlanSource, PlanStep, RuntimeEvent, SessionId, StepStatus, TurnId};
use tokio_util::sync::CancellationToken;

#[test]
fn chat_engine_defaults_to_ask_mode() {
    let engine = test_engine();

    assert_eq!(AiChatMode::Ask, engine.mode());
}

#[test]
fn chat_engine_switching_mode_cancels_in_flight_work_without_clearing_messages() {
    let mut engine = test_engine();
    let cancel_token = CancellationToken::new();
    engine.push_user_message("keep this");
    engine.is_loading = true;
    engine.cancel_token = Some(cancel_token.clone());

    engine.set_mode(AiChatMode::Plan);

    assert_eq!(AiChatMode::Plan, engine.mode());
    assert!(cancel_token.is_cancelled());
    assert!(!engine.is_loading);
    assert!(engine.cancel_token.is_none());
    assert_eq!(1, engine.messages.len());
    assert_eq!("keep this", engine.messages[0].content);
}

#[test]
fn chat_engine_runtime_plan_updates_upsert_plan_message() {
    let mut engine = test_engine();
    let turn_id = TurnId::from_string("turn_1");
    let mut inspect = PlanStep::new("Inspect", "Read current state");
    inspect.status = StepStatus::Running;
    let plan = Plan::new("Ship plan mode", PlanSource::Llm).with_steps(vec![inspect]);

    engine.apply_runtime_event(runtime_event_plan(turn_id.clone(), plan));
    assert_eq!(1, engine.messages.len());
    assert!(engine.messages[0].content.contains("### Plan"));
    assert!(engine.messages[0].content.contains("- [~] Inspect"));

    let mut complete = PlanStep::new("Inspect", "Read current state");
    complete.status = StepStatus::Completed;
    let plan = Plan::new("Ship plan mode", PlanSource::Llm).with_steps(vec![complete]);
    engine.apply_runtime_event(runtime_event_plan(turn_id, plan));

    assert_eq!(1, engine.messages.len());
    assert!(engine.messages[0].content.contains("- [x] Inspect"));
}

#[test]
fn chat_engine_runtime_assistant_events_stream_and_finalize_single_message() {
    let mut engine = test_engine();
    let session_id = SessionId::from_string("session_1");
    let turn_id = TurnId::from_string("turn_1");

    engine.apply_runtime_event(RuntimeEvent::AssistantMessageDelta {
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        delta: "hel".to_string(),
    });
    engine.apply_runtime_event(RuntimeEvent::AssistantMessageDelta {
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        delta: "lo".to_string(),
    });
    engine.apply_runtime_event(RuntimeEvent::AssistantMessage {
        session_id,
        turn_id,
        text: "hello".to_string(),
    });

    assert_eq!(1, engine.messages.len());
    assert_eq!("hello", engine.messages[0].content);
    assert!(!engine.messages[0].is_streaming);
}

#[test]
fn chat_engine_runtime_need_user_input_finishes_loading() {
    let mut engine = test_engine();
    engine.is_loading = true;
    engine.cancel_token = Some(CancellationToken::new());

    engine.apply_runtime_event(RuntimeEvent::NeedUserInput {
        session_id: SessionId::from_string("session_1"),
        turn_id: TurnId::from_string("turn_1"),
        question: "Which database?".to_string(),
    });

    assert!(!engine.is_loading);
    assert!(engine.cancel_token.is_none());
    assert_eq!(
        "Input required: Which database?",
        engine.messages[0].content
    );
}

fn test_engine() -> ChatEngine {
    ChatEngine::new(StorageManager::new_with_connection(test_connection()))
}

fn test_connection() -> SqliteConnection {
    let path = std::env::temp_dir().join(format!(
        "onetcli_ai_chat_engine_{}.db",
        uuid::Uuid::new_v4()
    ));
    SqliteConnection::open_with_pool_size(path, 1).expect("test sqlite should open")
}

fn runtime_event_plan(turn_id: TurnId, plan: Plan) -> RuntimeEvent {
    RuntimeEvent::PlanUpdated {
        session_id: SessionId::from_string("session_1"),
        turn_id,
        plan,
    }
}
