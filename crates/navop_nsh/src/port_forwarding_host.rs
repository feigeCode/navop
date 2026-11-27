use std::sync::Arc;

use gpui::{
    AnyElement, App, AppContext as _, Entity, FocusHandle, Focusable as _, IntoElement,
    ParentElement, Styled, Window, div,
};
use nsh_platform_gpui::{ComponentContext, NativeComponent};
use one_core::storage::ConnectionType;
use port_forwarding::PortForwardingRuntime;
use port_forwarding_view::{PortForwardingTab, PortForwardingTabConfig};
use serde_json::Value;

use crate::NavopNshServices;

pub struct PortForwardingHost {
    context: ComponentContext,
    connection_id: Option<i64>,
    runtime: Arc<tokio::sync::Mutex<PortForwardingRuntime>>,
    view: Option<Entity<PortForwardingTab>>,
    focus: Option<FocusHandle>,
    error: Option<String>,
}

impl PortForwardingHost {
    pub fn new(context: ComponentContext) -> Self {
        Self {
            context,
            connection_id: None,
            runtime: Arc::new(tokio::sync::Mutex::new(PortForwardingRuntime::new())),
            view: None,
            focus: None,
            error: None,
        }
    }

    fn ensure_started(&mut self, _window: &mut Window, cx: &mut App) {
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
                "Port-forwarding connection #{connection_id} was not found"
            ));
            return;
        };
        if connection.connection_type != ConnectionType::PortForwarding {
            self.error = Some(format!(
                "Connection #{connection_id} is not a port-forwarding connection"
            ));
            return;
        }
        let params = match connection.to_port_forwarding_params() {
            Ok(params) => params,
            Err(error) => {
                self.error = Some(format!("Invalid port-forwarding parameters: {error}"));
                return;
            }
        };
        let Some(ssh_connection) = services
            .resolved_connections
            .iter()
            .find(|candidate| candidate.id == Some(params.ssh_connection_id))
            .cloned()
        else {
            self.error = Some(format!(
                "Referenced SSH connection #{} was not found",
                params.ssh_connection_id
            ));
            return;
        };
        let config =
            match PortForwardingTabConfig::new(connection, ssh_connection, self.runtime.clone()) {
                Ok(config) => config,
                Err(error) => {
                    self.error = Some(format!("Failed to prepare tunnel: {error:#}"));
                    return;
                }
            };
        let view = cx.new(|cx| PortForwardingTab::new(config, cx));
        self.focus = Some(view.read(cx).focus_handle(cx));
        self.view = Some(view);
        self.context.redraw();
    }
}

impl NativeComponent for PortForwardingHost {
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
                        .unwrap_or_else(|| "Select a port-forwarding connection".to_owned()),
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
