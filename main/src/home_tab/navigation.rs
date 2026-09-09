use super::*;
use crate::navigation_applications::NavigationApplication;

impl HomePage {
    pub(crate) fn activate_navigation_application(
        &mut self,
        application: NavigationApplication,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match application {
            NavigationApplication::AiWorkbench => self.add_ai_workbench_tab(window, cx),
            NavigationApplication::Team => self.open_team_management(window, cx),
            NavigationApplication::Notes => self.add_notes_tab(window, cx),
            NavigationApplication::Toolbox => self.add_toolbox_tab(window, cx),
            NavigationApplication::JsonFormatter => self.add_json_formatter_tab(window, cx),
            NavigationApplication::SessionLogs => self.add_session_logs_tab(window, cx),
            NavigationApplication::CredentialVault => {
                self.add_credential_vault_tab(window, cx);
            }
            NavigationApplication::KnownHosts => self.add_known_hosts_tab(window, cx),
            NavigationApplication::Extensions => self.add_extensions_tab(window, cx),
        }
    }
}
