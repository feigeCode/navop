use std::rc::Rc;

use super::*;
use crate::navigation_quick_open::{
    NavigationApplication, NavigationQuickOpenRequest, NavigationTarget, show_navigation_quick_open,
};

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
            NavigationApplication::JsonFormatter => self.add_json_formatter_tab(window, cx),
            NavigationApplication::SessionLogs => self.add_session_logs_tab(window, cx),
            NavigationApplication::CredentialVault => {
                self.add_credential_vault_tab(window, cx);
            }
            NavigationApplication::Extensions => self.add_extensions_tab(window, cx),
        }
    }

    pub(super) fn show_legacy_connection_navigation_quick_open(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let home = cx.entity();
        let on_activate = Rc::new(move |target, _window: &mut Window, cx: &mut App| {
            if let NavigationTarget::Connection(filter) = target {
                home.update(cx, |home, cx| home.set_selected_filter(filter, cx));
            }
        });
        let request = NavigationQuickOpenRequest::connections(self.selected_filter, on_activate);
        show_navigation_quick_open(request, window, cx);
    }

    pub(crate) fn show_application_navigation_quick_open(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let home = cx.entity();
        let on_activate = Rc::new(move |target, window: &mut Window, cx: &mut App| {
            if let NavigationTarget::Application(application) = target {
                home.update(cx, |home, cx| {
                    home.activate_navigation_application(application, window, cx);
                });
            }
        });
        let request = NavigationQuickOpenRequest::applications(on_activate);
        show_navigation_quick_open(request, window, cx);
    }
}
