use super::*;
use tokio_util::sync::CancellationToken;

#[test]
fn cancelled_turn_cannot_write_or_clear_a_new_turn() {
    let (events, _rx) = tokio::sync::broadcast::channel(16);
    let session = Session::new(
        SessionId::from_string("session-cancel-test"),
        ResourceContext::new(),
        events,
    );
    let cancelled_id = TurnId::from_string("turn-cancelled");
    session.set_active_turn(ActiveTurn::new(
        cancelled_id.clone(),
        CancellationToken::new(),
        None,
    ));

    assert_eq!(
        Some(cancelled_id.clone()),
        session.cancel_and_detach_active_turn()
    );
    let current_id = TurnId::from_string("turn-current");
    session.set_active_turn(ActiveTurn::new(
        current_id.clone(),
        CancellationToken::new(),
        None,
    ));

    session.record_assistant_message(&cancelled_id, "late message");
    assert!(session.history_snapshot().items().is_empty());
    assert!(!session.clear_active_turn_if(&cancelled_id));
    assert_eq!(Some(current_id), session.active_turn_id());
}
