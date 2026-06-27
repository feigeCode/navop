//! ACP 客户端连接:把一个外部 agent 作为 stdio 子进程拉起,驱动一轮对话。
//!
//! 借鉴(非复制)`agent-client-protocol` 自带的 `yolo_one_shot_client` 示例(Apache-2.0):
//! 用 `Client.builder().on_receive_notification(..).on_receive_request(..).connect_with(agent, main_fn)`。
//! 为支持交互式多轮:`main_fn` 内建会话后把 `ConnectionTo` 克隆回句柄并 park,
//! 句柄据此随时 `send_request(PromptRequest)` / `send_notification(CancelNotification)`。
//!
//! 对外只暴露 `subscribe() -> broadcast::Receiver<RuntimeEvent>`,与自研 `Runtime::subscribe()`
//! 同型,因此 view 的事件泵与 `AgentTranscript` 完全复用。

use agent_client_protocol::schema::{
    AuthMethod, AuthMethodId, AuthenticateRequest, CancelNotification, ContentBlock,
    InitializeRequest, NewSessionRequest, PromptRequest, ProtocolVersion, RequestPermissionRequest,
    SessionId as AcpSessionId, SessionNotification, TextContent,
};
use agent_client_protocol::{Agent, Client, ConnectionTo};
use agent_runtime::{RuntimeEvent, SessionId, TurnId};
use gpui::AsyncApp;
use one_core::gpui_tokio::Tokio;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{broadcast, oneshot, watch};

use crate::acp::config::AcpAgentConfig;
use crate::acp::permission::{acp_permission_provider, resolve_acp_permission_request};
use crate::acp::translate::session_update_to_events;

const AUTHENTICATE_TIMEOUT: Duration = Duration::from_secs(15);

/// 已建立的 ACP 会话连接(交互式)。
pub struct AcpConnection {
    handle: tokio::runtime::Handle,
    conn: ConnectionTo<Agent>,
    acp_session_id: AcpSessionId,
    /// 合成的 agent_runtime 会话 id,用于给翻译出的事件打标签 + view 事件泵过滤。
    session_id: SessionId,
    turn_id_tx: watch::Sender<TurnId>,
    events_tx: broadcast::Sender<RuntimeEvent>,
    join: tokio::task::JoinHandle<()>,
    /// drop 时让 `main_fn` 的 `shutdown_rx` 解析,从而优雅结束连接(关闭子进程)。
    _shutdown: oneshot::Sender<()>,
}

impl Drop for AcpConnection {
    fn drop(&mut self) {
        self.join.abort();
    }
}

impl AcpConnection {
    /// 拉起 agent 子进程、初始化协议、新建会话;就绪后返回句柄。
    pub async fn connect(config: &AcpAgentConfig, cx: &mut AsyncApp) -> anyhow::Result<Self> {
        let handle = cx.update(|cx| Tokio::handle(cx));
        let permission_provider = cx.update(|cx| acp_permission_provider(cx));
        let agent = config.to_acp_agent();

        let (events_tx, _keep) = broadcast::channel(512);
        let session_id = SessionId::from_string(format!("acp:{}", uuid::Uuid::new_v4()));
        let (turn_id_tx, turn_id_rx) = watch::channel(new_acp_turn_id());

        let (ready_tx, ready_rx) =
            oneshot::channel::<Result<(ConnectionTo<Agent>, AcpSessionId), String>>();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        // 连接任务持有的克隆。
        let config_for_auth = config.clone();
        let ev_notif = events_tx.clone();
        let ev_err = events_tx.clone();
        let sid_notif = session_id.clone();
        let sid_err = session_id.clone();
        let tid_err_rx = turn_id_rx.clone();
        let permission_provider_for_request = permission_provider.clone();

        let join = handle.spawn(async move {
            let turn_id_for_notification = turn_id_rx.clone();
            let result = Client
                .builder()
                .on_receive_notification(
                    async move |notification: SessionNotification, _cx| {
                        let tid_notif = turn_id_for_notification.borrow().clone();
                        for event in
                            session_update_to_events(&notification.update, &sid_notif, &tid_notif)
                        {
                            let _ = ev_notif.send(event);
                        }
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .on_receive_request(
                    async move |request: RequestPermissionRequest, responder, _connection| {
                        responder.respond(
                            resolve_acp_permission_request(
                                permission_provider_for_request.clone(),
                                request,
                            )
                            .await,
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_with(agent, async move |connection: ConnectionTo<Agent>| {
                    let setup = async {
                        let init = connection
                            .send_request(InitializeRequest::new(ProtocolVersion::V1))
                            .block_task()
                            .await?;
                        let advertised: Vec<String> = init
                            .auth_methods
                            .iter()
                            .map(|m| m.id().0.to_string())
                            .collect();
                        tracing::info!(
                            agent_info = ?init.agent_info,
                            auth_methods = ?advertised,
                            "ACP initialized"
                        );
                        if let Some(method_id) =
                            select_non_interactive_auth_method(&init.auth_methods, &config_for_auth)
                        {
                            authenticate(&connection, method_id).await;
                        } else if !init.auth_methods.is_empty() {
                            tracing::info!(
                                auth_methods = ?advertised,
                                "ACP 未找到可自动使用的环境变量鉴权方式,跳过 authenticate,让 agent 使用自身本地配置"
                            );
                        }
                        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
                        let resp = connection
                            .send_request(NewSessionRequest::new(cwd))
                            .block_task()
                            .await?;
                        tracing::info!(session = %resp.session_id.0, "ACP session created");
                        Ok::<_, agent_client_protocol::Error>(resp.session_id)
                    }
                    .await;

                    match setup {
                        Ok(acp_session_id) => {
                            let _ = ready_tx.send(Ok((connection.clone(), acp_session_id)));
                            // park:保持反应堆运行,直到句柄被 drop。
                            let _ = shutdown_rx.await;
                            Ok(())
                        }
                        Err(err) => {
                            let _ = ready_tx.send(Err(format!("{err}")));
                            Err(err)
                        }
                    }
                })
                .await;

            if let Err(err) = result {
                let _ = ev_err.send(RuntimeEvent::TurnFailed {
                    session_id: sid_err,
                    turn_id: tid_err_rx.borrow().clone(),
                    reason: format!("ACP 连接结束:{err}"),
                });
            }
        });

        match ready_rx.await {
            Ok(Ok((conn, acp_session_id))) => Ok(Self {
                handle,
                conn,
                acp_session_id,
                session_id,
                turn_id_tx,
                events_tx,
                join,
                _shutdown: shutdown_tx,
            }),
            Ok(Err(err)) => {
                join.abort();
                anyhow::bail!("ACP agent 初始化失败:{err}")
            }
            Err(_) => {
                join.abort();
                anyhow::bail!("ACP agent 未就绪(进程可能已退出)")
            }
        }
    }

    /// 订阅事件流(与 `Runtime::subscribe()` 同型,供 view 事件泵复用)。
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events_tx.subscribe()
    }

    /// 合成会话 id(view 用来过滤事件 / 高亮)。
    pub fn session_id(&self) -> SessionId {
        self.session_id.clone()
    }

    /// 发起一轮:发送用户文本,流式更新经 `subscribe()` 推回。
    pub fn prompt(&self, text: String) {
        let turn_id = new_acp_turn_id();
        let _ = self.turn_id_tx.send(turn_id.clone());
        let _ = self.events_tx.send(RuntimeEvent::TurnStarted {
            session_id: self.session_id.clone(),
            turn_id: turn_id.clone(),
        });

        let conn = self.conn.clone();
        let acp_session_id = self.acp_session_id.clone();
        let events = self.events_tx.clone();
        let sid = self.session_id.clone();
        let tid = turn_id;

        self.handle.spawn(async move {
            let request = PromptRequest::new(
                acp_session_id,
                vec![ContentBlock::Text(TextContent::new(text))],
            );
            match conn.send_request(request).block_task().await {
                Ok(response) => {
                    tracing::info!(stop_reason = ?response.stop_reason, "ACP prompt completed");
                    let _ = events.send(RuntimeEvent::TurnCompleted {
                        session_id: sid,
                        turn_id: tid,
                        answer: None,
                    });
                }
                Err(err) => {
                    tracing::warn!("ACP prompt 失败:{err}");
                    let _ = events.send(RuntimeEvent::TurnFailed {
                        session_id: sid,
                        turn_id: tid,
                        reason: format!("{err}"),
                    });
                }
            }
        });
    }

    /// 中断当前轮(发送 ACP `session/cancel` 通知)。
    pub fn cancel(&self) {
        let conn = self.conn.clone();
        let acp_session_id = self.acp_session_id.clone();
        self.handle.spawn(async move {
            let _ = conn.send_notification(CancelNotification::new(acp_session_id));
        });
    }
}

fn new_acp_turn_id() -> TurnId {
    TurnId::from_string(format!("acp-turn:{}", uuid::Uuid::new_v4()))
}

async fn authenticate(connection: &ConnectionTo<Agent>, method_id: AuthMethodId) {
    tracing::info!(method = %method_id.0, "ACP authenticating");
    let request = connection
        .send_request(AuthenticateRequest::new(method_id))
        .block_task();
    match tokio::time::timeout(AUTHENTICATE_TIMEOUT, request).await {
        Ok(Ok(_)) => tracing::info!("ACP authenticate completed"),
        Ok(Err(err)) => tracing::warn!("ACP authenticate 失败(继续尝试新建会话):{err}"),
        Err(_) => tracing::warn!("ACP authenticate 超时(继续尝试新建会话)"),
    }
}

fn select_non_interactive_auth_method(
    methods: &[AuthMethod],
    config: &AcpAgentConfig,
) -> Option<AuthMethodId> {
    methods
        .iter()
        .find(|method| auth_env_available(method.id(), config))
        .map(|method| method.id().clone())
}

fn auth_env_available(method_id: &AuthMethodId, config: &AcpAgentConfig) -> bool {
    use crate::acp::config::AcpTransport;
    let method_key = method_id.0.to_ascii_lowercase();
    let config_env = match &config.transport {
        AcpTransport::Stdio { env, .. } => env.as_slice(),
        AcpTransport::Http { .. } => &[],
    };
    config_env
        .iter()
        .any(|(name, value)| !value.is_empty() && normalize_env_name(name) == method_key)
        || std::env::vars()
            .any(|(name, value)| !value.is_empty() && normalize_env_name(&name) == method_key)
}

fn normalize_env_name(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}
