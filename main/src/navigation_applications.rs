//! Application destinations shared by Home and the Toolbox registry.
use rust_i18n::t;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NavigationApplication {
    AiWorkbench,
    Team,
    Notes,
    JsonFormatter,
    Toolbox,
    SessionLogs,
    CredentialVault,
    KnownHosts,
    Extensions,
}

pub(crate) fn home_applications(show_team: bool) -> Vec<NavigationApplication> {
    use NavigationApplication::*;
    let mut applications = vec![AiWorkbench];
    if show_team {
        applications.push(Team);
    }
    applications.extend([
        Notes,
        Extensions,
        Toolbox,
        CredentialVault,
        KnownHosts,
        SessionLogs,
    ]);
    applications
}

impl NavigationApplication {
    pub(crate) fn label(self) -> String {
        match self {
            Self::AiWorkbench => {
                t!("Settings.General.Startup.default_page_ai_workbench").to_string()
            }
            Self::Team => t!("TeamManagement.title").to_string(),
            Self::Notes => t!("Home.notes").to_string(),
            Self::Toolbox => t!("Home.toolbox").to_string(),
            Self::JsonFormatter => t!("Home.json_formatter").to_string(),
            Self::SessionLogs => t!("Home.session_logs").to_string(),
            Self::CredentialVault => t!("Home.credential_vault").to_string(),
            Self::KnownHosts => t!("Home.known_hosts").to_string(),
            Self::Extensions => t!("Home.extensions").to_string(),
        }
    }

    pub(crate) fn icon(self) -> one_assets::IconName {
        match self {
            Self::AiWorkbench => one_assets::IconName::AILine,
            Self::Team => one_assets::IconName::TeamLine,
            Self::Notes => one_assets::IconName::NotesLine,
            Self::Toolbox => one_assets::IconName::LayoutDashboard,
            Self::JsonFormatter => one_assets::IconName::Json,
            // 会话日志用线性图标；IconName::Terminal 的默认资源是固定填充彩色 SVG。
            Self::SessionLogs => one_assets::IconName::SquareTerminal,
            Self::CredentialVault => one_assets::IconName::Key,
            // 已知主机使用线性图标，与侧栏其余 *_Line 图标风格一致。
            Self::KnownHosts => one_assets::IconName::ServerLine,
            Self::Extensions => one_assets::IconName::ExtensionsLine,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn home_application_order_and_team_gate_are_stable() {
        use NavigationApplication::*;
        assert_eq!(
            home_applications(true),
            vec![
                AiWorkbench,
                Team,
                Notes,
                Extensions,
                Toolbox,
                CredentialVault,
                KnownHosts,
                SessionLogs
            ]
        );
        assert_eq!(
            home_applications(false),
            vec![
                AiWorkbench,
                Notes,
                Extensions,
                Toolbox,
                CredentialVault,
                KnownHosts,
                SessionLogs
            ]
        );
        assert!(!home_applications(true).contains(&JsonFormatter));
    }
}
