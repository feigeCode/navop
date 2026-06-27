use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::cloud_sync::models::{CloudSyncData, data_type};
use crate::cloud_sync::service::{CloudSyncService, SyncError};
use crate::storage::traits::Repository;
use crate::storage::{ConnectionRepository, StoredConnection, Workspace, WorkspaceRepository};

use super::{PersonalSyncItemSnapshot, PersonalSyncLocalSource, SyncStoreError};

const CONNECTION_PREFIX: &str = "connection:";
const WORKSPACE_PREFIX: &str = "workspace:";

#[derive(Clone)]
pub struct PersonalSyncLocalRepositorySource {
    connections: ConnectionRepository,
    workspaces: WorkspaceRepository,
    service: Arc<RwLock<CloudSyncService>>,
}

impl PersonalSyncLocalRepositorySource {
    pub fn new(
        connections: ConnectionRepository,
        workspaces: WorkspaceRepository,
        service: Arc<RwLock<CloudSyncService>>,
    ) -> Self {
        Self {
            connections,
            workspaces,
            service,
        }
    }

    fn connection_snapshot(
        &self,
        conn: &StoredConnection,
    ) -> Result<PersonalSyncItemSnapshot, SyncStoreError> {
        let id = required_id(conn.id, "connection")?;
        let record = self.export_connection(conn)?;
        Ok(PersonalSyncItemSnapshot {
            local_id: format!("{CONNECTION_PREFIX}{id}"),
            cloud_id: conn.cloud_id.clone(),
            data_type: data_type::CONNECTION.to_string(),
            updated_at: conn.updated_at.unwrap_or(0),
            last_synced_at: conn.last_synced_at,
            checksum: record.checksum,
            team_id: conn.team_id.clone(),
        })
    }

    fn workspace_snapshot(
        &self,
        workspace: &Workspace,
    ) -> Result<PersonalSyncItemSnapshot, SyncStoreError> {
        let id = required_id(workspace.id, "workspace")?;
        let record = self.export_workspace(workspace)?;
        Ok(PersonalSyncItemSnapshot {
            local_id: format!("{WORKSPACE_PREFIX}{id}"),
            cloud_id: workspace.cloud_id.clone(),
            data_type: data_type::WORKSPACE.to_string(),
            updated_at: workspace.updated_at.unwrap_or(0),
            last_synced_at: None,
            checksum: record.checksum,
            team_id: None,
        })
    }

    fn export_connection(&self, conn: &StoredConnection) -> Result<CloudSyncData, SyncStoreError> {
        let workspace_cloud_id = self.workspace_cloud_id(conn.workspace_id)?;
        let mut record = self
            .service
            .read()
            .map_err(lock_error)?
            .prepare_sync_data_upload_with_workspace_cloud_id(conn, None, &[], workspace_cloud_id)
            .map_err(sync_error)?;
        preserve_cloud_id(&mut record, conn.cloud_id.as_deref());
        Ok(record)
    }

    fn export_workspace(&self, workspace: &Workspace) -> Result<CloudSyncData, SyncStoreError> {
        let mut record = self
            .service
            .read()
            .map_err(lock_error)?
            .prepare_workspace_sync_data_upload(workspace, None, &[])
            .map_err(sync_error)?;
        preserve_cloud_id(&mut record, workspace.cloud_id.as_deref());
        Ok(record)
    }

    fn workspace_cloud_id(
        &self,
        workspace_id: Option<i64>,
    ) -> Result<Option<String>, SyncStoreError> {
        let Some(id) = workspace_id else {
            return Ok(None);
        };
        Ok(self
            .workspaces
            .get(id)
            .map_err(repository_error)?
            .and_then(|workspace| workspace.cloud_id))
    }

    fn workspace_id_by_cloud_id(
        &self,
        workspace_cloud_id: Option<&str>,
    ) -> Result<Option<i64>, SyncStoreError> {
        let Some(cloud_id) = workspace_cloud_id else {
            return Ok(None);
        };
        Ok(self
            .workspaces
            .list()
            .map_err(repository_error)?
            .into_iter()
            .find(|workspace| workspace.cloud_id.as_deref() == Some(cloud_id))
            .and_then(|workspace| workspace.id))
    }

    fn apply_connection(
        &self,
        record: &CloudSyncData,
        local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError> {
        let (mut conn, workspace_cloud_id) = self
            .service
            .read()
            .map_err(lock_error)?
            .decrypt_sync_data_connection_with_workspace_cloud_id(record)
            .map_err(sync_error)?;
        conn.id = local.and_then(|item| parse_prefixed_id(&item.local_id, CONNECTION_PREFIX).ok());
        conn.workspace_id = self.workspace_id_by_cloud_id(workspace_cloud_id.as_deref())?;
        conn.cloud_id = Some(record.id.clone());
        conn.last_synced_at = Some(record.updated_at / 1000);
        conn.updated_at = Some(record.updated_at / 1000);

        match conn.id {
            Some(id) => {
                self.connections.update(&conn).map_err(repository_error)?;
                self.connections
                    .update_sync_status_with_updated_at(
                        id,
                        Some(record.id.clone()),
                        conn.last_synced_at,
                        record.updated_at / 1000,
                    )
                    .map_err(repository_error)
            }
            None => self
                .connections
                .insert(&mut conn)
                .map(|_| ())
                .map_err(repository_error),
        }
    }

    fn apply_workspace(
        &self,
        record: &CloudSyncData,
        local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError> {
        let mut workspace = self
            .service
            .read()
            .map_err(lock_error)?
            .decrypt_sync_data_workspace(record)
            .map_err(sync_error)?;
        workspace.id =
            local.and_then(|item| parse_prefixed_id(&item.local_id, WORKSPACE_PREFIX).ok());
        workspace.cloud_id = Some(record.id.clone());

        if workspace.id.is_some() {
            self.workspaces
                .update_from_cloud(&workspace)
                .map_err(repository_error)
        } else {
            self.workspaces
                .insert(&mut workspace)
                .map(|_| ())
                .map_err(repository_error)
        }
    }
}

#[async_trait]
impl PersonalSyncLocalSource for PersonalSyncLocalRepositorySource {
    async fn list_items(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError> {
        let mut items = Vec::new();
        for conn in self.connections.list_personal().map_err(repository_error)? {
            if conn.sync_enabled {
                items.push(self.connection_snapshot(&conn)?);
            }
        }
        for workspace in self.workspaces.list().map_err(repository_error)? {
            items.push(self.workspace_snapshot(&workspace)?);
        }
        Ok(items)
    }

    async fn export_item(
        &self,
        item: &PersonalSyncItemSnapshot,
    ) -> Result<CloudSyncData, SyncStoreError> {
        if let Ok(id) = parse_prefixed_id(&item.local_id, CONNECTION_PREFIX) {
            return self.export_connection(&load_required_connection(&self.connections, id)?);
        }
        let id = parse_prefixed_id(&item.local_id, WORKSPACE_PREFIX)?;
        self.export_workspace(&load_required_workspace(&self.workspaces, id)?)
    }

    async fn apply_remote(
        &self,
        record: &CloudSyncData,
        local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError> {
        if record.team_id.is_some() {
            return Ok(());
        }
        match record.data_type.as_str() {
            data_type::CONNECTION => self.apply_connection(record, local),
            data_type::WORKSPACE => self.apply_workspace(record, local),
            other => Err(SyncStoreError::Parse(format!(
                "unsupported personal sync data type: {other}"
            ))),
        }
    }

    async fn mark_synced(
        &self,
        local_id: &str,
        cloud_id: &str,
        synced_at: i64,
    ) -> Result<(), SyncStoreError> {
        if let Ok(id) = parse_prefixed_id(local_id, CONNECTION_PREFIX) {
            return self
                .connections
                .update_sync_status(id, Some(cloud_id.to_string()), Some(synced_at))
                .map_err(repository_error);
        }
        let id = parse_prefixed_id(local_id, WORKSPACE_PREFIX)?;
        self.workspaces
            .update_cloud_id(id, Some(cloud_id.to_string()))
            .map_err(repository_error)
    }
}

fn preserve_cloud_id(record: &mut CloudSyncData, cloud_id: Option<&str>) {
    if let Some(cloud_id) = cloud_id {
        record.id = cloud_id.to_string();
    }
}

fn parse_prefixed_id(local_id: &str, prefix: &str) -> Result<i64, SyncStoreError> {
    local_id
        .strip_prefix(prefix)
        .ok_or_else(|| SyncStoreError::Parse(format!("invalid local id: {local_id}")))?
        .parse::<i64>()
        .map_err(|error| SyncStoreError::Parse(error.to_string()))
}

fn load_required_connection(
    repository: &ConnectionRepository,
    id: i64,
) -> Result<StoredConnection, SyncStoreError> {
    repository
        .get(id)
        .map_err(repository_error)?
        .ok_or_else(|| SyncStoreError::Parse(format!("connection not found: {id}")))
}

fn load_required_workspace(
    repository: &WorkspaceRepository,
    id: i64,
) -> Result<Workspace, SyncStoreError> {
    repository
        .get(id)
        .map_err(repository_error)?
        .ok_or_else(|| SyncStoreError::Parse(format!("workspace not found: {id}")))
}

fn required_id(id: Option<i64>, entity: &str) -> Result<i64, SyncStoreError> {
    id.ok_or_else(|| SyncStoreError::Parse(format!("{entity} has no local id")))
}

fn repository_error(error: anyhow::Error) -> SyncStoreError {
    SyncStoreError::Io(error.to_string())
}

fn sync_error(error: SyncError) -> SyncStoreError {
    SyncStoreError::Parse(error.to_string())
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> SyncStoreError {
    SyncStoreError::Io(error.to_string())
}
