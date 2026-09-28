use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::SessionNotification;
use agent_runtime::{RuntimeEvent, SessionId, TurnId};
use tokio::sync::broadcast;

use crate::acp::state::AcpSessionState;
use crate::acp::translate::{AcpEventTranslator, session_update_to_events_for_agent};
use crate::acp::turn::AcpTurnTracker;

use super::runner::ConnectShared;

pub(super) struct NotificationContext {
    events: broadcast::Sender<RuntimeEvent>,
    session_id: SessionId,
    state: Arc<Mutex<AcpSessionState>>,
    active_turn: Arc<Mutex<Option<AcpTurnTracker>>>,
    history_replay: Arc<Mutex<Option<TurnId>>>,
    translator: Arc<Mutex<AcpEventTranslator>>,
    agent_name: String,
}

impl NotificationContext {
    pub(super) fn new(shared: &ConnectShared) -> Self {
        Self {
            events: shared.events_tx.clone(),
            session_id: shared.session_id.clone(),
            state: shared.state.clone(),
            active_turn: shared.active_turn.clone(),
            history_replay: shared.history_replay.clone(),
            translator: Arc::new(Mutex::new(AcpEventTranslator)),
            agent_name: shared.config.name.to_string(),
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
    let active = context.active_turn.lock().ok().and_then(|mut active| {
        let tracker = active.as_mut()?;
        tracker.observe(&notification.update);
        Some(tracker.turn_id().clone())
    });
    let replay_window = context
        .history_replay
        .lock()
        .ok()
        .and_then(|replay| replay.clone());
    let Some((turn_id, replay)) = notification_turn(active, replay_window) else {
        return Ok(());
    };
    let events = context.translator.lock().map_or_else(
        |_| {
            session_update_to_events_for_agent(
                &notification.update,
                &context.session_id,
                &turn_id,
                &context.agent_name,
                replay,
            )
        },
        |mut translator| {
            translator.session_update_to_events(
                &notification.update,
                &context.session_id,
                &turn_id,
                &context.agent_name,
                replay,
            )
        },
    );
    for event in events {
        let _ = context.events.send(event);
    }
    Ok(())
}

/// 这条通知该归到哪个轮次，以及它的翻译要不要按「历史回放」走。
///
/// 三级：有活动轮次就归它（直播）；没有活动轮次但回放窗口开着，就归回放轮次；两者都没有
/// 就返回 `None` —— 无主的通知一律丢弃，绝不算进任何会话。
///
/// 抽成纯函数是因为这条回落就是「点开 ACP 历史会话什么都不显示」的根因所在：
/// `session/load` 重放历史时没有 prompt，`active_turn` 必然是空的，此前的实现直接
/// `return Ok(())` 把整段回放扔掉了。
fn notification_turn(
    active: Option<TurnId>,
    replay_window: Option<TurnId>,
) -> Option<(TurnId, bool)> {
    match active {
        Some(turn_id) => Some((turn_id, false)),
        None => replay_window.map(|turn_id| (turn_id, true)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(id: &str) -> TurnId {
        TurnId::from_string(id)
    }

    #[test]
    fn an_active_turn_wins_over_the_replay_window() {
        assert_eq!(
            notification_turn(Some(turn("live")), Some(turn("replay"))),
            Some((turn("live"), false)),
            "有活动轮次时就是直播，回放窗口不该改写归属"
        );
    }

    #[test]
    fn a_replay_window_takes_over_while_history_is_being_loaded() {
        assert_eq!(
            notification_turn(None, Some(turn("replay"))),
            Some((turn("replay"), true)),
            "`session/load` 期间没有轮次，历史必须落到回放轮次上"
        );
    }

    #[test]
    fn unowned_notifications_are_dropped() {
        assert_eq!(
            notification_turn(None, None),
            None,
            "既没有轮次也没有回放窗口：这条通知不该被算进任何会话"
        );
    }
}
