use gpui::{
    AnyElement, App, AppContext as _, Entity, FocusHandle, Focusable as _, IntoElement,
    ParentElement, Styled, Window, div,
};
use nsh_platform_gpui::{ComponentContext, NativeComponent};
use one_core::storage::ConnectionType;
use serde_json::Value;
use terminal_view::TerminalView;

use crate::NavopNshServices;

/// Embeds the full terminal implementation for serial and Telnet sessions.
///
/// nsh-base owns the VT renderer for local/SSH terminals. Serial and Telnet
/// additionally need protocol negotiation, login scripts and backend-specific
/// lifecycle management; those remain in `terminal`/`terminal_view`.
pub struct TerminalHost {
    context: ComponentContext,
    connection_id: Option<i64>,
    view: Option<Entity<TerminalView>>,
    focus: Option<FocusHandle>,
    error: Option<String>,
}

impl TerminalHost {
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
                "Terminal connection #{connection_id} was not found"
            ));
            return;
        };
        let view = match connection.connection_type {
            ConnectionType::Serial => cx.new(|cx| TerminalView::new_serial(connection, window, cx)),
            ConnectionType::Telnet => cx.new(|cx| TerminalView::new_telnet(connection, window, cx)),
            ConnectionType::SshSftp => cx.new(|cx| TerminalView::new_ssh(connection, window, cx)),
            other => {
                self.error = Some(format!(
                    "{} connections are not supported by the advanced terminal",
                    other.label()
                ));
                return;
            }
        };
        self.focus = Some(view.read(cx).focus_handle(cx));
        self.view = Some(view);
        self.context.redraw();
    }
}

impl NativeComponent for TerminalHost {
    fn render(&mut self, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.ensure_started(window, cx);
        if let Some(view) = &self.view {
            return Some(view.clone().into_any_element());
        }
        Some(
            div()
                .size_full()
                .p_4()
                .child(
                    self.error
                        .clone()
                        .unwrap_or_else(|| "Select a serial or Telnet connection".to_owned()),
                )
                .into_any_element(),
        )
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
