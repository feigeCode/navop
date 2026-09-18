//! 一次性 ACP agent 探测。
//!
//! 命令存在不等于「是一个可用的 ACP agent」：参数写错、协议不匹配、未登录都会在
//! `initialize` 或 `session/new` 阶段暴露。这里跑一次短生命周期会话，拿到 agent
//! 自报身份与它广告的模型候选，让聊天视图在真正连接前就能显示真实状态。
//!
//! Agent CLI 启动时可能索引自身 cwd，因此探测在独立临时目录运行，不使用 `$HOME`
//! （与会话目录发现同一理由）。

use std::path::PathBuf;
use std::time::Duration;

use super::config::AcpAgentConfig;
use super::state::AcpSessionState;
use super::{AcpConnectOutcome, AcpConnection};

/// 探测进程上限。与用户配置的交互式超时解耦，避免只是打开菜单就长时间挂起。
#[cfg_attr(test, allow(dead_code))]
const PROBE_TIMEOUT: Duration = Duration::from_secs(12);
/// 探测进程的临时工作目录名。
const PROBE_DIRECTORY: &str = "navop-acp-probe";

/// agent 广告的一个模型候选。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcpModelInfo {
    pub id: String,
    pub label: String,
}

/// 一次性探测的结论。`error` 为空即视为已识别。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AcpAgentProbe {
    pub name: Option<String>,
    pub version: Option<String>,
    pub auth_methods: Vec<String>,
    pub models: Vec<AcpModelInfo>,
    pub error: Option<String>,
}

impl AcpAgentProbe {
    /// 命令确实作为一个 ACP agent 应答过（含「需要登录」）。
    pub(crate) fn identified(&self) -> bool {
        self.error.is_none()
    }
}

/// 从会话状态提取 agent 身份与模型候选；纯函数，便于单测。
pub(crate) fn probe_from_state(state: &AcpSessionState) -> AcpAgentProbe {
    let info = state.agent_info();
    AcpAgentProbe {
        name: info.map(display_name),
        version: info
            .map(|info| info.version.clone())
            .filter(|version| !version.trim().is_empty()),
        auth_methods: Vec::new(),
        models: state
            .model_options()
            .into_iter()
            .map(|(id, label)| AcpModelInfo { id, label })
            .collect(),
        error: None,
    }
}

fn display_name(info: &agent_client_protocol::schema::Implementation) -> String {
    info.title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| info.name.clone())
}

/// 在独立临时目录跑一次 ACP 会话，返回 agent 身份与模型候选。
#[cfg_attr(test, allow(dead_code))]
pub async fn probe_agent(config: &AcpAgentConfig, handle: tokio::runtime::Handle) -> AcpAgentProbe {
    let Ok(cwd) = probe_directory() else {
        return AcpAgentProbe {
            error: Some("ACP probe directory is unavailable".to_string()),
            ..Default::default()
        };
    };
    let mut probe_config = config.clone();
    probe_config.timeouts.connect = PROBE_TIMEOUT;
    probe_config.timeouts.authenticate = PROBE_TIMEOUT;

    let attempt = tokio::time::timeout(
        PROBE_TIMEOUT + Duration::from_secs(3),
        AcpConnection::connect_with_runtime(&probe_config, cwd, handle),
    )
    .await;

    match attempt {
        Ok(Ok(AcpConnectOutcome::Ready(connection))) => probe_from_state(&connection.state()),
        // 需要登录也能证明命令确实是一个 ACP agent；模型留待真正连接后再取。
        Ok(Ok(AcpConnectOutcome::AuthenticationRequired(pending))) => AcpAgentProbe {
            auth_methods: pending.methods(),
            ..Default::default()
        },
        Ok(Err(error)) => AcpAgentProbe {
            error: Some(error.to_string()),
            ..Default::default()
        },
        Err(_) => AcpAgentProbe {
            error: Some("ACP agent probe timed out".to_string()),
            ..Default::default()
        },
    }
}

fn probe_directory() -> anyhow::Result<PathBuf> {
    let directory = std::env::temp_dir().join(PROBE_DIRECTORY);
    std::fs::create_dir_all(&directory)?;
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use agent_client_protocol::schema::{
        NewSessionResponse, SessionConfigOption, SessionConfigSelectOption,
    };

    use super::*;

    #[test]
    fn reports_identity_and_advertised_models() {
        let mut state = AcpSessionState::default();
        state.set_agent_info(Some(agent_client_protocol::schema::Implementation::new(
            "codex", "1.2.3",
        )));
        state.apply_new_session_response(
            &NewSessionResponse::new("s1").config_options(vec![SessionConfigOption::select(
                "model",
                "Model",
                "gpt-5",
                vec![
                    SessionConfigSelectOption::new("gpt-5", "GPT-5"),
                    SessionConfigSelectOption::new("gpt-5-mini", "GPT-5 mini"),
                ],
            )]),
        );

        let probe = probe_from_state(&state);

        assert!(probe.identified());
        assert_eq!(Some("codex".to_string()), probe.name);
        assert_eq!(Some("1.2.3".to_string()), probe.version);
        let ids: Vec<&str> = probe.models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(vec!["gpt-5", "gpt-5-mini"], ids);
        assert_eq!("GPT-5 mini", probe.models[1].label);
    }

    #[test]
    fn prefers_the_human_readable_title_for_the_name() {
        let mut state = AcpSessionState::default();
        state.set_agent_info(Some(
            agent_client_protocol::schema::Implementation::new("codex-acp", "1.0.0")
                .title("Codex"),
        ));

        assert_eq!(Some("Codex".to_string()), probe_from_state(&state).name);
    }

    #[test]
    fn without_session_metadata_reports_nothing() {
        let probe = probe_from_state(&AcpSessionState::default());

        assert!(probe.identified());
        assert_eq!(None, probe.name);
        assert_eq!(None, probe.version);
        assert!(probe.models.is_empty());
    }

    #[test]
    fn probe_directory_stays_out_of_the_home_directory() {
        let directory = probe_directory().expect("probe directory");

        assert!(directory.starts_with(std::env::temp_dir()));
        if let Some(home) = dirs_home() {
            assert_ne!(directory, home);
            assert!(!directory.starts_with(home));
        }
    }

    fn dirs_home() -> Option<PathBuf> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| !home.as_os_str().is_empty())
    }
}
