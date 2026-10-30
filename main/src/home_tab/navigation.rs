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
            NavigationApplication::Team => self.add_team_tab(window, cx),
            NavigationApplication::Notes => self.add_notes_tab(window, cx),
            NavigationApplication::Toolbox => self.add_toolbox_tab(window, cx),
            NavigationApplication::JsonFormatter => self.add_json_formatter_tab(window, cx),
            NavigationApplication::SessionLogs => {
                let _ = app_kit::registry().open_tab(
                    &domain_api::TabIntent::SessionLogs,
                    TabOpenMode::Activate,
                    self,
                    window,
                    cx,
                );
            }
            NavigationApplication::CredentialVault => {
                self.add_credential_vault_tab(window, cx);
            }
            NavigationApplication::KnownHosts => {
                let _ = app_kit::registry().open_tab(
                    &domain_api::TabIntent::KnownHosts,
                    TabOpenMode::Activate,
                    self,
                    window,
                    cx,
                );
            }
            NavigationApplication::Extensions => self.add_extensions_tab(window, cx),
            NavigationApplication::Settings => self.add_settings_tab(window, cx),
        }
    }
}
