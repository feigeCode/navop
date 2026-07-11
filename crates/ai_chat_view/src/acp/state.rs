use agent_client_protocol::schema::{
    AgentCapabilities, AvailableCommand, LoadSessionResponse, NewSessionResponse,
    ResumeSessionResponse, SessionConfigOption, SessionMode, SessionModeId, SessionUpdate,
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

/// ACP 会话状态快照。用于保存协议层元数据,不直接承担渲染职责。
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AcpSessionState {
    phase: AcpConnectionPhase,
    agent_capabilities: AgentCapabilities,
    available_commands: Vec<AvailableCommand>,
    current_mode_id: Option<SessionModeId>,
    available_modes: Vec<SessionMode>,
    config_options: Vec<SessionConfigOption>,
    title: Option<String>,
    updated_at: Option<String>,
    usage: Option<AcpUsage>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AcpUsage {
    pub used: u64,
    pub size: u64,
    pub cost: Option<agent_client_protocol::schema::Cost>,
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

    pub(crate) fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub(crate) fn updated_at(&self) -> Option<&str> {
        self.updated_at.as_deref()
    }

    pub(crate) fn usage(&self) -> Option<&AcpUsage> {
        self.usage.as_ref()
    }

    pub(crate) fn set_agent_capabilities(&mut self, capabilities: AgentCapabilities) {
        self.agent_capabilities = capabilities;
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
        modes: Option<&agent_client_protocol::schema::SessionModeState>,
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

fn phase_transition_allowed(current: &AcpConnectionPhase, next: &AcpConnectionPhase) -> bool {
    use AcpConnectionPhase as Phase;
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
            | (Phase::AuthenticationRequired { .. }, Phase::Closed)
            | (Phase::CreatingSession, Phase::Ready)
            | (Phase::Ready, Phase::RunningTurn { .. })
            | (Phase::Ready, Phase::Closed)
            | (Phase::RunningTurn { .. }, Phase::Ready)
            | (Phase::RunningTurn { .. }, Phase::Closed)
            | (_, Phase::Failed { .. })
            | (Phase::Failed { .. }, Phase::Closed)
    )
}

#[cfg(test)]
mod tests {
    use agent_client_protocol::schema::{
        AvailableCommand, AvailableCommandsUpdate, ConfigOptionUpdate, ContentBlock,
        CurrentModeUpdate, NewSessionResponse, SessionConfigOption, SessionConfigSelectOption,
        SessionInfoUpdate, SessionMode, SessionModeState, SessionUpdate, TextContent, UsageUpdate,
    };

    use super::{AcpConnectionPhase, AcpSessionState};

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
            agent_client_protocol::schema::ContentChunk::new(ContentBlock::Text(TextContent::new(
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
}
