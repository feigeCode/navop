use agent_client_protocol::schema::v1::{
    AgentCapabilities, AvailableCommand, Implementation, LoadSessionResponse, NewSessionResponse,
    ResumeSessionResponse, SessionConfigKind, SessionConfigOption, SessionConfigSelectOption,
    SessionConfigSelectOptions, SessionMode, SessionModeId, SessionUpdate,
};
use agent_runtime::TurnId;

use super::AcpError;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AcpConnectionPhase {
    #[default]
    Starting,
    Initializing,
    AuthenticationRequired {
        methods: Vec<String>,
    },
    Authenticating {
        method_id: String,
    },
    CreatingSession,
    Ready,
    RunningTurn {
        turn_id: TurnId,
    },
    Failed {
        error: AcpError,
    },
    Closed,
}

/// 一次连接里 ACP 会话**实际**是怎么打开的。
///
/// 与 [`super::AcpSessionOpen`] 分工不同：那个是由 agent 能力推出的「**可以**怎么开」，
/// 由 `acp_session_open_kind()` 从能力协商得出；这个记录「**实际**怎么开的」，是视图判断
/// 「要不要告诉用户上一轮上下文没接上」的唯一依据。
///
/// 复用失败只写日志是不够的：用户以为对话还在，继续追问才发现 agent 什么都不记得——
/// 这正是「会话记不住」的观感来源，必须让 UI 有机会说出来。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcpSessionContinuity {
    /// 接上了记住的会话，并让 agent 回放了历史（`session/load`）。
    ReusedWithHistory,
    /// 接上了记住的会话，但没有回放历史（`session/resume`）：agent 记得，屏幕上看不到。
    ReusedWithoutHistory,
    /// 没有可复用的记忆，或 agent 不具备复用能力：开了一条新会话。
    ///
    /// 这种情况不必提示——此前从没延续成功过，用户没有「上下文还在」的错误预期。
    StartedFresh,
    /// 有记忆但复用失败，退而新建：**上一轮的上下文已经断了**。
    RestartedAfterReuseFailure,
}

/// ACP 会话状态快照。用于保存协议层元数据,不直接承担渲染职责。
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AcpSessionState {
    phase: AcpConnectionPhase,
    agent_info: Option<Implementation>,
    agent_capabilities: AgentCapabilities,
    available_commands: Vec<AvailableCommand>,
    current_mode_id: Option<SessionModeId>,
    available_modes: Vec<SessionMode>,
    config_options: Vec<SessionConfigOption>,
    title: Option<String>,
    updated_at: Option<String>,
    usage: Option<AcpUsage>,
    /// 这次连接实际怎么打开会话的；连接还没走到开会话时为 `None`。
    session_continuity: Option<AcpSessionContinuity>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AcpUsage {
    pub used: u64,
    pub size: u64,
    pub cost: Option<agent_client_protocol::schema::v1::Cost>,
}

impl AcpSessionState {
    pub(crate) fn phase(&self) -> &AcpConnectionPhase {
        &self.phase
    }

    pub(crate) fn transition(&mut self, next: AcpConnectionPhase) -> Result<(), String> {
        if phase_transition_allowed(&self.phase, &next) {
            self.phase = next;
            Ok(())
        } else {
            Err(format!("{:?} -> {:?}", self.phase, next))
        }
    }

    pub(crate) fn agent_capabilities(&self) -> &AgentCapabilities {
        &self.agent_capabilities
    }

    pub(crate) fn agent_info(&self) -> Option<&Implementation> {
        self.agent_info.as_ref()
    }

    pub(crate) fn available_commands(&self) -> &[AvailableCommand] {
        &self.available_commands
    }

    pub(crate) fn current_mode_id(&self) -> Option<&SessionModeId> {
        self.current_mode_id.as_ref()
    }

    pub(crate) fn available_modes(&self) -> &[SessionMode] {
        &self.available_modes
    }

    pub(crate) fn config_options(&self) -> &[SessionConfigOption] {
        &self.config_options
    }

    pub(crate) fn current_model_config(&self) -> Option<&SessionConfigOption> {
        self.config_options.iter().find(|option| {
            matches!(
                option.category,
                Some(agent_client_protocol::schema::v1::SessionConfigOptionCategory::Model)
            ) || option.id.0.eq_ignore_ascii_case("model")
                || option.name.to_ascii_lowercase().contains("model")
        })
    }

    /// agent 把会话模式放在 `category=mode` 的配置项里（新式：opencode 等）。
    ///
    /// 传统走 `SessionModeState.available_modes`；这里只在没有 `modes` 时才用。
    /// 判定刻意排除 `model`：`"model".contains("mode")` 为真，不能用裸子串匹配。
    pub(crate) fn current_mode_config(&self) -> Option<&SessionConfigOption> {
        self.config_options.iter().find(|option| {
            matches!(
                option.category,
                Some(agent_client_protocol::schema::v1::SessionConfigOptionCategory::Mode)
            ) || option.id.0.eq_ignore_ascii_case("mode")
                || (option.name.to_ascii_lowercase().contains("mode")
                    && !option.name.to_ascii_lowercase().contains("model"))
        })
    }

    /// agent 广告的模型候选 `(value_id, label)`,按 agent 给出的顺序。
    ///
    /// 只读 `model` 类配置的 select 值;没有该配置或不是 select 时返回空,
    /// 不伪造候选。分组与平面列表都展开成同一顺序。
    pub(crate) fn model_options(&self) -> Vec<(String, String)> {
        let Some(option) = self.current_model_config() else {
            return Vec::new();
        };
        select_values(option)
            .into_iter()
            .map(|value| (value.value.to_string(), value.name.clone()))
            .collect()
    }

    /// agent 广告的会话模式候选 `(value_id, label, description)`,用于执行模式下拉。
    pub(crate) fn mode_options(&self) -> Vec<(String, String, Option<String>)> {
        let Some(option) = self.current_mode_config() else {
            return Vec::new();
        };
        select_values(option)
            .into_iter()
            .map(|value| {
                (
                    value.value.to_string(),
                    value.name.clone(),
                    value.description.clone(),
                )
            })
            .collect()
    }

    /// 当前会话模式的展示文案。
    ///
    /// 优先传统 `modes`；没有时取 `category=mode` 配置项的当前值。
    pub(crate) fn current_mode_label(&self) -> Option<String> {
        if let Some(current) = self.current_mode_id() {
            return Some(
                self.available_modes()
                    .iter()
                    .find(|mode| mode.id == *current)
                    .map(|mode| mode.name.clone())
                    .unwrap_or_else(|| current.0.to_string()),
            );
        }
        let option = self.current_mode_config()?;
        let SessionConfigKind::Select(select) = &option.kind else {
            return None;
        };
        let current = select.current_value.to_string();
        Some(
            select_values(option)
                .into_iter()
                .find(|value| value.value.to_string() == current)
                .map(|value| value.name.clone())
                .unwrap_or(current),
        )
    }

    pub(crate) fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub(crate) fn updated_at(&self) -> Option<&str> {
        self.updated_at.as_deref()
    }

    pub(crate) fn usage(&self) -> Option<&AcpUsage> {
        self.usage.as_ref()
    }

    /// 这次连接实际怎么打开会话的（复用 / 没记忆新建 / 复用失败降级）。
    pub(crate) fn session_continuity(&self) -> Option<AcpSessionContinuity> {
        self.session_continuity
    }

    pub(crate) fn set_session_continuity(&mut self, continuity: AcpSessionContinuity) {
        self.session_continuity = Some(continuity);
    }

    pub(crate) fn set_agent_capabilities(&mut self, capabilities: AgentCapabilities) {
        self.agent_capabilities = capabilities;
    }

    pub(crate) fn set_agent_info(&mut self, info: Option<Implementation>) {
        self.agent_info = info;
    }

    pub(crate) fn set_current_mode(&mut self, mode_id: SessionModeId) {
        self.current_mode_id = Some(mode_id);
    }

    pub(crate) fn replace_config_options(&mut self, config_options: Vec<SessionConfigOption>) {
        self.config_options = config_options;
    }

    pub(crate) fn apply_new_session_response(&mut self, response: &NewSessionResponse) {
        self.apply_modes_and_config(response.modes.as_ref(), response.config_options.as_ref());
    }

    pub(crate) fn apply_load_session_response(&mut self, response: &LoadSessionResponse) {
        self.apply_modes_and_config(response.modes.as_ref(), response.config_options.as_ref());
    }

    pub(crate) fn apply_resume_session_response(&mut self, response: &ResumeSessionResponse) {
        self.apply_modes_and_config(response.modes.as_ref(), response.config_options.as_ref());
    }

    pub(crate) fn apply_session_update(&mut self, update: &SessionUpdate) {
        match update {
            SessionUpdate::AvailableCommandsUpdate(update) => {
                self.available_commands = update.available_commands.clone();
            }
            SessionUpdate::CurrentModeUpdate(update) => {
                self.current_mode_id = Some(update.current_mode_id.clone());
            }
            SessionUpdate::ConfigOptionUpdate(update) => {
                self.config_options = update.config_options.clone();
            }
            SessionUpdate::SessionInfoUpdate(update) => {
                update.title.clone().update_to(&mut self.title);
                update.updated_at.clone().update_to(&mut self.updated_at);
            }
            SessionUpdate::UsageUpdate(update) => {
                self.usage = Some(AcpUsage {
                    used: update.used,
                    size: update.size,
                    cost: update.cost.clone(),
                });
            }
            _ => {}
        }
    }

    fn apply_modes_and_config(
        &mut self,
        modes: Option<&agent_client_protocol::schema::v1::SessionModeState>,
        config_options: Option<&Vec<SessionConfigOption>>,
    ) {
        if let Some(modes) = modes {
            self.current_mode_id = Some(modes.current_mode_id.clone());
            self.available_modes = modes.available_modes.clone();
        }
        if let Some(config_options) = config_options {
            self.config_options = config_options.clone();
        }
    }
}

/// select 型配置项的候选值，分组展开成同一顺序；非 select 返回空。
fn select_values(option: &SessionConfigOption) -> Vec<&SessionConfigSelectOption> {
    let SessionConfigKind::Select(select) = &option.kind else {
        return Vec::new();
    };
    match &select.options {
        SessionConfigSelectOptions::Ungrouped(values) => values.iter().collect(),
        SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| group.options.iter())
            .collect(),
        _ => Vec::new(),
    }
}

fn phase_transition_allowed(current: &AcpConnectionPhase, next: &AcpConnectionPhase) -> bool {
    use AcpConnectionPhase as Phase;

    if matches!(next, Phase::Closed) {
        return !matches!(current, Phase::Closed);
    }
    if matches!(next, Phase::Failed { .. }) {
        return !matches!(current, Phase::Failed { .. } | Phase::Closed);
    }

    matches!(
        (current, next),
        (Phase::Starting, Phase::Initializing)
            | (Phase::Initializing, Phase::Authenticating { .. })
            | (Phase::Initializing, Phase::AuthenticationRequired { .. })
            | (Phase::Initializing, Phase::CreatingSession)
            | (Phase::Authenticating { .. }, Phase::CreatingSession)
            | (
                Phase::Authenticating { .. },
                Phase::AuthenticationRequired { .. }
            )
            | (
                Phase::AuthenticationRequired { .. },
                Phase::Authenticating { .. }
            )
            | (Phase::CreatingSession, Phase::Ready)
            | (Phase::Ready, Phase::RunningTurn { .. })
            | (Phase::RunningTurn { .. }, Phase::Ready)
    )
}

#[cfg(test)]
mod tests {
    use agent_client_protocol::schema::v1::{
        AvailableCommand, AvailableCommandsUpdate, ConfigOptionUpdate, ContentBlock,
        CurrentModeUpdate, NewSessionResponse, SessionConfigOption, SessionConfigOptionCategory,
        SessionConfigSelectOption, SessionInfoUpdate, SessionMode, SessionModeState, SessionUpdate,
        TextContent, UsageUpdate,
    };

    use agent_runtime::TurnId;

    use super::{AcpConnectionPhase, AcpSessionContinuity, AcpSessionState};
    use crate::acp::{AcpError, AcpErrorKind};

    #[test]
    fn applies_initial_modes_and_config_options_from_new_session() {
        let mut state = AcpSessionState::default();
        let modes = SessionModeState::new(
            "ask",
            vec![
                SessionMode::new("ask", "Ask"),
                SessionMode::new("code", "Code"),
            ],
        );
        let config = SessionConfigOption::select(
            "model",
            "Model",
            "fast",
            vec![SessionConfigSelectOption::new("fast", "Fast")],
        );

        state.apply_new_session_response(
            &NewSessionResponse::new("s1")
                .modes(modes)
                .config_options(vec![config]),
        );

        assert_eq!(Some("ask"), state.current_mode_id().map(|id| id.0.as_ref()));
        assert_eq!(2, state.available_modes().len());
        assert_eq!(1, state.config_options().len());
    }

    #[test]
    fn mode_config_option_surfaces_as_execution_modes() {
        let mut state = AcpSessionState::default();
        let model_config = SessionConfigOption::select(
            "model",
            "Model",
            "fast",
            vec![SessionConfigSelectOption::new("fast", "Fast")],
        );
        let mode_config = SessionConfigOption::select(
            "mode",
            "Session Mode",
            "build",
            vec![
                SessionConfigSelectOption::new("build", "build").description("The default agent."),
                SessionConfigSelectOption::new("plan", "plan"),
                SessionConfigSelectOption::new("scout", "scout"),
            ],
        )
        .category(SessionConfigOptionCategory::Mode);

        state.apply_new_session_response(
            &NewSessionResponse::new("s1").config_options(vec![model_config, mode_config]),
        );

        // mode 配置不能被当成 model，反之亦然。
        assert_eq!(
            Some("mode"),
            state.current_mode_config().map(|option| option.id.0.as_ref())
        );
        assert_eq!(
            Some("model"),
            state.current_model_config().map(|option| option.id.0.as_ref())
        );
        assert_eq!(
            vec!["build", "plan", "scout"],
            state
                .mode_options()
                .into_iter()
                .map(|(value, _, _)| value)
                .collect::<Vec<_>>()
        );
        assert_eq!(Some("build".to_string()), state.current_mode_label());
    }

    #[test]
    fn session_updates_replace_commands_modes_config_and_usage() {
        let mut state = AcpSessionState::default();

        state.apply_session_update(&SessionUpdate::AvailableCommandsUpdate(
            AvailableCommandsUpdate::new(vec![AvailableCommand::new("plan", "Create plan")]),
        ));
        state.apply_session_update(&SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(
            "code",
        )));
        state.apply_session_update(&SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(
            Vec::new(),
        )));
        state.apply_session_update(&SessionUpdate::SessionInfoUpdate(
            SessionInfoUpdate::new()
                .title("ACP title")
                .updated_at("2026-06-28T00:00:00Z"),
        ));
        state.apply_session_update(&SessionUpdate::UsageUpdate(UsageUpdate::new(42, 100)));
        state.apply_session_update(&SessionUpdate::AgentMessageChunk(
            agent_client_protocol::schema::v1::ContentChunk::new(ContentBlock::Text(TextContent::new(
                "ignored by state",
            ))),
        ));

        assert_eq!(1, state.available_commands().len());
        assert_eq!(
            Some("code"),
            state.current_mode_id().map(|id| id.0.as_ref())
        );
        assert_eq!(Some("ACP title"), state.title());
        assert_eq!(Some("2026-06-28T00:00:00Z"), state.updated_at());
        assert_eq!(
            Some((42, 100)),
            state.usage().map(|usage| (usage.used, usage.size))
        );
    }

    #[test]
    fn ready_cannot_be_entered_before_session_creation() {
        let mut state = AcpSessionState::default();

        state.transition(AcpConnectionPhase::Initializing).unwrap();
        let error = state.transition(AcpConnectionPhase::Ready).unwrap_err();

        assert!(error.contains("Initializing -> Ready"));
        assert_eq!(AcpConnectionPhase::Initializing, state.phase);
    }

    #[test]
    fn every_non_closed_phase_can_transition_to_closed() {
        let phases = vec![
            AcpConnectionPhase::Starting,
            AcpConnectionPhase::Initializing,
            AcpConnectionPhase::AuthenticationRequired {
                methods: vec!["token".to_string()],
            },
            AcpConnectionPhase::Authenticating {
                method_id: "token".to_string(),
            },
            AcpConnectionPhase::CreatingSession,
            AcpConnectionPhase::Ready,
            AcpConnectionPhase::RunningTurn {
                turn_id: TurnId::from_string("turn"),
            },
            AcpConnectionPhase::Failed {
                error: test_error(),
            },
        ];

        for phase in phases {
            let mut state = AcpSessionState {
                phase: phase.clone(),
                ..AcpSessionState::default()
            };

            state
                .transition(AcpConnectionPhase::Closed)
                .unwrap_or_else(|error| panic!("{phase:?} should close: {error}"));
            assert_eq!(AcpConnectionPhase::Closed, state.phase);
        }
    }

    #[test]
    fn closed_is_terminal_and_cannot_transition_to_failed() {
        let mut state = AcpSessionState {
            phase: AcpConnectionPhase::Closed,
            ..AcpSessionState::default()
        };

        let error = state
            .transition(AcpConnectionPhase::Failed {
                error: test_error(),
            })
            .expect_err("closed state must not transition to failed");

        assert!(error.contains("Closed -> Failed"));
        assert_eq!(AcpConnectionPhase::Closed, state.phase);
    }

    #[test]
    fn session_continuity_is_absent_until_a_session_is_opened() {
        let mut state = AcpSessionState::default();

        assert_eq!(
            None,
            state.session_continuity(),
            "还没走到开会话时不能凭能力协商猜一个结论"
        );

        state.set_session_continuity(AcpSessionContinuity::RestartedAfterReuseFailure);

        assert_eq!(
            Some(AcpSessionContinuity::RestartedAfterReuseFailure),
            state.session_continuity()
        );
    }

    fn test_error() -> AcpError {
        AcpError::new(
            AcpErrorKind::ConnectionClosed,
            "agent",
            "Agent",
            "connection closed",
        )
    }
}
