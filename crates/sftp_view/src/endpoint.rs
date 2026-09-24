#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LeftEndpointKind {
    Local,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaneSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DragSource {
    LocalLeft,
    RemoteLeft,
    RemoteRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransferRoute {
    Upload,
    Download,
    ServerToServer { source: PaneSide, target: PaneSide },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LeftEndpointValue {
    Local,
    Remote(i64),
}

#[derive(Clone)]
pub(crate) struct LeftEndpointItem {
    value: LeftEndpointValue,
    title: String,
    subtitle: Option<String>,
    icon: IconName,
}

impl LeftEndpointItem {
    fn local(title: String) -> Self {
        Self {
            value: LeftEndpointValue::Local,
            title,
            subtitle: None,
            icon: IconName::HardDrive,
        }
    }

    fn remote(connection: &StoredConnection) -> Option<Self> {
        let id = connection.id?;
        Some(Self {
            value: LeftEndpointValue::Remote(id),
            // 列表里不重复拼主机名，主机走行尾的副标题。
            title: connection.name.clone(),
            subtitle: connection_endpoint(connection),
            icon: connection.connection_type.icon(),
        })
    }

    pub(crate) fn value(&self) -> &LeftEndpointValue {
        &self.value
    }

    pub(crate) fn title_text(&self) -> &str {
        &self.title
    }

    /// 副标题与终端文件面板的目标选择器一致：`user@host:port`。
    pub(crate) fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    pub(crate) fn icon(&self) -> IconName {
        self.icon.clone()
    }
}

/// `user@host:port`；连接记录不完整（参数取不到）时返回 `None`。
fn connection_endpoint(connection: &StoredConnection) -> Option<String> {
    let endpoint = connection_endpoint_label(connection);
    (!endpoint.is_empty()).then_some(endpoint)
}

impl SelectItem for LeftEndpointItem {
    type Value = LeftEndpointValue;

    fn title(&self) -> SharedString {
        self.title.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

pub(crate) fn endpoint_items(
    current_connection: &StoredConnection,
    local_title: String,
    cx: &App,
) -> Vec<LeftEndpointItem> {
    let mut items = vec![LeftEndpointItem::local(local_title)];
    items.extend(
        ssh_connections(cx)
            .into_iter()
            .filter(|candidate| !same_connection(candidate, current_connection))
            .filter_map(|connection| LeftEndpointItem::remote(&connection)),
    );
    items
}

pub(crate) fn connection_title(connection: &StoredConnection) -> String {
    let host = connection
        .to_ssh_params()
        .ok()
        .map(|params| params.host)
        .filter(|host| !host.trim().is_empty())
        .or_else(|| {
            // 独立 FTP 连接没有 SSH 参数，标题回退到 FTP 主机。
            connection
                .to_ftp_params()
                .ok()
                .map(|params| params.host)
                .filter(|host| !host.trim().is_empty())
        });
    host.map_or_else(
        || connection.name.clone(),
        |host| format!("{} ({host})", connection.name),
    )
}

pub(crate) fn load_connection(id: i64, cx: &App) -> Option<StoredConnection> {
    let storage = cx.try_global::<GlobalStorageState>()?;
    let repository = storage.storage.get::<ConnectionRepository>()?;
    let connection = repository.get(id).ok().flatten()?;
    Some(with_runtime_credentials(&repository, connection))
}

/// 解析左侧端点建连所需的运行时凭据。
///
/// 记住的密码只存在于凭据库里，直接用存储记录会拿空密码建连并连接失败；
/// 凭据库拿不到（例如本机缺少该钥匙串）时退化为「本次连接输入」，
/// 由切换流程弹窗收集。
fn with_runtime_credentials(
    repository: &ConnectionRepository,
    connection: StoredConnection,
) -> StoredConnection {
    match repository.resolve_runtime_connection(&connection) {
        Ok(resolved) => resolved,
        Err(error) => {
            tracing::warn!(
                connection_id = ?connection.id,
                error = %error,
                "解析左侧端点凭据失败，改为本次连接输入"
            );
            prompt_for_credentials(connection)
        }
    }
}

fn prompt_for_credentials(mut connection: StoredConnection) -> StoredConnection {
    if connection.connection_type != ConnectionType::SshSftp {
        return connection;
    }
    let Ok(mut params) = connection.to_ssh_params() else {
        return connection;
    };
    params.credential_reference = None;
    params.username.clear();
    params.auth_method = SshAuthMethod::Password {
        password: String::new(),
    };
    params.prompt_username = Some(true);
    params.prompt_password = Some(true);
    match serde_json::to_string(&params) {
        Ok(params) => connection.params = params,
        Err(error) => tracing::warn!(error = %error, "构造待录入凭据的连接参数失败"),
    }
    connection
}

fn ssh_connections(cx: &App) -> Vec<StoredConnection> {
    let Some(storage) = cx.try_global::<GlobalStorageState>() else {
        return Vec::new();
    };
    let Some(repository) = storage.storage.get::<ConnectionRepository>() else {
        return Vec::new();
    };
    repository
        .list()
        .unwrap_or_default()
        .into_iter()
        .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
        .collect()
}

fn same_connection(left: &StoredConnection, right: &StoredConnection) -> bool {
    match (left.id, right.id) {
        (Some(left_id), Some(right_id)) => left_id == right_id,
        _ => {
            left.name == right.name
                && left.connection_type == right.connection_type
                && left.params == right.params
        }
    }
}

pub(crate) fn transfer_route(
    left_endpoint: LeftEndpointKind,
    source: DragSource,
    target: PaneSide,
) -> Option<TransferRoute> {
    match (left_endpoint, source, target) {
        (_, DragSource::LocalLeft | DragSource::RemoteLeft, PaneSide::Left)
        | (_, DragSource::RemoteRight, PaneSide::Right) => None,
        (LeftEndpointKind::Local, DragSource::LocalLeft, PaneSide::Right) => {
            Some(TransferRoute::Upload)
        }
        (LeftEndpointKind::Local, DragSource::RemoteRight, PaneSide::Left) => {
            Some(TransferRoute::Download)
        }
        (LeftEndpointKind::Remote, DragSource::RemoteLeft, PaneSide::Right) => {
            Some(TransferRoute::ServerToServer {
                source: PaneSide::Left,
                target: PaneSide::Right,
            })
        }
        (LeftEndpointKind::Remote, DragSource::RemoteRight, PaneSide::Left) => {
            Some(TransferRoute::ServerToServer {
                source: PaneSide::Right,
                target: PaneSide::Left,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DragSource, LeftEndpointKind, PaneSide, TransferRoute, same_connection, transfer_route,
    };
    use one_core::storage::{SshAuthMethod, SshParams, StoredConnection};

    fn connection(id: i64, name: &str) -> StoredConnection {
        let mut connection = StoredConnection::new_ssh(
            name.to_string(),
            SshParams {
                remote_file: None,
                sftp_default_directory: None,
                disabled_jump_server: None,
                sftp_account: None,
                host: format!("{name}.internal"),
                port: 22,
                username: "deploy".to_string(),
                auth_method: SshAuthMethod::Agent,
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        );
        connection.id = Some(id);
        connection
    }

    #[test]
    fn local_left_to_right_uses_upload() {
        assert_eq!(
            Some(TransferRoute::Upload),
            transfer_route(
                LeftEndpointKind::Local,
                DragSource::LocalLeft,
                PaneSide::Right,
            )
        );
    }

    #[test]
    fn right_to_local_left_uses_download() {
        assert_eq!(
            Some(TransferRoute::Download),
            transfer_route(
                LeftEndpointKind::Local,
                DragSource::RemoteRight,
                PaneSide::Left,
            )
        );
    }

    #[test]
    fn remote_left_to_right_uses_server_copy() {
        assert_eq!(
            Some(TransferRoute::ServerToServer {
                source: PaneSide::Left,
                target: PaneSide::Right,
            }),
            transfer_route(
                LeftEndpointKind::Remote,
                DragSource::RemoteLeft,
                PaneSide::Right,
            )
        );
    }

    #[test]
    fn right_to_remote_left_uses_server_copy() {
        assert_eq!(
            Some(TransferRoute::ServerToServer {
                source: PaneSide::Right,
                target: PaneSide::Left,
            }),
            transfer_route(
                LeftEndpointKind::Remote,
                DragSource::RemoteRight,
                PaneSide::Left,
            )
        );
    }

    #[test]
    fn drops_back_onto_the_source_pane_are_ignored() {
        assert_eq!(
            None,
            transfer_route(
                LeftEndpointKind::Remote,
                DragSource::RemoteLeft,
                PaneSide::Left,
            )
        );
        assert_eq!(
            None,
            transfer_route(
                LeftEndpointKind::Remote,
                DragSource::RemoteRight,
                PaneSide::Right,
            )
        );
    }

    #[test]
    fn current_server_is_excluded_by_stable_id() {
        assert!(same_connection(
            &connection(7, "source"),
            &connection(7, "renamed")
        ));
        assert!(!same_connection(
            &connection(7, "source"),
            &connection(8, "source")
        ));
    }
}
use gpui::{App, SharedString};
use gpui_component::select::SelectItem;
use one_assets::IconName;
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, SshAuthMethod, StoredConnection,
    traits::Repository,
};
use sftp_transfer::connection_endpoint_label;
