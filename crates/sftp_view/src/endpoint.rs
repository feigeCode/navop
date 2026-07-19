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
    icon: IconName,
}

impl LeftEndpointItem {
    fn local(title: String) -> Self {
        Self {
            value: LeftEndpointValue::Local,
            title,
            icon: IconName::HardDrive,
        }
    }

    fn remote(connection: &StoredConnection) -> Option<Self> {
        let id = connection.id?;
        Some(Self {
            value: LeftEndpointValue::Remote(id),
            title: connection_title(connection),
            icon: connection.connection_type.icon(),
        })
    }

    pub(crate) fn value(&self) -> &LeftEndpointValue {
        &self.value
    }

    pub(crate) fn title_text(&self) -> &str {
        &self.title
    }

    pub(crate) fn icon(&self) -> IconName {
        self.icon.clone()
    }
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
        .filter(|host| !host.trim().is_empty());
    host.map_or_else(
        || connection.name.clone(),
        |host| format!("{} ({host})", connection.name),
    )
}

pub(crate) fn load_connection(id: i64, cx: &App) -> Option<StoredConnection> {
    let storage = cx.try_global::<GlobalStorageState>()?;
    let repository = storage.storage.get::<ConnectionRepository>()?;
    repository.get(id).ok().flatten()
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
                host: format!("{name}.internal"),
                port: 22,
                username: "deploy".to_string(),
                auth_method: SshAuthMethod::Agent,
                connect_timeout: None,
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                jump_server: None,
                proxy: None,
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
use gpui_component::{IconName, select::SelectItem};
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection, traits::Repository,
};
