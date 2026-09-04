mod delegate;
#[cfg(test)]
mod tests;

use std::rc::Rc;

use gpui::{
    App, AppContext as _, InteractiveElement as _, ParentElement as _, Styled as _, Window, div, px,
};
use gpui_component::{
    Sizable as _, Size, WindowExt as _,
    list::{List, ListState},
};
use one_core::storage::ConnectionType;
use rust_i18n::t;

use delegate::NavigationQuickOpenDelegate;

const DIALOG_WIDTH: gpui::Pixels = px(440.0);
const DIALOG_MARGIN_TOP: gpui::Pixels = px(72.0);
const DIALOG_LIST_MAX_HEIGHT: gpui::Pixels = px(420.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NavigationApplication {
    AiWorkbench,
    Team,
    Notes,
    #[cfg(feature = "api-testing")]
    ApiTesting,
    JsonFormatter,
    SessionLogs,
    CredentialVault,
    Extensions,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum NavigationTarget {
    Connection(ConnectionType),
    Application(NavigationApplication),
}

pub(crate) type NavigationActivate = Rc<dyn Fn(NavigationTarget, &mut Window, &mut App) + 'static>;

#[derive(Clone, Copy)]
pub(crate) struct NavigationAvailability {
    pub show_ai_workbench: bool,
    pub show_team: bool,
}

pub(crate) struct NavigationQuickOpenRequest {
    title: String,
    search_placeholder: String,
    items: Vec<NavigationQuickOpenItem>,
    on_activate: NavigationActivate,
}

#[derive(Clone)]
pub(super) struct NavigationQuickOpenItem {
    pub target: NavigationTarget,
    pub label: String,
    pub selected: bool,
}

pub(crate) fn visible_connection_types() -> Vec<ConnectionType> {
    vec![
        ConnectionType::All,
        ConnectionType::SshSftp,
        ConnectionType::Database,
        ConnectionType::Redis,
        ConnectionType::MongoDB,
        ConnectionType::Mqtt,
        ConnectionType::Serial,
        ConnectionType::Telnet,
    ]
}

pub(crate) fn overflow_connection_types() -> Vec<ConnectionType> {
    ConnectionType::all()
        .into_iter()
        .filter(|connection_type| is_overflow_connection_type(*connection_type))
        .collect()
}

pub(crate) fn is_overflow_connection_type(connection_type: ConnectionType) -> bool {
    matches!(
        connection_type,
        ConnectionType::PortForwarding
            | ConnectionType::Rdp
            | ConnectionType::Vnc
            | ConnectionType::Extension
    )
}

pub(crate) fn leading_navigation_applications(
    availability: NavigationAvailability,
) -> Vec<NavigationApplication> {
    let mut applications = Vec::new();
    if availability.show_ai_workbench {
        applications.push(NavigationApplication::AiWorkbench);
    }
    if availability.show_team {
        applications.push(NavigationApplication::Team);
    }
    applications.push(NavigationApplication::Notes);
    #[cfg(feature = "api-testing")]
    applications.push(NavigationApplication::ApiTesting);
    applications.push(NavigationApplication::Extensions);
    applications
}

pub(crate) fn overflow_navigation_applications() -> Vec<NavigationApplication> {
    vec![
        NavigationApplication::SessionLogs,
        NavigationApplication::CredentialVault,
        NavigationApplication::JsonFormatter,
    ]
}

/// Every navigation application in display order: the pinned entries and the
/// secondary entries. The modern home lays these out as tiles instead of
/// collapsing them behind a menu. Settings is deliberately absent: its global
/// entry moved to the tab bar.
pub(crate) fn all_navigation_applications(
    availability: NavigationAvailability,
) -> Vec<NavigationApplication> {
    let mut applications = leading_navigation_applications(availability);
    applications.extend(overflow_navigation_applications());
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
            #[cfg(feature = "api-testing")]
            Self::ApiTesting => t!("Home.api_testing").to_string(),
            Self::JsonFormatter => t!("Home.json_formatter").to_string(),
            Self::SessionLogs => t!("Home.session_logs").to_string(),
            Self::CredentialVault => t!("Home.credential_vault").to_string(),
            Self::Extensions => t!("Home.extensions").to_string(),
        }
    }

    pub(crate) fn icon(self) -> gpui_component::IconName {
        delegate::application_icon(self)
    }
}

impl NavigationQuickOpenItem {
    fn connection(connection_type: ConnectionType, selected: ConnectionType) -> Self {
        Self {
            target: NavigationTarget::Connection(connection_type),
            label: connection_type.label().to_string(),
            selected: connection_type == selected,
        }
    }

    fn application(application: NavigationApplication) -> Self {
        Self {
            target: NavigationTarget::Application(application),
            label: application.label(),
            selected: false,
        }
    }
}

impl NavigationQuickOpenRequest {
    pub(crate) fn connections(
        selected_filter: ConnectionType,
        on_activate: NavigationActivate,
    ) -> Self {
        Self {
            title: t!("Home.more_connection_types").to_string(),
            search_placeholder: t!("Home.search_connection_types").to_string(),
            items: overflow_connection_types()
                .into_iter()
                .map(|filter| NavigationQuickOpenItem::connection(filter, selected_filter))
                .collect(),
            on_activate,
        }
    }

    pub(crate) fn applications(on_activate: NavigationActivate) -> Self {
        Self {
            title: t!("Home.more_applications").to_string(),
            search_placeholder: t!("Home.search_applications").to_string(),
            items: overflow_navigation_applications()
                .into_iter()
                .map(NavigationQuickOpenItem::application)
                .collect(),
            on_activate,
        }
    }
}

pub(crate) fn show_navigation_quick_open(
    request: NavigationQuickOpenRequest,
    window: &mut Window,
    cx: &mut App,
) {
    let list = cx.new(|cx| {
        let delegate = NavigationQuickOpenDelegate::new(request.items, request.on_activate);
        ListState::new(delegate, window, cx).searchable(true)
    });
    let list_for_focus = list.clone();
    let title = request.title;
    let search_placeholder = request.search_placeholder;
    window.open_dialog(cx, move |dialog, _window, _cx| {
        dialog
            .title(title.clone())
            .w(DIALOG_WIDTH)
            .margin_top(DIALOG_MARGIN_TOP)
            .close_button(false)
            .content({
                let list = list.clone();
                let search_placeholder = search_placeholder.clone();
                move |content, _window, _cx| {
                    content.p_0().child(
                        div().id("navigation-quick-open-dialog").pb_2().child(
                            List::new(&list)
                                .search_placeholder(search_placeholder.clone())
                                .with_size(Size::Large)
                                .max_h(DIALOG_LIST_MAX_HEIGHT),
                        ),
                    )
                }
            })
    });
    list_for_focus.update(cx, |state, cx| state.focus(window, cx));
}
