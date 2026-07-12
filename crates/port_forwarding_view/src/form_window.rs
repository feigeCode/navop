use connection_form::team::{
    TeamSelectItem, create_team_select, resolve_team_assignment, selected_team_id,
};
use gpui::{App, AppContext, Context, Entity, FocusHandle, Window};
use gpui_component::{IndexPath, input::InputState, select::SelectState};
use one_core::cloud_sync::TeamOption;
use one_core::storage::{
    ConnectionType, PortForwardingKind, PortForwardingParams, StoredConnection, Workspace,
};
use rust_i18n::t;

use crate::input_values::{non_empty_text, parse_port, trimmed_text};
use crate::persistence::save_connection;
use crate::selects::{ForwardingKindSelectItem, SshConnectionSelectItem, WorkspaceSelectItem};

pub struct PortForwardingFormWindowConfig {
    pub editing_connection: Option<StoredConnection>,
    pub ssh_connections: Vec<StoredConnection>,
    pub workspaces: Vec<Workspace>,
    pub teams: Vec<TeamOption>,
}

pub struct PortForwardingFormWindow {
    pub(super) focus_handle: FocusHandle,
    pub(super) is_editing: bool,
    pub(super) editing_id: Option<i64>,
    pub(super) editing_cloud_id: Option<String>,
    pub(super) editing_last_synced_at: Option<i64>,
    pub(super) editing_owner_id: Option<String>,
    pub(super) name_input: Entity<InputState>,
    pub(super) bind_host_input: Entity<InputState>,
    pub(super) bind_port_input: Entity<InputState>,
    pub(super) target_host_input: Entity<InputState>,
    pub(super) target_port_input: Entity<InputState>,
    pub(super) remark_input: Entity<InputState>,
    pub(super) ssh_select: Entity<SelectState<Vec<SshConnectionSelectItem>>>,
    pub(super) kind_select: Entity<SelectState<Vec<ForwardingKindSelectItem>>>,
    pub(super) workspace_select: Entity<SelectState<Vec<WorkspaceSelectItem>>>,
    pub(super) team_select: Entity<SelectState<Vec<TeamSelectItem>>>,
    pub(super) sync_enabled: bool,
    pub(super) validation_error: Option<String>,
}

impl PortForwardingFormWindow {
    pub fn new(
        config: PortForwardingFormWindowConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let is_editing = config.editing_connection.is_some();
        let name_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("PortForwarding.name_placeholder"))
        });
        let bind_host_input = cx.new(|cx| InputState::new(window, cx).placeholder("127.0.0.1"));
        let bind_port_input = cx.new(|cx| InputState::new(window, cx).placeholder("0"));
        let target_host_input = cx.new(|cx| InputState::new(window, cx).placeholder("127.0.0.1"));
        let target_port_input = cx.new(|cx| InputState::new(window, cx).placeholder("3306"));
        let remark_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("PortForwarding.remark_placeholder"))
                .auto_grow(3, 10)
        });

        let ssh_items: Vec<_> = config
            .ssh_connections
            .iter()
            .filter(|conn| conn.connection_type == ConnectionType::SshSftp)
            .filter_map(SshConnectionSelectItem::from_connection)
            .collect();
        let ssh_index = (!ssh_items.is_empty()).then(|| IndexPath::default());
        let ssh_select = cx.new(|cx| SelectState::new(ssh_items, ssh_index, window, cx));
        let kind_select = cx.new(|cx| {
            SelectState::new(
                ForwardingKindSelectItem::all(),
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let workspace_select = cx.new(|cx| {
            let mut items = vec![WorkspaceSelectItem::none()];
            items.extend(
                config
                    .workspaces
                    .iter()
                    .map(WorkspaceSelectItem::from_workspace),
            );
            SelectState::new(items, Some(IndexPath::default()), window, cx)
        });
        let team_select = create_team_select(&config.teams, None, window, cx);

        let mut form = Self {
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
            editing_owner_id: config
                .editing_connection
                .as_ref()
                .and_then(|c| c.owner_id.clone()),
            name_input,
            bind_host_input,
            bind_port_input,
            target_host_input,
            target_port_input,
            remark_input,
            ssh_select,
            kind_select,
            workspace_select,
            team_select,
            sync_enabled: true,
            validation_error: None,
        };
        form.load_editing_connection(config.editing_connection.as_ref(), window, cx);
        form
    }

    fn load_editing_connection(
        &mut self,
        connection: Option<&StoredConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = connection else { return };
        self.sync_enabled = connection.sync_enabled;
        self.name_input.update(cx, |state, cx| {
            state.set_value(&connection.name, window, cx)
        });
        if let Some(remark) = &connection.remark {
            self.remark_input
                .update(cx, |state, cx| state.set_value(remark, window, cx));
        }
        if let Ok(params) = connection.to_port_forwarding_params() {
            self.apply_params(&params, window, cx);
        }
        if let Some(workspace_id) = connection.workspace_id {
            self.workspace_select.update(cx, |select, cx| {
                select.set_selected_value(&Some(workspace_id), window, cx);
            });
        }
        if let Some(team_id) = &connection.team_id {
            self.team_select.update(cx, |select, cx| {
                select.set_selected_value(&Some(team_id.clone()), window, cx);
            });
        }
    }

    fn apply_params(
        &mut self,
        params: &PortForwardingParams,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ssh_select.update(cx, |select, cx| {
            select.set_selected_value(&params.ssh_connection_id, window, cx)
        });
        self.kind_select.update(cx, |select, cx| {
            select.set_selected_value(&params.kind, window, cx)
        });
        self.bind_host_input.update(cx, |state, cx| {
            state.set_value(&params.bind_host, window, cx)
        });
        self.bind_port_input.update(cx, |state, cx| {
            state.set_value(&params.bind_port.to_string(), window, cx)
        });
        self.target_host_input.update(cx, |state, cx| {
            state.set_value(&params.target_host, window, cx)
        });
        self.target_port_input.update(cx, |state, cx| {
            state.set_value(&params.target_port.to_string(), window, cx)
        });
    }

    fn build_params(&self, cx: &App) -> Result<PortForwardingParams, String> {
        let ssh_connection_id = self
            .ssh_select
            .read(cx)
            .selected_value()
            .copied()
            .ok_or_else(|| t!("PortForwarding.validation_ssh").to_string())?;
        let kind = self
            .kind_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or_default();
        let bind_host = trimmed_text(&self.bind_host_input, cx);
        if bind_host.is_empty() {
            return Err(t!("PortForwarding.validation_bind_host").to_string());
        }
        let bind_port = parse_port(&self.bind_port_input, &t!("PortForwarding.bind_port"), cx)?;
        let (target_host, target_port) = if kind == PortForwardingKind::Local {
            let target_host = trimmed_text(&self.target_host_input, cx);
            let target_port = parse_port(
                &self.target_port_input,
                &t!("PortForwarding.target_port"),
                cx,
            )?;
            if target_host.is_empty() {
                return Err(t!("PortForwarding.validation_target_host").to_string());
            }
            if target_port == 0 {
                return Err(t!("PortForwarding.validation_target_port").to_string());
            }
            (target_host, target_port)
        } else {
            (String::new(), 0)
        };
        Ok(PortForwardingParams {
            ssh_connection_id,
            kind,
            bind_host,
            bind_port,
            target_host,
            target_port,
        })
    }

    pub(super) fn on_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let params = match self.build_params(cx) {
            Ok(params) => params,
            Err(error) => {
                self.validation_error = Some(error);
                cx.notify();
                return;
            }
        };
        let name = self.connection_name(&params, cx);
        let mut conn = StoredConnection::new_port_forwarding(name, params, self.workspace_id(cx));
        conn.sync_enabled = self.sync_enabled;
        let assignment = match resolve_team_assignment(
            self.team_id(cx),
            self.is_editing,
            self.editing_owner_id.clone(),
            cx,
        ) {
            Ok(assignment) => assignment,
            Err(error) => {
                self.validation_error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        conn.team_id = assignment.team_id;
        conn.owner_id = assignment.owner_id;
        conn.remark = non_empty_text(&self.remark_input, cx);
        if self.is_editing {
            conn.id = self.editing_id;
            conn.cloud_id = self.editing_cloud_id.clone();
            conn.last_synced_at = self.editing_last_synced_at;
        }
        save_connection(conn, self.is_editing, cx);
        window.remove_window();
    }

    fn connection_name(&self, params: &PortForwardingParams, cx: &App) -> String {
        let name = trimmed_text(&self.name_input, cx);
        if !name.is_empty() {
            return name;
        }
        match params.kind {
            PortForwardingKind::Local => format!(
                "{}:{} -> {}:{}",
                params.bind_host, params.bind_port, params.target_host, params.target_port
            ),
            PortForwardingKind::Dynamic => {
                format!("SOCKS {}:{}", params.bind_host, params.bind_port)
            }
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
