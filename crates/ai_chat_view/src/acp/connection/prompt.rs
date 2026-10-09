use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, PromptRequest, PromptResponse, TextContent,
};
use agent_runtime::{RuntimeEvent, TurnId};
use rust_i18n::t;
use tokio::sync::watch;

use crate::acp::error::extract_rpc_error_detail;
use crate::acp::state::AcpConnectionPhase;
use crate::acp::turn::{AcpTurnTracker, TurnOutcome, TurnProgress};
use crate::acp::{AcpError, AcpErrorKind, AcpRecoveryAction};

use super::{
    AcpActiveTurns, AcpConnection, PromptCompletionClaim, claim_prompt_completion,
    connection_closed_error, new_acp_turn_id,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcpPromptStartError {
    AlreadyRunning,
    NotReady,
    ImageUnsupported,
}

impl fmt::Display for AcpPromptStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => {
                write!(formatter, "{}", t!("AgentUi.acp_turn_already_running"))
            }
            Self::NotReady => write!(formatter, "{}", t!("AgentUi.acp_not_ready")),
            Self::ImageUnsupported => {
                write!(formatter, "{}", t!("AgentUi.acp_image_not_supported"))
            }
        }
    }
}

impl std::error::Error for AcpPromptStartError {}

impl AcpConnection {
    pub fn prompt(&self, text: String) -> Result<TurnId, AcpPromptStartError> {
        self.try_prompt(vec![ContentBlock::Text(TextContent::new(text))])
    }

    pub fn try_prompt(&self, prompt: Vec<ContentBlock>) -> Result<TurnId, AcpPromptStartError> {
        self.validate_prompt_capabilities(&prompt)?;
        let turn_id = new_acp_turn_id();
        let progress = self.register_turn(turn_id.clone())?;
        self.emit_turn_started(turn_id.clone());
        let request = PromptRequest::new(self.acp_session_id.clone(), prompt);
        let connection = self.conn.clone();
        let acp_session_id = self.acp_session_id.clone();
        let idle = self.prompt_timeout;
        let events = self.events_tx.clone();
        let session_id = self.session_id.clone();
        let active_turn = self.active_turn.clone();
        let state = self.state.clone();
        let agent_id = self.agent_id.clone();
        let agent_name = self.agent_name.clone();
        let expected_turn_id = turn_id.clone();
        self.handle.spawn(async move {
            let pending = connection.send_request(request).block_task();
            tokio::pin!(pending);
            // 「判卡死」与「cancel 握手」在这里分开做：wait_for_prompt 只管纯计时判据，
            // 回包 future 由本块持有。判卡死后**不能**直接 drop 它——drop 会拆掉
            // JSON-RPC 响应路由的接收端，agent 对 session/cancel 的回包就会以
            // connection-fatal 的「failed to send response, receiver dropped」浮出，
            // 把整条连接打成 Failed 而不是 Ready（合并自 dev 的 cd9d08877）。
            // 所以先发 cancel，再陪 future 走完握手：正常 agent 会回话收场；对那些
            // 永远不回话的 agent，用一个不小于 30s 的宽限期兜底，然后才按超时收场。
            let result = wait_for_prompt(pending.as_mut(), progress, idle).await;
            if matches!(result, PromptWait::Stalled) {
                let _ =
                    connection.send_notification(CancelNotification::new(acp_session_id));
                let cancel_grace = idle.max(Duration::from_secs(30));
                let _ = tokio::time::timeout(cancel_grace, pending.as_mut()).await;
            }
            finish_prompt(
                PromptContext {
                    events,
                    session_id,
                    active_turn,
                    state,
                    agent_id,
                    agent_name,
                    idle,
                    expected_turn_id,
                },
                result,
            )
            .await;
        });
        Ok(turn_id)
    }

    pub fn cancel(&self) {
        let connection = self.conn.clone();
        let session_id = self.acp_session_id.clone();
        self.handle.spawn(async move {
            let _ = connection.send_notification(CancelNotification::new(session_id));
        });
    }

    fn validate_prompt_capabilities(
        &self,
        prompt: &[ContentBlock],
    ) -> Result<(), AcpPromptStartError> {
        let includes_image = prompt
            .iter()
            .any(|block| matches!(block, ContentBlock::Image(_)));
        if !includes_image {
            return Ok(());
        }
        let state = self
            .state
            .lock()
            .map_err(|_| AcpPromptStartError::NotReady)?;
        if !state.agent_capabilities().prompt_capabilities.image {
            return Err(AcpPromptStartError::ImageUnsupported);
        }
        Ok(())
    }

    /// 占住这一轮，并把它的进展刻度接收端交出去。
    ///
    /// 接收端必须**在 tracker 装进 `active_turn` 之后**立刻取：刻度只增不减，但订阅到的
    /// 接收端初始值是 `default`，取晚了就会漏掉这中间到达的通知。
    ///
    /// steer：同会话已有在飞轮次时**不再拒绝**——直接再入一轮，两轮并存；
    /// 通知层把后续 `session/update` 同时喂给两个 tracker，事件归最新一轮。
    fn register_turn(
        &self,
        turn_id: TurnId,
    ) -> Result<watch::Receiver<TurnProgress>, AcpPromptStartError> {
        let protocol_session_id = self.acp_session_id.0.to_string();
        let mut active = self
            .active_turn
            .lock()
            .map_err(|_| AcpPromptStartError::NotReady)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| AcpPromptStartError::NotReady)?;
        match state.phase() {
            AcpConnectionPhase::Ready => state
                .transition(AcpConnectionPhase::RunningTurn {
                    turn_id: turn_id.clone(),
                })
                .map_err(|_| AcpPromptStartError::NotReady)?,
            // 已有轮次在飞（后台轮次或 steer 插话）：相位停在某一轮的
            // RunningTurn 上。状态机不允许 RunningTurn -> RunningTurn，也不需要
            // ——相位只描述「连接忙」，每轮的生死由 `active_turn` 表自己记。
            AcpConnectionPhase::RunningTurn { .. } => {}
            _ => return Err(AcpPromptStartError::NotReady),
        }
        let tracker = AcpTurnTracker::new(turn_id.clone(), protocol_session_id);
        let progress = tracker.progress();
        active.insert(turn_id.as_str().to_string(), tracker);
        Ok(progress)
    }

    fn emit_turn_started(&self, turn_id: TurnId) {
        let _ = self.events_tx.send(RuntimeEvent::TurnStarted {
            session_id: self.session_id.clone(),
            turn_id: turn_id.clone(),
        });
        let _ = self.events_tx.send(RuntimeEvent::Status {
            session_id: self.session_id.clone(),
            turn_id,
            title: responding_status_title(&self.agent_name),
            is_done: false,
        });
    }
}

fn responding_status_title(agent_name: &str) -> String {
    t!("AgentUi.acp_responding", name = agent_name).to_string()
}

struct PromptContext {
    events: tokio::sync::broadcast::Sender<RuntimeEvent>,
    session_id: agent_runtime::SessionId,
    active_turn: AcpActiveTurns,
    state: std::sync::Arc<std::sync::Mutex<crate::acp::AcpSessionState>>,
    agent_id: String,
    agent_name: String,
    /// 判超时用的空闲窗口；写进错误详情，好让日志里看得出等的是多久。
    idle: Duration,
    expected_turn_id: TurnId,
}

/// 一轮的等待是怎么结束的。
enum PromptWait {
    /// agent 回话了（成功，或它自己报的协议错误）。
    Settled(Result<PromptResponse, agent_client_protocol::Error>),
    /// 判定卡死：没有工具在跑，且静默满了一整个空闲窗口。
    Stalled,
}

/// 等这一轮的 `session/prompt` 响应，同时看守「卡死」。
///
/// 判据是**没有任何进展**，不是总时长——后者是本次修复前的行为，会把正常的长任务误杀：
/// 实测两次超时都恰好落在发起后第 600 秒，而 OpenCode 那边那一轮还在正常产出。规则三条：
///
/// 1. 有工具在执行 ⇒ **不设截止**。agent 显然在干活，工具自己的超时归它管；
/// 2. 收到这条会话的通知 ⇒ 截止延后一整个 `idle`（模型在流式输出）；
/// 3. 既没有工具在跑、又静默满 `idle` ⇒ 判卡死。cancel 通知与回包 future 的收尾
///    都由调用方做（见 [`AcpConnection::try_prompt`]），这里只保留纯计时判据，
///    好直接对三条规则写测试。
///
async fn wait_for_prompt<F>(
    mut pending: Pin<&mut F>,
    mut progress: watch::Receiver<TurnProgress>,
    idle: Duration,
) -> PromptWait
where
    F: Future<Output = Result<PromptResponse, agent_client_protocol::Error>>,
{
    // 发送端随 tracker 一起活。它要是先没了（这一轮被本地放弃、连接被收掉），就再也
    // 不会有人来续期——此时退化成一次普通的空闲等待，不能把截止永远关掉。
    let mut reachable = true;
    loop {
        let current = { *progress.borrow_and_update() };
        let armed = !reachable || current.inflight_tools == 0;
        let deadline = armed.then(|| tokio::time::Instant::now() + idle);
        tokio::select! {
            settled = pending.as_mut() => return PromptWait::Settled(settled),
            changed = progress.changed(), if reachable => {
                reachable = changed.is_ok();
            }
            () = sleep_until_opt(deadline) => return PromptWait::Stalled,
        }
    }
}

/// `Some` 时睡到那个时刻，`None` 时不设截止（永远挂起，把 `select!` 的这一支摘掉）。
async fn sleep_until_opt(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn finish_prompt(context: PromptContext, result: PromptWait) {
    let closed_error = connection_closed_error(&context.agent_id, &context.agent_name, None);
    let claim = claim_prompt_completion(
        &context.active_turn,
        &context.state,
        &context.expected_turn_id,
        &closed_error,
    );
    let Some(claim) = claim else {
        return;
    };
    let tracker = match claim {
        PromptCompletionClaim::Ready(tracker) => tracker,
        PromptCompletionClaim::Failed { turn_id, error } => {
            emit_failed(&context, turn_id, error);
            return;
        }
        // 轮次已不在（被放弃/已被收走），相位顺手收好了，不用发事件。
        PromptCompletionClaim::Reclaimed => return,
    };
    let turn_id = tracker.turn_id().clone();
    match result {
        PromptWait::Settled(Ok(response)) => {
            emit_success(&context, turn_id, tracker, response.stop_reason)
        }
        PromptWait::Settled(Err(error)) => emit_protocol_error(&context, turn_id, error),
        PromptWait::Stalled => emit_timeout(&context, turn_id),
    }
}

fn emit_success(
    context: &PromptContext,
    turn_id: TurnId,
    tracker: AcpTurnTracker,
    stop_reason: agent_client_protocol::schema::v1::StopReason,
) {
    match tracker.finish_success(stop_reason) {
        TurnOutcome::Completed => emit_completed(context, turn_id),
        TurnOutcome::Cancelled => emit_cancelled(context, turn_id),
        TurnOutcome::EmptyResponse => {
            let error = AcpError::empty_response(&context.agent_id, &context.agent_name);
            emit_failed(context, turn_id, error);
        }
    }
}

fn emit_completed(context: &PromptContext, turn_id: TurnId) {
    let _ = context.events.send(RuntimeEvent::TurnCompleted {
        session_id: context.session_id.clone(),
        turn_id,
        answer: None,
    });
}

fn emit_cancelled(context: &PromptContext, turn_id: TurnId) {
    let _ = context.events.send(RuntimeEvent::TurnCancelled {
        session_id: context.session_id.clone(),
        turn_id,
    });
}

fn emit_protocol_error(
    context: &PromptContext,
    turn_id: TurnId,
    protocol: agent_client_protocol::Error,
) {
    let detail = extract_rpc_error_detail(&protocol.message, protocol.data.as_ref());
    let error = AcpError::new(
        AcpErrorKind::PromptFailed,
        &context.agent_id,
        &context.agent_name,
        t!("AgentUi.acp_prompt_failed").to_string(),
    )
    .with_detail(detail)
    .with_recovery(AcpRecoveryAction::Retry);
    emit_failed(context, turn_id, error);
}

fn emit_timeout(context: &PromptContext, turn_id: TurnId) {
    // session/cancel 已在判卡死处发出（见 `try_prompt`），这里只负责把失败讲清楚。
    // 详情不能空着：只有「等了多久」能告诉用户这次到底是他网络的问题、还是 agent 真的死了。
    // 修复前这里是空串，日志里只剩 `kind=PromptTimeout detail=`，什么也判断不出来。
    let detail = t!(
        "AgentUi.acp_prompt_timeout_detail",
        seconds = context.idle.as_secs().to_string()
    )
    .to_string();
    let error = AcpError::new(
        AcpErrorKind::PromptTimeout,
        &context.agent_id,
        &context.agent_name,
        t!("AgentUi.acp_prompt_timeout").to_string(),
    )
    .with_detail(detail)
    .with_recovery(AcpRecoveryAction::Retry);
    emit_failed(context, turn_id, error);
}

fn emit_failed(context: &PromptContext, turn_id: TurnId, error: AcpError) {
    tracing::warn!(kind = ?error.kind, detail = %error.detail, "ACP prompt failed");
    let _ = context.events.send(RuntimeEvent::TurnFailed {
        session_id: context.session_id.clone(),
        turn_id,
        reason: error.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use agent_runtime::TurnId;

    use crate::acp::state::{AcpConnectionPhase, AcpSessionState};
    use crate::acp::turn::AcpTurnTracker;
    use crate::acp::{AcpError, AcpErrorKind};

    #[test]
    fn responding_status_uses_visible_agent_name() {
        assert_eq!(
            rust_i18n::t!("AgentUi.acp_responding", name = "Claude Code"),
            super::responding_status_title("Claude Code")
        );
    }

    #[test]
    fn active_turn_is_only_taken_when_the_expected_id_matches() {
        let current_turn = TurnId::from_string("current");
        let stale_turn = TurnId::from_string("stale");
        let active_turn = map_active_turn(AcpTurnTracker::new(
            current_turn.clone(),
            "ses_main".to_string(),
        ));

        let state = running_state(&current_turn);
        let closed_error = closed_error();
        assert!(
            super::claim_prompt_completion(&active_turn, &state, &stale_turn, &closed_error)
                .is_none()
        );
        assert_eq!(Some(current_turn.clone()), active_turn
            .lock()
            .expect("active turn lock")
            .get(current_turn.as_str())
            .map(|tracker| tracker.turn_id().clone()));

        let super::PromptCompletionClaim::Ready(tracker) =
            super::claim_prompt_completion(&active_turn, &state, &current_turn, &closed_error)
                .expect("matching active turn should be taken")
        else {
            panic!("running prompt should complete as ready");
        };
        assert_eq!(&current_turn, tracker.turn_id());
        assert!(active_turn.lock().expect("active turn lock").is_empty());
    }

    #[test]
    fn terminal_events_are_published_only_after_connection_is_ready() {
        let turn_id = TurnId::from_string("turn");
        let state = running_state(&turn_id);
        let active_turn = map_active_turn(AcpTurnTracker::new(
            turn_id.clone(),
            "ses_main".to_string(),
        ));

        let claim = super::claim_prompt_completion(&active_turn, &state, &turn_id, &closed_error());

        assert!(matches!(
            claim,
            Some(super::PromptCompletionClaim::Ready(_))
        ));
        assert_eq!(
            &AcpConnectionPhase::Ready,
            state.lock().expect("state lock").phase()
        );
    }

    #[test]
    fn background_turn_completes_while_another_session_still_runs() {
        // 后台轮次：相位是另一轮（另一条会话）的 RunningTurn，不是当前轮次的。
        // 这轮照样要从在飞表里收走，且相位保持 RunningTurn（那轮还活着）。
        let turn_id = TurnId::from_string("turn-bg");
        let foreground = TurnId::from_string("turn-fg");
        let state = running_state(&foreground);
        let active_turn = map_active_turn(AcpTurnTracker::new(
            turn_id.clone(),
            "ses_bg".to_string(),
        ));
        active_turn.lock().expect("active turn lock").insert(
            foreground.as_str().to_string(),
            AcpTurnTracker::new(foreground.clone(), "ses_fg".to_string()),
        );

        let claim = super::claim_prompt_completion(&active_turn, &state, &turn_id, &closed_error());

        assert!(matches!(claim, Some(super::PromptCompletionClaim::Reclaimed)));
        let active = active_turn.lock().expect("active turn lock");
        assert!(!active.contains_key(turn_id.as_str()), "后台轮次应被收走");
        assert!(
            active.contains_key(foreground.as_str()),
            "另一会话的在飞轮次不受影响"
        );
        drop(active);
        assert!(matches!(
            state.lock().expect("state lock").phase(),
            AcpConnectionPhase::RunningTurn { .. }
        ));
    }

    #[test]
    fn failed_connection_wins_prompt_completion_without_duplicate_success() {
        let turn_id = TurnId::from_string("turn");
        let state = running_state(&turn_id);
        let active_turn = map_active_turn(AcpTurnTracker::new(
            turn_id.clone(),
            "ses_main".to_string(),
        ));
        let error = AcpError::new(
            AcpErrorKind::ConnectionClosed,
            "agent",
            "Agent",
            "connection failed",
        );
        state
            .lock()
            .expect("state lock")
            .transition(AcpConnectionPhase::Failed {
                error: error.clone(),
            })
            .expect("connection failure");

        let claim = super::claim_prompt_completion(&active_turn, &state, &turn_id, &closed_error());

        assert!(matches!(
            claim,
            Some(super::PromptCompletionClaim::Failed {
                turn_id: claimed_turn,
                error: claimed_error,
            }) if claimed_turn == turn_id && claimed_error == error
        ));
        assert!(active_turn.lock().expect("active turn lock").is_empty());
        assert!(
            super::claim_prompt_completion(&active_turn, &state, &turn_id, &closed_error())
                .is_none(),
            "a terminal turn can only be claimed once"
        );
    }

    #[test]
    fn closed_connection_converts_late_prompt_success_into_failure() {
        let turn_id = TurnId::from_string("turn");
        let state = running_state(&turn_id);
        let active_turn = map_active_turn(AcpTurnTracker::new(
            turn_id.clone(),
            "ses_main".to_string(),
        ));
        state
            .lock()
            .expect("state lock")
            .transition(AcpConnectionPhase::Closed)
            .expect("connection close");
        let closed_error = closed_error();

        let claim = super::claim_prompt_completion(&active_turn, &state, &turn_id, &closed_error);

        assert!(matches!(
            claim,
            Some(super::PromptCompletionClaim::Failed {
                turn_id: claimed_turn,
                error,
            }) if claimed_turn == turn_id && error == closed_error
        ));
        assert!(active_turn.lock().expect("active turn lock").is_empty());
    }

    // ---- 超时判据 ----------------------------------------------------------
    //
    // 下面四条对应「卡死」判据的三条规则，以及它们各自最容易写错的地方。

    use std::time::Duration;

    use tokio::sync::watch;

    use crate::acp::turn::TurnProgress;

    type PendingResult = Result<super::PromptResponse, agent_client_protocol::Error>;

    /// 一个永远不回话的 `session/prompt`：把变量全压到「超时判据」上。
    fn never_settles() -> impl std::future::Future<Output = PendingResult> {
        std::future::pending()
    }

    fn progress_channel() -> (watch::Sender<TurnProgress>, watch::Receiver<TurnProgress>) {
        watch::channel(TurnProgress::default())
    }

    const IDLE: Duration = Duration::from_millis(80);

    #[tokio::test]
    async fn silence_with_nothing_running_is_a_stall() {
        let (_tx, rx) = progress_channel();
        let fut = never_settles();
        tokio::pin!(fut);

        let outcome =
            tokio::time::timeout(IDLE * 4, super::wait_for_prompt(fut.as_mut(), rx, IDLE))
                .await
                .expect("静默满一个窗口就该判卡死");

        assert!(matches!(outcome, super::PromptWait::Stalled));
    }

    #[tokio::test]
    async fn a_running_tool_suspends_the_deadline_entirely() {
        let (tx, rx) = progress_channel();
        tx.send_modify(|progress| progress.inflight_tools = 1);
        let fut = never_settles();
        tokio::pin!(fut);

        let wait = super::wait_for_prompt(fut.as_mut(), rx, IDLE);
        tokio::pin!(wait);

        // 远超一个空闲窗口的时间过去，这一轮也不该被判卡死：OpenCode 在整个 `task`
        // 执行期间一条通知都不发，本次子代理实测最长 182 分钟。
        assert!(
            tokio::time::timeout(IDLE * 6, &mut wait).await.is_err(),
            "有工具在跑时不能按静默计时"
        );

        // 工具收尾 ⇒ 计时重新开始，并且照常会判卡死。
        tx.send_modify(|progress| progress.inflight_tools = 0);
        let outcome = tokio::time::timeout(IDLE * 4, &mut wait)
            .await
            .expect("工具收尾后应当重新开始计时");
        assert!(matches!(outcome, super::PromptWait::Stalled));
    }

    #[tokio::test]
    async fn continuous_activity_keeps_renewing_the_deadline() {
        let (tx, rx) = progress_channel();
        // 每 1/4 个窗口推进一次刻度，连推 16 次 —— 总时长 4 个窗口。
        let feeder = tokio::spawn(async move {
            for _ in 0..16 {
                tokio::time::sleep(IDLE / 4).await;
                tx.send_modify(|progress| progress.revision += 1);
            }
        });
        let fut = never_settles();
        tokio::pin!(fut);

        let wait = super::wait_for_prompt(fut.as_mut(), rx, IDLE);
        tokio::pin!(wait);

        // 喂食还在进行时（已经过了两个窗口）绝不能判超时：这正是修复前会误杀的形状 ——
        // 一轮跑了十几分钟、每几秒就有一段输出，却因为「总时长到点」被砍掉。
        assert!(
            tokio::time::timeout(IDLE * 2, &mut wait).await.is_err(),
            "还在持续输出时不该判卡死"
        );

        // 喂食停了之后，照常在空闲窗口到期时判卡死 —— 续期不是「永不停表」。
        let outcome = tokio::time::timeout(IDLE * 8, &mut wait)
            .await
            .expect("停更之后应当在空闲窗口到期时判卡死");
        assert!(matches!(outcome, super::PromptWait::Stalled));
        feeder.await.expect("feeder task");
    }

    #[tokio::test]
    async fn losing_the_progress_sender_falls_back_to_a_plain_idle_wait() {
        let (tx, rx) = progress_channel();
        // 工具在跑 ⇒ 本来是不计时的。
        tx.send_modify(|progress| progress.inflight_tools = 1);
        // 但发送端随 tracker 一起没了（这一轮被本地放弃、连接被收掉）：不会再有进展来
        // 表示工具收尾，此时必须退化成普通空闲等待，否则这条任务会永远挂着。
        drop(tx);
        let fut = never_settles();
        tokio::pin!(fut);

        let outcome =
            tokio::time::timeout(IDLE * 4, super::wait_for_prompt(fut.as_mut(), rx, IDLE))
                .await
                .expect("发送端消失后不能永远不计时");

        assert!(matches!(outcome, super::PromptWait::Stalled));
    }

    #[tokio::test]
    async fn a_settled_prompt_wins_over_the_idle_timer() {
        let (_tx, rx) = progress_channel();
        let settled = async {
            Ok(super::PromptResponse::new(
                agent_client_protocol::schema::v1::StopReason::EndTurn,
            ))
        };
        tokio::pin!(settled);

        let outcome = super::wait_for_prompt(settled.as_mut(), rx, IDLE).await;

        assert!(matches!(outcome, super::PromptWait::Settled(Ok(_))));
    }

    fn running_state(turn_id: &TurnId) -> Arc<Mutex<AcpSessionState>> {
        let state = Arc::new(Mutex::new(AcpSessionState::default()));
        {
            let mut state = state.lock().expect("state lock");
            state
                .transition(AcpConnectionPhase::Initializing)
                .expect("initialize");
            state
                .transition(AcpConnectionPhase::CreatingSession)
                .expect("create session");
            state.transition(AcpConnectionPhase::Ready).expect("ready");
            state
                .transition(AcpConnectionPhase::RunningTurn {
                    turn_id: turn_id.clone(),
                })
                .expect("run turn");
        }
        state
    }

    fn map_active_turn(
        tracker: AcpTurnTracker,
    ) -> std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<String, AcpTurnTracker>>,
    > {
        let mut map = std::collections::HashMap::new();
        map.insert(tracker.turn_id().as_str().to_string(), tracker);
        std::sync::Arc::new(std::sync::Mutex::new(map))
    }

    fn closed_error() -> AcpError {
        AcpError::new(
            AcpErrorKind::ConnectionClosed,
            "agent",
            "Agent",
            "connection closed",
        )
    }
}
