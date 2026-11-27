use gpui::{
    AnyElement, App, AppContext as _, Entity, FocusHandle, Focusable as _, IntoElement,
    ParentElement, Styled, Window, div,
};
use nsh_platform_gpui::{ComponentContext, NativeComponent};
use remote_desktop_view::{RemoteDesktopView, RemoteDesktopViewConfig};
use serde_json::Value;

use crate::NavopNshServices;

/// Embeds the existing GPUI remote-desktop implementation in an nsh page.
pub struct RemoteDesktopHost {
    context: ComponentContext,
    connection_id: Option<i64>,
    view: Option<Entity<RemoteDesktopView>>,
    focus: Option<FocusHandle>,
    error: Option<String>,
}

impl RemoteDesktopHost {
    pub fn new(context: ComponentContext) -> Self {
        Self {
            context,
            connection_id: None,
            view: None,
            focus: None,
            error: None,
        }
    }

    fn ensure_started(&mut self, window: &mut Window, cx: &mut App) {
        if self.view.is_some() || self.error.is_some() {
            return;
        }
        let Some(connection_id) = self.connection_id else {
            return;
        };
        let Some(services) = NavopNshServices::current() else {
            self.error = Some("Navop services are unavailable".to_owned());
            return;
        };
        let Some(connection) = services
            .resolved_connections
            .iter()
            .find(|connection| connection.id == Some(connection_id))
            .cloned()
        else {
            self.error = Some(format!(
                "Remote desktop connection #{connection_id} was not found"
            ));
            return;
        };
        let params = match connection.to_remote_desktop_params() {
            Ok(params) => params,
            Err(error) => {
                self.error = Some(format!("Invalid remote desktop parameters: {error}"));
                return;
            }
        };
        let title = connection.name.clone();
        let view = cx.new(|cx| {
            RemoteDesktopView::new(
                RemoteDesktopViewConfig {
                    options: remote_desktop::RemoteDesktopConnectionOptions::from_storage_params(
                        params,
                    ),
                    title,
                    tab_index: None,
                },
                window.window_handle(),
                cx,
            )
        });
        self.focus = Some(view.read(cx).focus_handle(cx));
        self.view = Some(view);
        self.context.redraw();
    }
}

impl NativeComponent for RemoteDesktopHost {
    fn render(&mut self, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.ensure_started(window, cx);
        if let Some(view) = &self.view {
            return Some(view.clone().into_any_element());
        }
        let message = self
            .error
            .clone()
            .unwrap_or_else(|| "Select an RDP or VNC connection".to_owned());
        Some(div().size_full().p_4().child(message).into_any_element())
    }

    fn set_prop(&mut self, name: &str, value: Option<&Value>, _window: &mut Window, _cx: &mut App) {
        if name != "connection_id" {
            return;
        }
        let next = value.and_then(Value::as_i64);
        if self.connection_id != next {
            self.connection_id = next;
            self.view = None;
            self.focus = None;
            self.error = None;
            self.context.redraw();
        }
    }

    fn focus_handle(&self) -> Option<FocusHandle> {
        self.focus.clone()
    }
}
