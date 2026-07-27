mod inputs;
mod persistence;
mod proxy;
mod selects;
mod view;

use connection_form::team::{
    TeamSelectItem, create_team_select, refresh_team_options, resolve_team_assignment,
    selected_team_id,
};
use gpui::{App, Context, Entity, FocusHandle, Window};
use gpui_component::input::InputState;
use gpui_component::select::SelectState;
use one_core::cloud_sync::TeamOption;
use one_core::storage::{
    ProxyType, RemoteDesktopParams, RemoteDesktopProtocol, StoredConnection, Workspace,
};
use rust_i18n::t;

use self::inputs::{create_inputs, input_text, non_empty_text, parse_u16};
use self::persistence::{emit_saved_connection, persist_connection};
use self::selects::{WorkspaceSelectItem, create_workspace_select};

pub struct RemoteDesktopFormWindowConfig {
    pub protocol: RemoteDesktopProtocol,
    pub editing_connection: Option<StoredConnection>,
    pub workspaces: Vec<Workspace>,
    pub teams: Vec<TeamOption>,
}

pub struct RemoteDesktopFormWindow {
    protocol: RemoteDesktopProtocol,
    focus_handle: FocusHandle,
    is_editing: bool,
    editing_id: Option<i64>,
    editing_cloud_id: Option<String>,
    editing_last_synced_at: Option<i64>,
    editing_connection: Option<StoredConnection>,
    name_input: Entity<InputState>,
    host_input: Entity<InputState>,
    port_input: Entity<InputState>,
    username_input: Entity<InputState>,
    password_input: Entity<InputState>,
    domain_input: Entity<InputState>,
    proxy_host_input: Entity<InputState>,
    proxy_port_input: Entity<InputState>,
    proxy_username_input: Entity<InputState>,
    proxy_password_input: Entity<InputState>,
    workspace_select: Entity<SelectState<Vec<WorkspaceSelectItem>>>,
    team_select: Entity<SelectState<Vec<TeamSelectItem>>>,
    read_only: bool,
    audio_playback: bool,
    proxy_enabled: bool,
    proxy_type: ProxyType,
    sync_enabled: bool,
    error: Option<String>,
}

impl RemoteDesktopFormWindow {
    pub fn new(
        config: RemoteDesktopFormWindowConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut form = Self::empty(config, window, cx);
        form.load_editing_connection(window, cx);
        form.focus_handle.focus(window, cx);
        form
    }

    fn empty(
        config: RemoteDesktopFormWindowConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let is_editing = config.editing_connection.is_some();
        let inputs = create_inputs(config.protocol, window, cx);
        let editing_connection = config.editing_connection.clone();
        Self {
            protocol: config.protocol,
            focus_handle: cx.focus_handle(),
            is_editing,
            editing_id: config.editing_connection.as_ref().and_then(|c| c.id),
            editing_cloud_id: config
                .editing_connection
                .as_ref()
                .and_then(|c| c.cloud_id.clone()),
            editing_last_synced_at: config
                .editing_connection
                .as_ref()
                .and_then(|c| c.last_synced_at),
            editing_connection,
            name_input: inputs.name,
            host_input: inputs.host,
            port_input: inputs.port,
            username_input: inputs.username,
            password_input: inputs.password,
            domain_input: inputs.domain,
            proxy_host_input: inputs.proxy_host,
            proxy_port_input: inputs.proxy_port,
            proxy_username_input: inputs.proxy_username,
            proxy_password_input: inputs.proxy_password,
            workspace_select: create_workspace_select(&config, window, cx),
            team_select: create_team_select(&config.teams, None, window, cx),
            read_only: false,
            audio_playback: false,
            proxy_enabled: false,
            proxy_type: ProxyType::Socks5,
            sync_enabled: config
                .editing_connection
                .as_ref()
                .map(|connection| connection.sync_enabled)
                .unwrap_or(true),
            error: None,
        }
    }

    fn load_editing_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.editing_connection.clone() else {
            return;
        };
        self.name_input.update(cx, |state, cx| {
            state.set_value(&connection.name, window, cx)
        });
        if let Some(team_id) = &connection.team_id {
            self.team_select.update(cx, |state, cx| {
                state.set_selected_value(&Some(team_id.clone()), window, cx)
            });
        }
        if let Some(workspace_id) = connection.workspace_id {
            self.workspace_select.update(cx, |state, cx| {
                state.set_selected_value(&Some(workspace_id), window, cx)
            });
        }
        if let Ok(params) = connection.to_remote_desktop_params() {
            self.apply_params(params, window, cx);
        }
    }

    fn apply_params(
        &mut self,
        params: RemoteDesktopParams,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = params.host;
        let username = params.username.unwrap_or_default();
        let password = params.password.unwrap_or_default();
        let domain = params.domain.unwrap_or_default();
        self.host_input
            .update(cx, |state, cx| state.set_value(&host, window, cx));
        self.port_input.update(cx, |state, cx| {
            state.set_value(&params.port.to_string(), window, cx)
        });
        self.username_input
            .update(cx, |state, cx| state.set_value(&username, window, cx));
        self.password_input
            .update(cx, |state, cx| state.set_value(&password, window, cx));
        self.domain_input
            .update(cx, |state, cx| state.set_value(&domain, window, cx));
        self.read_only = params.read_only;
        self.audio_playback = audio_playback_for_protocol(self.protocol, params.audio_playback);
        self.apply_proxy(params.proxy, window, cx);
    }

    fn build_params(&self, cx: &App) -> Result<RemoteDesktopParams, String> {
        let host = input_text(&self.host_input, cx).trim().to_string();
        if host.is_empty() {
            return Err(t!("RemoteDesktopForm.host_required").to_string());
        }
        let port_label = t!("RemoteDesktopForm.label_port").to_string();
        let proxy = proxy::build_proxy_config(
            self.proxy_enabled,
            self.proxy_type,
            &input_text(&self.proxy_host_input, cx),
            &input_text(&self.proxy_port_input, cx),
            &input_text(&self.proxy_username_input, cx),
            &input_text(&self.proxy_password_input, cx),
        )
        .map_err(proxy::proxy_error_message)?;
        Ok(RemoteDesktopParams {
            protocol: self.protocol,
            host,
            port: parse_u16(&input_text(&self.port_input, cx), &port_label)?,
            username: non_empty_text(&self.username_input, cx),
            password: non_empty_text(&self.password_input, cx),
            domain: non_empty_text(&self.domain_input, cx),
            read_only: self.read_only,
            audio_playback: audio_playback_for_protocol(self.protocol, self.audio_playback),
            proxy,
        })
    }

    fn on_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self
            .build_params(cx)
            .and_then(|params| self.save_connection(params, cx).map_err(|e| e.to_string()))
        {
            Ok(connection) => {
                emit_saved_connection(connection, self.is_editing, cx);
                window.remove_window();
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn save_connection(
        &self,
        params: RemoteDesktopParams,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<StoredConnection> {
        let mut connection = StoredConnection::new_remote_desktop(
            self.connection_name(&params, cx),
            params,
            self.workspace_id(cx),
        );
        connection.sync_enabled = self.sync_enabled;
        let assignment = resolve_team_assignment(
            self.team_id(cx),
            self.is_editing,
            self.editing_connection
                .as_ref()
                .and_then(|connection| connection.owner_id.clone()),
            cx,
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        connection.team_id = assignment.team_id;
        connection.owner_id = assignment.owner_id;
        if self.is_editing {
            connection.id = self.editing_id;
            connection.cloud_id = self.editing_cloud_id.clone();
            connection.last_synced_at = self.editing_last_synced_at;
        }
        persist_connection(connection, self.is_editing, cx)
    }

    fn request_team_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        refresh_team_options(&self.team_select, window, cx);
    }

    fn connection_name(&self, params: &RemoteDesktopParams, cx: &App) -> String {
        let name = input_text(&self.name_input, cx).trim().to_string();
        if name.is_empty() {
            format!("{}:{}", params.host, params.port)
        } else {
            name
        }
    }

    fn workspace_id(&self, cx: &App) -> Option<i64> {
        self.workspace_select
            .read(cx)
            .selected_value()
            .cloned()
            .flatten()
    }

    fn team_id(&self, cx: &App) -> Option<String> {
        selected_team_id(&self.team_select, cx)
    }
}

fn audio_playback_for_protocol(protocol: RemoteDesktopProtocol, enabled: bool) -> bool {
    protocol == RemoteDesktopProtocol::Rdp && enabled
}

#[cfg(test)]
mod tests {
    use one_core::storage::RemoteDesktopProtocol;

    use super::audio_playback_for_protocol;

    #[test]
    fn audio_playback_is_only_enabled_for_rdp() {
        assert!(audio_playback_for_protocol(
            RemoteDesktopProtocol::Rdp,
            true
        ));
        assert!(!audio_playback_for_protocol(
            RemoteDesktopProtocol::Vnc,
            true
        ));
        assert!(!audio_playback_for_protocol(
            RemoteDesktopProtocol::Rdp,
            false
        ));
    }
}
