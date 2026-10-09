use std::path::PathBuf;

use agent_client_protocol::{Agent, ConnectionTo};
use agent_client_protocol::schema::v1::{
    CloseSessionRequest, CloseSessionResponse, DeleteSessionRequest, DeleteSessionResponse,
    ListSessionsRequest, ListSessionsResponse, LoadSessionRequest, LoadSessionResponse,
    LogoutRequest, LogoutResponse, NewSessionRequest, NewSessionResponse, ResumeSessionRequest,
    ResumeSessionResponse, SessionConfigId, SessionConfigValueId, SessionId as AcpSessionId,
    SessionInfo, SessionModeId, SetSessionConfigOptionRequest, SetSessionConfigOptionResponse,
    SetSessionModeRequest, SetSessionModeResponse,
};

use super::AcpConnection;

/// `session/list` 的分页上限。防止一个不回 cursor 的 agent 把这里变成死循环。
const SESSION_LIST_PAGE_LIMIT: usize = 10;

impl AcpConnection {
    /// 把交互会话指针挪到这条协议会话上。
    ///
    /// 连接自己的 `acp_session_id` 与通知层的共享指针必须同步更新：通知层拿不到
    /// `AcpConnection`，只认共享指针。
    fn point_interactive_at(&self, acp_session_id: &AcpSessionId) {
        if let Ok(mut interactive) = self.interactive_session.lock() {
            *interactive = acp_session_id.0.to_string();
        }
    }

    pub async fn create_session(&mut self, cwd: PathBuf) -> anyhow::Result<NewSessionResponse> {
        let response = self
            .conn
            .send_request(NewSessionRequest::new(cwd))
            .block_task()
            .await?;
        self.acp_session_id = response.session_id.clone();
        self.point_interactive_at(&response.session_id);
        if let Ok(mut state) = self.state.lock() {
            state.apply_new_session_response(&response);
        }
        Ok(response)
    }

    pub async fn list_sessions(
        &self,
        cwd: Option<PathBuf>,
        cursor: Option<String>,
    ) -> anyhow::Result<ListSessionsResponse> {
        let request = ListSessionsRequest::new().cwd(cwd).cursor(cursor);
        Ok(self.conn.send_request(request).block_task().await?)
    }

    /// 列出 `cwd` 下的历史会话,自动翻页。
    ///
    /// ponytail: 最多翻 [`SESSION_LIST_PAGE_LIMIT`] 页;真遇到页数不够用的 agent
    /// 再改成把 cursor 暴露给 UI 做「加载更多」。
    pub async fn list_all_sessions(
        &self,
        cwd: Option<PathBuf>,
    ) -> anyhow::Result<Vec<SessionInfo>> {
        let mut sessions = Vec::new();
        let mut cursor = None;
        for _ in 0..SESSION_LIST_PAGE_LIMIT {
            let response = self.list_sessions(cwd.clone(), cursor).await?;
            sessions.extend(response.sessions);
            match response.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Ok(sessions)
    }

    /// 打开一条历史会话。agent 会把整段历史重放成一批 `session/update`。
    ///
    /// 回放窗口的**开**在调用方（视图要拿同一个轮次 id 去放行事件，见
    /// [`AcpConnection::begin_history_replay`]），**关**在这里：协议保证响应派发于所有
    /// 回放通知之后，所以响应一回来就关不会漏掉尾巴，而且无论成功失败都能关掉。
    pub async fn load_session(
        &mut self,
        acp_session_id: AcpSessionId,
        cwd: PathBuf,
    ) -> anyhow::Result<LoadSessionResponse> {
        let response = self
            .conn
            .send_request(LoadSessionRequest::new(acp_session_id.clone(), cwd))
            .block_task()
            .await;
        self.end_history_replay();
        let response = response?;
        self.acp_session_id = acp_session_id;
        self.point_interactive_at(&self.acp_session_id);
        if let Ok(mut state) = self.state.lock() {
            state.apply_load_session_response(&response);
        }
        Ok(response)
    }

    /// 取一份「发起子代理详情 `session/load`」所需的句柄。
    ///
    /// 见 [`AcpDetailLoader`]：视图要在 `'static` 的异步块里 await 这次请求，
    /// 而 `AcpConnection` 归视图所有、借不进异步块。
    pub(crate) fn detail_loader(&self) -> AcpDetailLoader {
        AcpDetailLoader {
            conn: self.conn.clone(),
            detail_sessions: self.detail_sessions.clone(),
        }
    }

    /// 接上记住的会话（`session/resume`）。
    ///
    /// 协议里 `resume` 只接上下文、不回放历史，但窗口照样要围着它开一次：agent 实现
    /// 并不都守这条约定，真有 agent 顺手推几条 `session/update` 过来，那些通知同样
    /// 没有轮次可归；开窗的成本只是把这段窗口里的无主通知算成历史，而不开就是直接丢。
    pub async fn resume_session(
        &mut self,
        acp_session_id: AcpSessionId,
        cwd: PathBuf,
    ) -> anyhow::Result<ResumeSessionResponse> {
        let response = self
            .conn
            .send_request(ResumeSessionRequest::new(acp_session_id.clone(), cwd))
            .block_task()
            .await;
        self.end_history_replay();
        let response = response?;
        self.acp_session_id = acp_session_id;
        self.point_interactive_at(&self.acp_session_id);
        if let Ok(mut state) = self.state.lock() {
            state.apply_resume_session_response(&response);
        }
        Ok(response)
    }

    pub async fn close_session(&self) -> anyhow::Result<CloseSessionResponse> {
        Ok(self
            .conn
            .send_request(CloseSessionRequest::new(self.acp_session_id.clone()))
            .block_task()
            .await?)
    }

    pub async fn delete_session(
        &self,
        acp_session_id: AcpSessionId,
    ) -> anyhow::Result<DeleteSessionResponse> {
        Ok(self
            .conn
            .send_request(DeleteSessionRequest::new(acp_session_id))
            .block_task()
            .await?)
    }

    pub async fn set_mode(&self, mode_id: SessionModeId) -> anyhow::Result<SetSessionModeResponse> {
        let response = self
            .conn
            .send_request(SetSessionModeRequest::new(
                self.acp_session_id.clone(),
                mode_id.clone(),
            ))
            .block_task()
            .await?;
        if let Ok(mut state) = self.state.lock() {
            state.set_current_mode(mode_id);
        }
        Ok(response)
    }

    pub async fn set_config_option(
        &self,
        config_id: SessionConfigId,
        value: SessionConfigValueId,
    ) -> anyhow::Result<SetSessionConfigOptionResponse> {
        let response = self
            .conn
            .send_request(SetSessionConfigOptionRequest::new(
                self.acp_session_id.clone(),
                config_id,
                value,
            ))
            .block_task()
            .await?;
        if let Ok(mut state) = self.state.lock() {
            state.replace_config_options(response.config_options.clone());
        }
        Ok(response)
    }

    pub async fn logout(&self) -> anyhow::Result<LogoutResponse> {
        Ok(self
            .conn
            .send_request(LogoutRequest::new())
            .block_task()
            .await?)
    }
}

/// 发起「子代理详情」`session/load` 所需的最小句柄。
///
/// # 为什么不直接借 `&AcpConnection`
///
/// 视图要在 `cx.spawn` 的 `'static` 异步块里 await 这次请求，而 `AcpConnection`
/// 归视图所有、借不进异步块。照 [`AgentChatView::open_protocol_session`] 那样
/// `take()` 走也不行：那次请求发生在用户**主动切换会话**的时刻，连接让位是应当的；
/// 而点开一张子代理卡片时主会话往往还在跑，把连接抽走会让界面显示成断线。
///
/// 协议连接句柄本身可克隆，把最小集合拷出来最省事。
///
/// [`AgentChatView::open_protocol_session`]: crate::agent_view::AgentChatView::open_protocol_session
#[derive(Clone)]
pub(crate) struct AcpDetailLoader {
    conn: ConnectionTo<Agent>,
    detail_sessions: super::AcpDetailSessions,
}

impl AcpDetailLoader {
    /// 把一条子代理子会话登记成详情会话，并让它回放整段历史。
    ///
    /// 与 [`AcpConnection::load_session`] 的关键差别：**不动当前会话指针**。
    /// 用户点的是主对话里的一张卡片，主会话必须留在原地——把连接指针挪到子会话上，
    /// 之后的 prompt、取消、权限确认、模式切换全会打到错误的会话。
    ///
    /// 子会话的 `session/update` 靠 [`super::AcpDetailSessions`] 那张表分流，
    /// 因此这里也**不开主会话的历史回放窗口**：抢用同一个回放轮次会让子代理历史
    /// 被算进主转录。
    ///
    /// 登记必须发生在请求**之前**：回放是随请求一起推过来的，晚一步注册，
    /// 先到的几条通知就会因为「认不出是哪条会话」被丢掉。
    pub(crate) async fn load(
        &self,
        acp_session_id: AcpSessionId,
        cwd: PathBuf,
    ) -> anyhow::Result<()> {
        let protocol_id = acp_session_id.0.to_string();
        // 登记独立成块：`MutexGuard` 不能跨过下面的 `.await`，否则既触发
        // `clippy::await_holding_lock`，也把「锁被持有到请求返回」变成真事实。
        let detail_session_id = crate::acp::detail_session_uid_for(&protocol_id);
        {
            let mut registry = self
                .detail_sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("子代理详情会话注册表不可用"))?;
            registry.insert(protocol_id.clone(), detail_session_id);
        }

        let response = self
            .conn
            .send_request(LoadSessionRequest::new(acp_session_id, cwd))
            .block_task()
            .await;
        match response {
            Ok(_) => Ok(()),
            Err(error) => {
                // 没登记成功就不会有回放。留着这条映射，用户再点一次卡片只会得到
                // 一片空白（视图以为已经加载过了）。
                if let Ok(mut registry) = self.detail_sessions.lock() {
                    registry.remove(&protocol_id);
                }
                Err(error.into())
            }
        }
    }
}
