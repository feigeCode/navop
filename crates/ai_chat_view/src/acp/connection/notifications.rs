use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{SessionNotification, SessionUpdate};
use agent_runtime::{RuntimeEvent, SessionId, TurnId};
use tokio::sync::broadcast;

use crate::acp::detail_turn_id_for;
use crate::acp::state::AcpSessionState;
use crate::acp::translate::{AcpEventTranslator, session_update_to_events_for_agent};

use super::runner::ConnectShared;
use super::{AcpActiveTurns, AcpInteractiveSession};

pub(super) struct NotificationContext {
    events: broadcast::Sender<RuntimeEvent>,
    session_id: SessionId,
    state: Arc<Mutex<AcpSessionState>>,
    active_turn: AcpActiveTurns,
    /// 当前交互会话指针；见 [`super::AcpInteractiveSession`]。
    interactive_session: AcpInteractiveSession,
    history_replay: Arc<Mutex<Option<TurnId>>>,
    /// 已登记的子代理详情会话；见 [`super::AcpDetailSessions`]。
    detail_sessions: super::AcpDetailSessions,
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
            interactive_session: shared.interactive_session.clone(),
            history_replay: shared.history_replay.clone(),
            detail_sessions: shared.detail_sessions.clone(),
            translator: Arc::new(Mutex::new(AcpEventTranslator)),
            agent_name: shared.config.name.to_string(),
        }
    }

    /// 这条通知是不是某条已登记的子代理详情会话的？是就返回它的内置会话 id。
    fn detail_target_for(&self, acp_session_id: &str) -> Option<SessionId> {
        let sessions = self.detail_sessions.lock().ok()?;
        detail_target(&sessions, acp_session_id)
    }
}

pub(super) fn handle_notification(
    context: &NotificationContext,
    notification: SessionNotification,
) -> Result<(), agent_client_protocol::Error> {
    let acp_session_id = notification.session_id.0.to_string();

    // 子代理详情会话先行分流。
    //
    // 这条通知来自某条被 `session/load` 登记过的**子会话**（见
    // [`AcpConnection::load_detail_session`]）。它有两个不能忽略的性质：
    // 一是它不属于主会话，写进主会话的 `AcpSessionState` 会污染模式的可用性判定；
    // 二是它没有轮次（回放不产生 prompt），走主通路只会被 `notification_turn` 丢掉。
    // 所以这里整条走另一条通路：翻译成带**详情会话 id** 的事件，直接发出去。
    if let Some(detail_session_id) = context.detail_target_for(&acp_session_id) {
        let turn_id = detail_turn_id_for(&acp_session_id);
        let events = translate(
            context,
            &notification.update,
            &detail_session_id,
            &turn_id,
            // 子代理详情是**回放**：agent 按 message 整段重放历史，不是流式增量。
            true,
        );
        for event in events {
            let _ = context.events.send(event);
        }
        return Ok(());
    }

    // 按协议会话 id 路由：这条通知归属哪条会话，就喂那条会话**所有**在飞轮次的
    // tracker（steer 后同会话可能多轮并存，卡死判据每轮都要看）；事件归属取
    // 最新注册的一轮。后台会话的轮次照样被 observe。
    let active = context.active_turn.lock().ok().and_then(|mut active| {
        let mut newest: Option<(std::time::Instant, TurnId)> = None;
        for tracker in active
            .values_mut()
            .filter(|tracker| tracker.protocol_session_id() == acp_session_id)
        {
            tracker.observe(&notification.update);
            let registered = tracker.registered_at();
            if newest
                .as_ref()
                .is_none_or(|(current, _)| registered > *current)
            {
                newest = Some((registered, tracker.turn_id().clone()));
            }
        }
        newest.map(|(_, turn_id)| turn_id)
    });
    // 交互状态只认交互会话：后台会话的标题/模式/用量不能污染前台 UI。
    let is_interactive = context
        .interactive_session
        .lock()
        .is_ok_and(|interactive| *interactive == acp_session_id);
    if is_interactive {
        if let Ok(mut state) = context.state.lock() {
            state.apply_session_update(&notification.update);
        }
    }
    let replay_window = context
        .history_replay
        .lock()
        .ok()
        .and_then(|replay| replay.clone());
    let Some((turn_id, replay)) = notification_turn(active, replay_window) else {
        return Ok(());
    };
    let events = translate(context, &notification.update, &context.session_id, &turn_id, replay);
    for event in events {
        let _ = context.events.send(event);
    }
    Ok(())
}

/// 用连接的翻译器把一条 update 翻成事件。
///
/// 翻译器是无状态的；`map_or_else` 的两个分支只是「锁中毒时照常翻译」。抽出来是因为
/// 主会话与详情会话两条通路都要用它，复制一份迟早会漂移。
fn translate(
    context: &NotificationContext,
    update: &SessionUpdate,
    session_id: &SessionId,
    turn_id: &TurnId,
    replay: bool,
) -> Vec<RuntimeEvent> {
    context.translator.lock().map_or_else(
        |_| {
            session_update_to_events_for_agent(
                update,
                session_id,
                turn_id,
                &context.agent_name,
                replay,
            )
        },
        |mut translator| {
            translator.session_update_to_events(update, session_id, turn_id, &context.agent_name, replay)
        },
    )
}

/// 在注册表里查这条通知的目标详情会话（纯查表，便于单测）。
fn detail_target(
    detail_sessions: &HashMap<String, SessionId>,
    acp_session_id: &str,
) -> Option<SessionId> {
    detail_sessions.get(acp_session_id).cloned()
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

    fn detail_registry(pairs: &[(&str, &str)]) -> HashMap<String, SessionId> {
        pairs
            .iter()
            .map(|(acp, detail)| (acp.to_string(), SessionId::from_string(*detail)))
            .collect()
    }

    #[test]
    fn a_registered_child_session_routes_to_its_detail_session() {
        let sessions = detail_registry(&[("ses_child", "acp-sub:ses_child")]);
        assert_eq!(
            detail_target(&sessions, "ses_child"),
            Some(SessionId::from_string("acp-sub:ses_child")),
            "已登记的子会话必须被分流，否则回放会灌进主转录"
        );
    }

    #[test]
    fn an_unregistered_session_is_not_a_detail_session() {
        let sessions = detail_registry(&[("ses_child", "acp-sub:ses_child")]);
        // 主会话、以及从未 load 过的子会话，都不该被误判成详情会话。
        assert_eq!(detail_target(&sessions, "ses_main"), None);
        assert_eq!(detail_target(&sessions, "ses_other"), None);
        assert_eq!(detail_target(&HashMap::new(), "ses_child"), None);
    }

    #[test]
    fn the_detail_turn_id_is_stable_and_kept_out_of_real_turns() {
        // 同一条子会话的每次回放必须用同一个轮次 id，否则转录会把同一份历史
        // 当成两轮拼接。前缀也必须与 `acp-turn:` 分开。
        assert_eq!(
            detail_turn_id_for("ses_child"),
            detail_turn_id_for("ses_child")
        );
        assert_eq!(
            detail_turn_id_for("ses_child").to_string(),
            "acp-sub-turn:ses_child"
        );
        assert!(
            !detail_turn_id_for("ses_child")
                .to_string()
                .starts_with("acp-turn:")
        );
    }
}
