//! MongoDB 连接表单窗口

use connection_form::team::{
    TeamSelectItem, create_team_select, refresh_team_options, refresh_teams_tooltip,
    resolve_team_assignment, selected_team_id, team_label, team_management_enabled,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, IndexPath, Sizable, Size,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState},
    radio::Radio,
    scroll::ScrollableElement,
    select::{Select, SelectItem, SelectState},
    tab::{Tab, TabBar},
    v_flex,
};
use one_core::cloud_sync::TeamOption;
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::gpui_tokio::Tokio;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionType, MongoDBParams, MongoDriverVariant, MongoSshTunnelConfig, StoredConnection,
    Workspace,
};
use rust_i18n::t;
use tracing::error;

use crate::GlobalMongoState;

/// MongoDB 表单窗口配置
pub struct MongoFormWindowConfig {
    pub editing_connection: Option<StoredConnection>,
    pub initial_connection: Option<StoredConnection>,
    pub on_saved: Option<MongoFormSavedCallback>,
    pub workspaces: Vec<Workspace>,
    pub teams: Vec<TeamOption>,
    pub ssh_connections: Vec<StoredConnection>,
}

pub type MongoFormSavedCallback =
    std::sync::Arc<dyn Fn(StoredConnection, &mut App) + Send + Sync + 'static>;

impl MongoFormWindowConfig {
    fn is_editing(&self) -> bool {
        self.editing_connection.is_some()
    }

    fn connection_to_load(&self) -> Option<&StoredConnection> {
        self.editing_connection
            .as_ref()
            .or(self.initial_connection.as_ref())
    }
}

#[derive(Clone, Default, PartialEq)]
struct WorkspaceSelectItem {
    id: Option<i64>,
    name: String,
}

impl WorkspaceSelectItem {
    fn none() -> Self {
        Self {
            id: None,
            name: t!("Common.none").to_string(),
        }
    }

    fn from_workspace(workspace: &Workspace) -> Self {
        Self {
            id: workspace.id,
            name: workspace.name.clone(),
        }
    }
}

impl SelectItem for WorkspaceSelectItem {
    type Value = Option<i64>;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

#[derive(Clone, Default, PartialEq)]
struct SshConnectionSelectItem {
    id: Option<i64>,
    name: String,
}

impl SshConnectionSelectItem {
    fn none() -> Self {
        Self {
            id: None,
            name: t!("ConnectionForm.ssh_connection_manual").to_string(),
        }
    }

    fn from_connection(connection: &StoredConnection) -> Self {
        let host = connection.to_ssh_params().ok().map(|params| params.host);
        let name = match host.as_deref().filter(|host| !host.trim().is_empty()) {
            Some(host) => format!("{} ({})", connection.name, host),
            None => connection.name.clone(),
        };

        Self {
            id: connection.id,
            name,
        }
    }
}

impl SelectItem for SshConnectionSelectItem {
    type Value = Option<i64>;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

#[derive(Clone, PartialEq)]
struct DriverVariantSelectItem {
    variant: MongoDriverVariant,
    label: String,
}

impl DriverVariantSelectItem {
    fn all() -> Vec<Self> {
        vec![
            Self {
                variant: MongoDriverVariant::Modern,
                label: t!("MongoForm.driver_variant_modern").to_string(),
            },
            Self {
                variant: MongoDriverVariant::Legacy,
                label: t!("MongoForm.driver_variant_legacy").to_string(),
            },
            Self {
                variant: MongoDriverVariant::Legacy32,
                label: t!("MongoForm.driver_variant_legacy_3_2").to_string(),
            },
        ]
    }
}

impl SelectItem for DriverVariantSelectItem {
    type Value = MongoDriverVariant;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.variant
    }
}

/// MongoDB 连接表单窗口
pub struct MongoFormWindow {
    focus_handle: FocusHandle,
    is_editing: bool,
    editing_id: Option<i64>,
    editing_cloud_id: Option<String>,
    editing_last_synced_at: Option<i64>,
    editing_owner_id: Option<String>,

    active_tab: usize,

    name_input: Entity<InputState>,
    host_input: Entity<InputState>,
    port_input: Entity<InputState>,
    database_input: Entity<InputState>,
    username_input: Entity<InputState>,
    password_input: Entity<InputState>,
    authentication_source_input: Entity<InputState>,
    replica_set_input: Entity<InputState>,
    read_preference_input: Entity<InputState>,
    connect_timeout_seconds_input: Entity<InputState>,
    application_name_input: Entity<InputState>,
    driver_variant_select: Entity<SelectState<Vec<DriverVariantSelectItem>>>,

    use_srv_record: bool,
    direct_connection: bool,
    use_tls: bool,
    ssh_connections: Vec<StoredConnection>,
    ssh_connection_select: Entity<SelectState<Vec<SshConnectionSelectItem>>>,
    ssh_tunnel_enabled: bool,
    ssh_host_input: Entity<InputState>,
    ssh_port_input: Entity<InputState>,
    ssh_username_input: Entity<InputState>,
    ssh_auth_type: String,
    ssh_password_input: Entity<InputState>,
    ssh_private_key_path_input: Entity<InputState>,
    ssh_private_key_content_input: Entity<InputState>,
    ssh_private_key_passphrase_input: Entity<InputState>,
    ssh_target_host_input: Entity<InputState>,
    ssh_target_port_input: Entity<InputState>,
    ssh_timeout_input: Entity<InputState>,

    workspace_select: Entity<SelectState<Vec<WorkspaceSelectItem>>>,
    team_select: Entity<SelectState<Vec<TeamSelectItem>>>,
    remark_input: Entity<InputState>,
    sync_enabled: bool,

    is_testing: bool,
    test_result: Option<Result<(), String>>,
    on_saved: Option<MongoFormSavedCallback>,
}

impl MongoFormWindow {
    pub fn new(config: MongoFormWindowConfig, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let is_editing = config.is_editing();
        let connection_to_load = config.connection_to_load().cloned();
        let editing_id = config.editing_connection.as_ref().and_then(|c| c.id);
        let editing_cloud_id = config
            .editing_connection
            .as_ref()
            .and_then(|c| c.cloud_id.clone());
        let editing_last_synced_at = config
            .editing_connection
            .as_ref()
            .and_then(|c| c.last_synced_at);
        let editing_owner_id = config
            .editing_connection
            .as_ref()
            .and_then(|c| c.owner_id.clone());

        let existing_parameters = connection_to_load
            .as_ref()
            .and_then(|connection| connection.to_mongodb_params().ok());

        let name_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("MongoForm.name_placeholder").to_string());
            if let Some(connection) = &connection_to_load {
                state.set_value(connection.name.clone(), window, cx);
            }
            state
        });

        let host_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("MongoForm.host_placeholder").to_string());
            if let Some(parameters) = &existing_parameters {
                if !parameters.host.is_empty() {
                    state.set_value(parameters.host.clone(), window, cx);
                }
            }
            state
        });

        let port_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("MongoForm.port_placeholder").to_string());
            if let Some(parameters) = &existing_parameters {
                if let Some(port_value) = parameters.port {
                    state.set_value(port_value.to_string(), window, cx);
                } else {
                    state.set_value("27017", window, cx);
                }
            } else {
                state.set_value("27017", window, cx);
            }
            state
        });

        let database_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("MongoForm.database_placeholder").to_string());
            if let Some(parameters) = &existing_parameters {
                if let Some(database) = &parameters.database {
                    if !database.is_empty() {
                        state.set_value(database.clone(), window, cx);
                    }
                }
            }
            state
        });

        let username_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("MongoForm.username_placeholder").to_string());
            if let Some(parameters) = &existing_parameters {
                if let Some(username) = &parameters.username {
                    if !username.is_empty() {
                        state.set_value(username.clone(), window, cx);
                    }
                }
            }
            state
        });

        let password_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("MongoForm.password_placeholder").to_string())
                .masked(true);
            if let Some(parameters) = &existing_parameters {
                if let Some(password) = &parameters.password {
                    if !password.is_empty() {
                        state.set_value(password.clone(), window, cx);
                    }
                }
            }
            state
        });

        let authentication_source_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("MongoForm.auth_source_placeholder").to_string());
            if let Some(parameters) = &existing_parameters {
                if let Some(auth_source) = &parameters.auth_source {
                    if !auth_source.is_empty() {
                        state.set_value(auth_source.clone(), window, cx);
                    }
                }
            }
            state
        });

        let replica_set_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("MongoForm.replica_set_placeholder").to_string());
            if let Some(parameters) = &existing_parameters {
                if let Some(replica_set) = &parameters.replica_set {
                    if !replica_set.is_empty() {
                        state.set_value(replica_set.clone(), window, cx);
                    }
                }
            }
            state
        });

        let read_preference_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("MongoForm.read_preference_placeholder").to_string());
            if let Some(parameters) = &existing_parameters {
                if let Some(read_preference) = &parameters.read_preference {
                    if !read_preference.is_empty() {
                        state.set_value(read_preference.clone(), window, cx);
                    }
                }
            }
            state
        });

        let connect_timeout_seconds_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("MongoForm.connect_timeout_placeholder").to_string());
            if let Some(parameters) = &existing_parameters {
                if let Some(timeout_seconds) = parameters.connect_timeout_seconds {
                    state.set_value(timeout_seconds.to_string(), window, cx);
                }
            }
            state
        });

        let application_name_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("MongoForm.app_name_placeholder").to_string());
            if let Some(parameters) = &existing_parameters {
                if let Some(application_name) = &parameters.application_name {
                    if !application_name.is_empty() {
                        state.set_value(application_name.clone(), window, cx);
                    }
                }
            }
            state
        });

        let remark_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("MongoForm.remark_placeholder").to_string());
            if let Some(connection) = &connection_to_load {
                if let Some(remark) = &connection.remark {
                    state.set_value(remark.clone(), window, cx);
                }
            }
            state
        });

        let workspace_items = {
            let mut items = vec![WorkspaceSelectItem::none()];
            items.extend(
                config
                    .workspaces
                    .iter()
                    .map(WorkspaceSelectItem::from_workspace),
            );
            items
        };

        let workspace_select = cx.new(|cx| {
            let mut state = SelectState::new(workspace_items, None, window, cx);
            if let Some(selected) = connection_to_load
                .as_ref()
                .and_then(|connection| connection.workspace_id)
            {
                state.set_selected_value(&Some(selected), window, cx);
            }
            state
        });

        let selected_team_id = connection_to_load
            .as_ref()
            .and_then(|c| c.team_id.as_deref());
        let team_select = create_team_select(&config.teams, selected_team_id, window, cx);

        let sync_enabled = connection_to_load
            .as_ref()
            .map(|connection| connection.sync_enabled)
            .unwrap_or(true);

        let use_srv_record = existing_parameters
            .as_ref()
            .map(|parameters| parameters.use_srv_record)
            .unwrap_or(false);
        let driver_variant = existing_parameters
            .as_ref()
            .map(|parameters| parameters.driver_variant)
            .unwrap_or_default();
        let driver_variant_items = DriverVariantSelectItem::all();
        let driver_variant_index = driver_variant_items
            .iter()
            .position(|item| item.variant == driver_variant)
            .map(IndexPath::new);
        let driver_variant_select =
            cx.new(|cx| SelectState::new(driver_variant_items, driver_variant_index, window, cx));
        let direct_connection = existing_parameters
            .as_ref()
            .map(|parameters| parameters.direct_connection)
            .unwrap_or(false);
        let use_tls = existing_parameters
            .as_ref()
            .map(|parameters| parameters.use_tls)
            .unwrap_or(false);
        let existing_ssh = existing_parameters
            .as_ref()
            .and_then(|p| p.ssh_tunnel.as_ref());
        let mut ssh_items = vec![SshConnectionSelectItem::none()];
        let ssh_connections: Vec<StoredConnection> = config
            .ssh_connections
            .into_iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .collect();
        ssh_items.extend(
            ssh_connections
                .iter()
                .map(SshConnectionSelectItem::from_connection),
        );
        let selected_ssh_connection_id = existing_ssh.and_then(|ssh| ssh.connection_id);
        let ssh_connection_select = cx.new(|cx| {
            let mut state = SelectState::new(ssh_items, Some(Default::default()), window, cx);
            if let Some(id) = selected_ssh_connection_id {
                state.set_selected_value(&Some(id), window, cx);
            }
            state
        });

        let ssh_host_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("jump.example.com");
            if let Some(ssh) = existing_ssh {
                state.set_value(ssh.host.clone(), window, cx);
            }
            state
        });

        let ssh_port_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("22");
            state.set_value(
                existing_ssh.map(|ssh| ssh.port).unwrap_or(22).to_string(),
                window,
                cx,
            );
            state
        });

        let ssh_username_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("root");
            if let Some(ssh) = existing_ssh {
                state.set_value(ssh.username.clone(), window, cx);
            }
            state
        });

        let ssh_password_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("ConnectionForm.enter_password").to_string())
                .masked(true);
            if let Some(ssh) = existing_ssh.and_then(|ssh| ssh.password.as_ref()) {
                state.set_value(ssh.clone(), window, cx);
            }
            state
        });

        let ssh_private_key_path_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("~/.ssh/id_rsa");
            if let Some(path) = existing_ssh.and_then(|ssh| ssh.private_key_path.as_ref()) {
                state.set_value(path.clone(), window, cx);
            }
            state
        });

        let ssh_private_key_content_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("ConnectionForm.ssh_private_key_content_placeholder"))
                .auto_grow(5, 14);
            if let Some(private_key) = existing_ssh.and_then(|ssh| ssh.private_key_content.as_ref())
            {
                state.set_value(private_key.clone(), window, cx);
            }
            state
        });

        let ssh_private_key_passphrase_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("ConnectionForm.enter_passphrase").to_string())
                .masked(true);
            if let Some(passphrase) =
                existing_ssh.and_then(|ssh| ssh.private_key_passphrase.as_ref())
            {
                state.set_value(passphrase.clone(), window, cx);
            }
            state
        });

        let ssh_target_host_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("ConnectionForm.target_host_placeholder").to_string());
            if let Some(host) = existing_ssh.and_then(|ssh| ssh.target_host.as_ref()) {
                state.set_value(host.clone(), window, cx);
            }
            state
        });

        let ssh_target_port_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("27017");
            if let Some(port) = existing_ssh.and_then(|ssh| ssh.target_port) {
                state.set_value(port.to_string(), window, cx);
            }
            state
        });

        let ssh_timeout_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("30");
            if let Some(timeout) = existing_ssh.and_then(|ssh| ssh.timeout) {
                state.set_value(timeout.to_string(), window, cx);
            }
            state
        });

        Self {
            focus_handle: cx.focus_handle(),
            is_editing,
            editing_id,
            editing_cloud_id,
            editing_last_synced_at,
            editing_owner_id,
            active_tab: 0,
            name_input,
            host_input,
            port_input,
            database_input,
            username_input,
            password_input,
            authentication_source_input,
            replica_set_input,
            read_preference_input,
            connect_timeout_seconds_input,
            application_name_input,
            driver_variant_select,
            use_srv_record,
            direct_connection,
            use_tls,
            ssh_connections,
            ssh_connection_select,
            ssh_tunnel_enabled: existing_ssh.is_some_and(|ssh| ssh.enabled),
            ssh_host_input,
            ssh_port_input,
            ssh_username_input,
            ssh_auth_type: existing_ssh
                .map(|ssh| ssh.auth_type.clone())
                .unwrap_or_else(|| "password".to_string()),
            ssh_password_input,
            ssh_private_key_path_input,
            ssh_private_key_content_input,
            ssh_private_key_passphrase_input,
            ssh_target_host_input,
            ssh_target_port_input,
            ssh_timeout_input,
            workspace_select,
            team_select,
            remark_input,
            sync_enabled,
            is_testing: false,
            test_result: None,
            on_saved: config.on_saved,
        }
    }

    fn get_workspace_id(&self, cx: &App) -> Option<i64> {
        self.workspace_select
            .read(cx)
            .selected_value()
            .cloned()
            .flatten()
    }

    fn get_team_id(&self, cx: &App) -> Option<String> {
        selected_team_id(&self.team_select, cx)
    }

    fn request_team_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        refresh_team_options(&self.team_select, window, cx);
    }

    fn selected_ssh_connection(&self, cx: &App) -> Option<&StoredConnection> {
        let selected_id = self
            .ssh_connection_select
            .read(cx)
            .selected_value()
            .cloned()
            .flatten()?;
        self.ssh_connections
            .iter()
            .find(|connection| connection.id == Some(selected_id))
    }

    fn optional_input_value(input: &Entity<InputState>, cx: &App) -> Option<String> {
        let value = input.read(cx).text().to_string().trim().to_string();
        if value.is_empty() { None } else { Some(value) }
    }

    fn optional_input_u16(input: &Entity<InputState>, cx: &App) -> Option<u16> {
        input.read(cx).text().to_string().trim().parse::<u16>().ok()
    }

    fn optional_input_u64(input: &Entity<InputState>, cx: &App) -> Option<u64> {
        input.read(cx).text().to_string().trim().parse::<u64>().ok()
    }

    fn build_ssh_tunnel_config(&self, cx: &App) -> Option<MongoSshTunnelConfig> {
        if !self.ssh_tunnel_enabled {
            return None;
        }

        Some(MongoSshTunnelConfig {
            enabled: true,
            connection_id: self
                .ssh_connection_select
                .read(cx)
                .selected_value()
                .cloned()
                .flatten(),
            host: self
                .ssh_host_input
                .read(cx)
                .text()
                .to_string()
                .trim()
                .to_string(),
            port: Self::optional_input_u16(&self.ssh_port_input, cx).unwrap_or(22),
            username: self
                .ssh_username_input
                .read(cx)
                .text()
                .to_string()
                .trim()
                .to_string(),
            auth_type: self.ssh_auth_type.clone(),
            password: Self::optional_input_value(&self.ssh_password_input, cx),
            private_key_path: Self::optional_input_value(&self.ssh_private_key_path_input, cx),
            private_key_content: Self::optional_input_value(
                &self.ssh_private_key_content_input,
                cx,
            ),
            private_key_passphrase: Self::optional_input_value(
                &self.ssh_private_key_passphrase_input,
                cx,
            ),
            target_host: Self::optional_input_value(&self.ssh_target_host_input, cx),
            target_port: Self::optional_input_u16(&self.ssh_target_port_input, cx),
            timeout: Self::optional_input_u64(&self.ssh_timeout_input, cx),
        })
    }

    fn build_parameters(&self, cx: &App) -> Result<MongoDBParams, String> {
        let host_value = self.host_input.read(cx).text().to_string();
        let host_value = host_value.trim().to_string();
        if host_value.is_empty() {
            return Err(t!("MongoForm.host_required").to_string());
        }

        let port_text = self.port_input.read(cx).text().to_string();
        let port_text = port_text.trim().to_string();
        let port_value = if port_text.is_empty() {
            None
        } else {
            Some(
                port_text
                    .parse::<u16>()
                    .map_err(|_| t!("MongoForm.port_must_be_number").to_string())?,
            )
        };

        let database_value = self.database_input.read(cx).text().to_string();
        let database_value = database_value.trim().to_string();
        let database = if database_value.is_empty() {
            None
        } else {
            Some(database_value)
        };

        let username_value = self.username_input.read(cx).text().to_string();
        let username_value = username_value.trim().to_string();
        let username = if username_value.is_empty() {
            None
        } else {
            Some(username_value)
        };

        let password_value = self.password_input.read(cx).text().to_string();
        let password = if password_value.is_empty() {
            None
        } else {
            Some(password_value)
        };

        let auth_source_value = self.authentication_source_input.read(cx).text().to_string();
        let auth_source_value = auth_source_value.trim().to_string();
        let auth_source = if auth_source_value.is_empty() {
            None
        } else {
            Some(auth_source_value)
        };

        let replica_set_value = self.replica_set_input.read(cx).text().to_string();
        let replica_set_value = replica_set_value.trim().to_string();
        let replica_set = if replica_set_value.is_empty() {
            None
        } else {
            Some(replica_set_value)
        };

        let read_preference_value = self.read_preference_input.read(cx).text().to_string();
        let read_preference_value = read_preference_value.trim().to_string();
        let read_preference = if read_preference_value.is_empty() {
            None
        } else {
            Some(read_preference_value)
        };

        let connect_timeout_text = self
            .connect_timeout_seconds_input
            .read(cx)
            .text()
            .to_string();
        let connect_timeout_text = connect_timeout_text.trim().to_string();
        let connect_timeout_seconds = if connect_timeout_text.is_empty() {
            None
        } else {
            Some(
                connect_timeout_text
                    .parse::<u64>()
                    .map_err(|_| t!("MongoForm.timeout_must_be_number").to_string())?,
            )
        };

        let application_name_value = self.application_name_input.read(cx).text().to_string();
        let application_name_value = application_name_value.trim().to_string();
        let application_name = if application_name_value.is_empty() {
            None
        } else {
            Some(application_name_value)
        };

        let mut params = MongoDBParams {
            driver_variant: self
                .driver_variant_select
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
            connection_string: String::new(),
            host: host_value,
            port: port_value,
            database,
            username,
            password,
            auth_source,
            replica_set,
            read_preference,
            use_srv_record: self.use_srv_record,
            direct_connection: self.direct_connection,
            use_tls: self.use_tls,
            connect_timeout_seconds,
            application_name,
            ssh_tunnel: None,
        };
        params.ssh_tunnel = self.build_ssh_tunnel_config(cx);
        if let Some(ssh_connection) = self.selected_ssh_connection(cx) {
            params
                .apply_referenced_ssh_tunnel(ssh_connection)
                .map_err(|error| error.to_string())?;
        }

        Ok(params)
    }

    fn on_test(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name_input.read(cx).text().to_string();
        let parameters = match self.build_parameters(cx) {
            Ok(parameters) => parameters,
            Err(error) => {
                self.is_testing = false;
                self.test_result = Some(Err(error));
                cx.notify();
                return;
            }
        };

        let test_name = if name.is_empty() {
            t!("MongoForm.default_name").to_string()
        } else {
            name
        };

        self.is_testing = true;
        self.test_result = None;
        cx.notify();
        let global_state = cx.global::<GlobalMongoState>().clone();

        cx.spawn(async move |this, cx| {
            let test_result: Result<(), String> = Tokio::spawn_result(cx, async move {
                global_state
                    .test_parameters(test_name, &parameters)
                    .await
                    .map_err(anyhow::Error::new)
            })
            .await
            .map_err(|error| {
                let detailed = format!("{:#}", error);
                error!("MongoDB 连接测试失败: {}", detailed);
                detailed
            });

            let _ = this.update(cx, |this, cx| {
                this.is_testing = false;
                this.test_result = Some(test_result);
                cx.notify();
            });
        })
        .detach();
    }

    fn on_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parameters = match self.build_parameters(cx) {
            Ok(parameters) => parameters,
            Err(error) => {
                self.test_result = Some(Err(error));
                cx.notify();
                return;
            }
        };
        let name = self.name_input.read(cx).text().to_string();

        let workspace_id = self.get_workspace_id(cx);
        let team_id = self.get_team_id(cx);
        let assignment = match resolve_team_assignment(
            team_id,
            self.is_editing,
            self.editing_owner_id.clone(),
            cx,
        ) {
            Ok(assignment) => assignment,
            Err(error) => {
                self.test_result = Some(Err(error.to_string()));
                cx.notify();
                return;
            }
        };
        let remark = {
            let value = self.remark_input.read(cx).text().to_string();
            if value.is_empty() { None } else { Some(value) }
        };
        let sync_enabled = self.sync_enabled;
        let is_editing = self.is_editing;
        let editing_id = self.editing_id;
        let editing_cloud_id = self.editing_cloud_id.clone();
        let editing_last_synced_at = self.editing_last_synced_at;
        let on_saved = self.on_saved.clone();

        let storage = cx
            .global::<one_core::storage::GlobalStorageState>()
            .storage
            .clone();

        cx.spawn(async move |_this, cx| {
            let result = Tokio::spawn_result(cx, async move {
                let repo = storage
                    .get::<one_core::storage::ConnectionRepository>()
                    .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;

                let mut connection = StoredConnection::new_mongodb(name, parameters, workspace_id);
                connection.sync_enabled = sync_enabled;
                connection.remark = remark;
                connection.team_id = assignment.team_id;
                connection.owner_id = assignment.owner_id;

                if is_editing {
                    connection.id = editing_id;
                    connection.cloud_id = editing_cloud_id;
                    connection.last_synced_at = editing_last_synced_at;
                    repo.update(&mut connection)?;
                } else {
                    repo.insert(&mut connection)?;
                }
                Ok::<StoredConnection, anyhow::Error>(connection)
            })
            .await;

            match result {
                Ok(saved_conn) => {
                    let _ = cx.update(|cx| {
                        if let Some(notifier) = get_notifier(cx) {
                            let event = if is_editing {
                                ConnectionDataEvent::ConnectionUpdated {
                                    connection: saved_conn.clone(),
                                }
                            } else {
                                ConnectionDataEvent::ConnectionCreated {
                                    connection: saved_conn.clone(),
                                }
                            };
                            notifier.update(cx, |_, cx| {
                                cx.emit(event);
                            });
                        }
                        if let Some(on_saved) = &on_saved {
                            on_saved(saved_conn, cx);
                        }
                    });
                }
                Err(error) => {
                    error!(
                        "{}",
                        t!("MongoForm.save_connection_failed", error = error).to_string()
                    );
                }
            }
        })
        .detach();

        window.remove_window();
    }

    fn render_form_row(&self, label: &str, child: impl IntoElement) -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_center()
            .child(
                div()
                    .w(px(120.0))
                    .text_sm()
                    .text_right()
                    .child(label.to_string()),
            )
            .child(div().flex_1().child(child))
    }

    fn render_basic_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(self.render_form_row(
                t!("MongoForm.driver_variant_label").as_ref(),
                Select::new(&self.driver_variant_select).w_full(),
            ))
            .child(self.render_form_row(
                t!("MongoForm.name_label").as_ref(),
                Input::new(&self.name_input),
            ))
            .child(self.render_form_row(
                t!("MongoForm.host_label").as_ref(),
                Input::new(&self.host_input),
            ))
            .child(self.render_form_row(
                t!("MongoForm.port_label").as_ref(),
                Input::new(&self.port_input),
            ))
            .child(self.render_form_row(
                t!("MongoForm.database_label").as_ref(),
                Input::new(&self.database_input),
            ))
            .child(self.render_form_row(
                t!("MongoForm.username_label").as_ref(),
                Input::new(&self.username_input),
            ))
            .child(self.render_form_row(
                t!("MongoForm.password_label").as_ref(),
                Input::new(&self.password_input).mask_toggle(),
            ))
            .child(self.render_form_row(
                t!("MongoForm.auth_source_label").as_ref(),
                Input::new(&self.authentication_source_input),
            ))
            .child(self.render_form_row(
                t!("MongoForm.workspace_label").as_ref(),
                Select::new(&self.workspace_select).w_full(),
            ))
            .when(team_management_enabled(cx), |form| {
                form.child(
                    self.render_form_row(
                        &team_label(),
                        h_flex()
                            .gap_2()
                            .child(Select::new(&self.team_select).w_full())
                            .child(
                                Button::new("sync-mongo-teams")
                                    .icon(IconName::Refresh)
                                    .ghost()
                                    .tooltip(refresh_teams_tooltip())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.request_team_sync(window, cx);
                                    })),
                            ),
                    ),
                )
            })
            .child(
                self.render_form_row(
                    t!("MongoForm.cloud_sync_label").as_ref(),
                    h_flex()
                        .gap_2()
                        .child(
                            Checkbox::new("mongo-sync-enabled")
                                .checked(self.sync_enabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sync_enabled = !this.sync_enabled;
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("MongoForm.cloud_sync_enabled").to_string()),
                        ),
                ),
            )
    }

    fn render_cluster_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(
                self.render_form_row(
                    t!("MongoForm.use_srv").as_ref(),
                    Checkbox::new("mongo-use-srv")
                        .checked(self.use_srv_record)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.use_srv_record = !this.use_srv_record;
                            cx.notify();
                        })),
                ),
            )
            .child(
                self.render_form_row(
                    t!("MongoForm.direct_connection").as_ref(),
                    Checkbox::new("mongo-direct-connection")
                        .checked(self.direct_connection)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.direct_connection = !this.direct_connection;
                            cx.notify();
                        })),
                ),
            )
            .child(self.render_form_row(
                t!("MongoForm.replica_set_label").as_ref(),
                Input::new(&self.replica_set_input),
            ))
            .child(self.render_form_row(
                t!("MongoForm.read_preference_label").as_ref(),
                Input::new(&self.read_preference_input),
            ))
    }

    fn render_advanced_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(
                self.render_form_row(
                    t!("MongoForm.tls_label").as_ref(),
                    Checkbox::new("mongo-use-tls")
                        .checked(self.use_tls)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.use_tls = !this.use_tls;
                            cx.notify();
                        })),
                ),
            )
            .child(self.render_form_row(
                t!("MongoForm.connect_timeout_label").as_ref(),
                Input::new(&self.connect_timeout_seconds_input),
            ))
            .child(self.render_form_row(
                t!("MongoForm.app_name_label").as_ref(),
                Input::new(&self.application_name_input),
            ))
    }

    fn render_ssh_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let using_ssh_reference = self
            .ssh_connection_select
            .read(cx)
            .selected_value()
            .is_some_and(|value| value.is_some());
        let auth_type = match self.ssh_auth_type.as_str() {
            "private_key_material" => "private_key_content",
            value => value,
        };

        v_flex()
            .gap_2()
            .child(
                self.render_form_row(
                    &t!("ConnectionForm.ssh_tunnel_enabled"),
                    Checkbox::new("mongo-ssh-tunnel-enabled")
                        .checked(self.ssh_tunnel_enabled)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ssh_tunnel_enabled = !this.ssh_tunnel_enabled;
                            cx.notify();
                        })),
                ),
            )
            .when(self.ssh_tunnel_enabled, |this| {
                this.child(self.render_form_row(
                    &t!("ConnectionForm.ssh_connection_id"),
                    Select::new(&self.ssh_connection_select).w_full(),
                ))
                .when(!using_ssh_reference, |this| {
                    this.child(self.render_form_row(
                        &t!("ConnectionForm.ssh_host"),
                        Input::new(&self.ssh_host_input),
                    ))
                    .child(self.render_form_row(
                        &t!("ConnectionForm.ssh_port"),
                        Input::new(&self.ssh_port_input),
                    ))
                    .child(self.render_form_row(
                        &t!("ConnectionForm.ssh_username"),
                        Input::new(&self.ssh_username_input),
                    ))
                    .child(
                        self.render_form_row(
                            &t!("ConnectionForm.ssh_auth_type"),
                            h_flex()
                                .flex_wrap()
                                .gap_4()
                                .child(
                                    Radio::new("mongo-ssh-auth-password")
                                        .label(t!("ConnectionForm.ssh_auth_password").to_string())
                                        .checked(auth_type == "password")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.ssh_auth_type = "password".to_string();
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Radio::new("mongo-ssh-auth-private-key")
                                        .label(
                                            t!("ConnectionForm.ssh_auth_private_key").to_string(),
                                        )
                                        .checked(auth_type == "private_key")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.ssh_auth_type = "private_key".to_string();
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Radio::new("mongo-ssh-auth-private-key-content")
                                        .label(
                                            t!("ConnectionForm.ssh_auth_private_key_content")
                                                .to_string(),
                                        )
                                        .checked(auth_type == "private_key_content")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.ssh_auth_type = "private_key_content".to_string();
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Radio::new("mongo-ssh-auth-agent")
                                        .label(t!("ConnectionForm.ssh_auth_agent").to_string())
                                        .checked(auth_type == "agent")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.ssh_auth_type = "agent".to_string();
                                            cx.notify();
                                        })),
                                ),
                        ),
                    )
                    .when(auth_type == "password", |this| {
                        this.child(self.render_form_row(
                            &t!("ConnectionForm.ssh_password"),
                            Input::new(&self.ssh_password_input).mask_toggle(),
                        ))
                    })
                    .when(auth_type == "private_key", |this| {
                        this.child(self.render_form_row(
                            &t!("ConnectionForm.ssh_private_key_path"),
                            Input::new(&self.ssh_private_key_path_input),
                        ))
                        .child(self.render_form_row(
                            &t!("ConnectionForm.ssh_private_key_passphrase"),
                            Input::new(&self.ssh_private_key_passphrase_input).mask_toggle(),
                        ))
                    })
                    .when(auth_type == "private_key_content", |this| {
                        this.child(self.render_form_row(
                            &t!("ConnectionForm.ssh_private_key_content"),
                            Input::new(&self.ssh_private_key_content_input),
                        ))
                        .child(self.render_form_row(
                            &t!("ConnectionForm.ssh_private_key_passphrase"),
                            Input::new(&self.ssh_private_key_passphrase_input).mask_toggle(),
                        ))
                    })
                })
                .child(self.render_form_row(
                    &t!("ConnectionForm.ssh_target_host"),
                    Input::new(&self.ssh_target_host_input),
                ))
                .child(self.render_form_row(
                    &t!("ConnectionForm.ssh_target_port"),
                    Input::new(&self.ssh_target_port_input),
                ))
                .child(self.render_form_row(
                    &t!("MongoForm.ssh_timeout_label"),
                    Input::new(&self.ssh_timeout_input),
                ))
            })
    }

    fn render_remark_tab(&self) -> impl IntoElement {
        v_flex().gap_2().child(self.render_form_row(
            t!("MongoForm.remark_label").as_ref(),
            Input::new(&self.remark_input),
        ))
    }
}

impl Focusable for MongoFormWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mongo_connection(name: &str) -> StoredConnection {
        StoredConnection::new_mongodb(
            name.to_string(),
            MongoDBParams {
                driver_variant: Default::default(),
                connection_string: String::new(),
                host: "127.0.0.1".to_string(),
                port: Some(27017),
                database: Some("app".to_string()),
                username: None,
                password: None,
                auth_source: None,
                replica_set: None,
                read_preference: None,
                use_srv_record: false,
                direct_connection: false,
                use_tls: false,
                connect_timeout_seconds: None,
                application_name: None,
                ssh_tunnel: None,
            },
            None,
        )
    }

    #[test]
    fn initial_connection_prefills_without_edit_mode() {
        let config = MongoFormWindowConfig {
            editing_connection: None,
            initial_connection: Some(mongo_connection("imported mongo")),
            on_saved: None,
            workspaces: Vec::new(),
            teams: Vec::new(),
            ssh_connections: Vec::new(),
        };

        assert!(!config.is_editing());
        assert_eq!(
            Some("imported mongo"),
            config
                .connection_to_load()
                .map(|connection| connection.name.as_str())
        );
    }

    #[test]
    fn editing_connection_takes_precedence_over_initial_connection() {
        let config = MongoFormWindowConfig {
            editing_connection: Some(mongo_connection("existing mongo")),
            initial_connection: Some(mongo_connection("imported mongo")),
            on_saved: None,
            workspaces: Vec::new(),
            teams: Vec::new(),
            ssh_connections: Vec::new(),
        };

        assert!(config.is_editing());
        assert_eq!(
            Some("existing mongo"),
            config
                .connection_to_load()
                .map(|connection| connection.name.as_str())
        );
    }
}

impl Render for MongoFormWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_testing = self.is_testing;
        let active_tab = self.active_tab;

        let test_result_element = match &self.test_result {
            Some(Ok(())) => Some(
                div()
                    .w_full()
                    .px_6()
                    .pb_2()
                    .text_sm()
                    .text_color(cx.theme().success)
                    .child(t!("MongoForm.test_success").to_string()),
            ),
            Some(Err(error)) => Some(
                div().w_full().px_6().pb_2().child(
                    div()
                        .w_full()
                        .max_h(px(120.0))
                        .overflow_y_scrollbar()
                        .whitespace_normal()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error.clone()),
                ),
            ),
            None => None,
        };

        v_flex()
            .justify_center()
            .size_full()
            .child(
                div().flex().justify_center().px_3().pt_2().child(
                    TabBar::new("mongodb-form-tabs")
                        .with_size(Size::Small)
                        .underline()
                        .selected_index(active_tab)
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.active_tab = *ix;
                            cx.notify();
                        }))
                        .child(Tab::new().label(t!("MongoForm.tab_basic").to_string()))
                        .child(Tab::new().label(t!("MongoForm.tab_cluster").to_string()))
                        .child(Tab::new().label(t!("MongoForm.tab_advanced").to_string()))
                        .child(Tab::new().label(t!("MongoForm.tab_ssh").to_string()))
                        .child(Tab::new().label(t!("MongoForm.tab_remark").to_string())),
                ),
            )
            .child(
                div()
                    .id("mongo-form-content")
                    .flex_1()
                    .p_4()
                    .overflow_y_scroll()
                    .child(match active_tab {
                        0 => self.render_basic_tab(cx).into_any_element(),
                        1 => self.render_cluster_tab(cx).into_any_element(),
                        2 => self.render_advanced_tab(cx).into_any_element(),
                        3 => self.render_ssh_tab(cx).into_any_element(),
                        4 => self.render_remark_tab().into_any_element(),
                        _ => div().into_any_element(),
                    }),
            )
            .when_some(test_result_element, |this, elem| this.child(elem))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .px_6()
                    .py_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("cancel")
                            .small()
                            .label(t!("Common.cancel").to_string())
                            .on_click(|_, window, _cx| {
                                window.remove_window();
                            }),
                    )
                    .child(
                        Button::new("test")
                            .small()
                            .outline()
                            .icon(IconName::Refresh)
                            .label(if is_testing {
                                t!("MongoForm.testing").to_string()
                            } else {
                                t!("MongoForm.test").to_string()
                            })
                            .disabled(is_testing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_test(window, cx);
                            })),
                    )
                    .child(
                        Button::new("ok")
                            .small()
                            .primary()
                            .label(t!("Common.save").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_save(window, cx);
                            })),
                    ),
            )
    }
}
