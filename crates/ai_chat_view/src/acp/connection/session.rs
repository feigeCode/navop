use std::path::PathBuf;

use agent_client_protocol::schema::{
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
    pub async fn create_session(&mut self, cwd: PathBuf) -> anyhow::Result<NewSessionResponse> {
        let response = self
            .conn
            .send_request(NewSessionRequest::new(cwd))
            .block_task()
            .await?;
        self.acp_session_id = response.session_id.clone();
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

    pub async fn load_session(
        &mut self,
        acp_session_id: AcpSessionId,
        cwd: PathBuf,
    ) -> anyhow::Result<LoadSessionResponse> {
        let response = self
            .conn
            .send_request(LoadSessionRequest::new(acp_session_id.clone(), cwd))
            .block_task()
            .await?;
        self.acp_session_id = acp_session_id;
        if let Ok(mut state) = self.state.lock() {
            state.apply_load_session_response(&response);
        }
        Ok(response)
    }

    pub async fn resume_session(
        &mut self,
        acp_session_id: AcpSessionId,
        cwd: PathBuf,
    ) -> anyhow::Result<ResumeSessionResponse> {
        let response = self
            .conn
            .send_request(ResumeSessionRequest::new(acp_session_id.clone(), cwd))
            .block_task()
            .await?;
        self.acp_session_id = acp_session_id;
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
