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
    Extensions,
}

pub(crate) fn home_applications(show_team: bool) -> Vec<NavigationApplication> {
    use NavigationApplication::*;
    let mut applications = vec![AiWorkbench];
    if show_team {
        applications.push(Team);
    }
    applications.extend([Notes, Extensions, Toolbox, CredentialVault, SessionLogs]);
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
            Self::Extensions => t!("Home.extensions").to_string(),
        }
    }

    pub(crate) fn icon(self) -> gpui_component::IconName {
        match self {
            Self::AiWorkbench => gpui_component::IconName::AILine,
            Self::Team => gpui_component::IconName::TeamLine,
            Self::Notes => gpui_component::IconName::NotesLine,
            Self::Toolbox => gpui_component::IconName::LayoutDashboard,
            Self::JsonFormatter => gpui_component::IconName::Json,
            Self::SessionLogs => gpui_component::IconName::Terminal,
            Self::CredentialVault => gpui_component::IconName::Key,
            Self::Extensions => gpui_component::IconName::ExtensionsLine,
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
                SessionLogs
            ]
        );
        assert!(!home_applications(true).contains(&JsonFormatter));
    }
}
