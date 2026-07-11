use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::SessionNotification;
use agent_runtime::{RuntimeEvent, SessionId};
use tokio::sync::broadcast;

use crate::acp::state::AcpSessionState;
use crate::acp::translate::{AcpEventTranslator, session_update_to_events};
use crate::acp::turn::AcpTurnTracker;

use super::runner::ConnectShared;

pub(super) struct NotificationContext {
    events: broadcast::Sender<RuntimeEvent>,
    session_id: SessionId,
    state: Arc<Mutex<AcpSessionState>>,
    active_turn: Arc<Mutex<Option<AcpTurnTracker>>>,
    translator: Arc<Mutex<AcpEventTranslator>>,
}

impl NotificationContext {
    pub(super) fn new(shared: &ConnectShared) -> Self {
        Self {
            events: shared.events_tx.clone(),
            session_id: shared.session_id.clone(),
            state: shared.state.clone(),
            active_turn: shared.active_turn.clone(),
            translator: Arc::new(Mutex::new(AcpEventTranslator)),
        }
    }
}

pub(super) fn handle_notification(
    context: &NotificationContext,
    notification: SessionNotification,
) -> Result<(), agent_client_protocol::Error> {
    if let Ok(mut state) = context.state.lock() {
        state.apply_session_update(&notification.update);
    }
    let turn_id = context.active_turn.lock().ok().and_then(|mut active| {
        let tracker = active.as_mut()?;
        tracker.observe(&notification.update);
        Some(tracker.turn_id().clone())
    });
    let Some(turn_id) = turn_id else {
        return Ok(());
    };
    let events = context.translator.lock().map_or_else(
        |_| session_update_to_events(&notification.update, &context.session_id, &turn_id),
        |mut translator| {
            translator.session_update_to_events(&notification.update, &context.session_id, &turn_id)
        },
    );
    for event in events {
        let _ = context.events.send(event);
    }
    Ok(())
}
