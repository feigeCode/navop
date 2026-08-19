use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::cloud_sync::models::{CloudSyncData, data_type};
use crate::cloud_sync::service::{CloudSyncService, SyncError};
use crate::storage::traits::Repository;
use crate::storage::{
    ConnectionRepository, CredentialEntry, CredentialRepository, DeleteCredentialOutcome,
    StoredConnection, Workspace, WorkspaceRepository,
};

use super::{PersonalSyncItemSnapshot, PersonalSyncLocalSource, SyncStoreError};

const CONNECTION_PREFIX: &str = "connection:";
const CREDENTIAL_PREFIX: &str = "credential:";
const WORKSPACE_PREFIX: &str = "workspace:";

#[derive(Clone)]
pub struct PersonalSyncLocalRepositorySource {
    connections: ConnectionRepository,
    credentials: CredentialRepository,
    workspaces: WorkspaceRepository,
    service: Arc<RwLock<CloudSyncService>>,
}

impl PersonalSyncLocalRepositorySource {
    pub fn new(
        connections: ConnectionRepository,
        credentials: CredentialRepository,
        workspaces: WorkspaceRepository,
        service: Arc<RwLock<CloudSyncService>>,
    ) -> Self {
        Self {
            connections,
            credentials,
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
            last_synced_at: workspace.last_synced_at,
            checksum: record.checksum,
            team_id: None,
        })
    }

    fn credential_snapshot(
        &self,
        credential: &CredentialEntry,
    ) -> Result<PersonalSyncItemSnapshot, SyncStoreError> {
        let id = required_id(credential.id, "credential")?;
        let record = self.export_credential(credential)?;
        Ok(PersonalSyncItemSnapshot {
            local_id: format!("{CREDENTIAL_PREFIX}{id}"),
            cloud_id: credential.cloud_id.clone(),
            data_type: data_type::CREDENTIAL.to_string(),
            updated_at: credential.updated_at.unwrap_or(0),
            last_synced_at: credential.last_synced_at,
            checksum: record.checksum,
            team_id: credential.team_id.clone(),
        })
    }

    fn export_connection(&self, conn: &StoredConnection) -> Result<CloudSyncData, SyncStoreError> {
        let workspace_cloud_id = self.workspace_cloud_id(conn.workspace_id)?;
        let mut sync_connection = conn.clone();
        sync_connection.params = self.connection_params_with_stable_credential_references(conn)?;
        let mut record = self
            .service
            .read()
            .map_err(lock_error)?
            .prepare_sync_data_upload_with_workspace_cloud_id(
                &sync_connection,
                None,
                &[],
                workspace_cloud_id,
            )
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

    fn export_credential(
        &self,
        credential: &CredentialEntry,
    ) -> Result<CloudSyncData, SyncStoreError> {
        let mut record = self
            .service
            .read()
            .map_err(lock_error)?
            .prepare_credential_sync_data_upload(credential)
            .map_err(sync_error)?;
        preserve_cloud_id(&mut record, credential.cloud_id.as_deref());
        Ok(record)
    }

    fn connection_params_with_stable_credential_references(
        &self,
        conn: &StoredConnection,
    ) -> Result<String, SyncStoreError> {
        let mut params = serde_json::from_str::<serde_json::Value>(&conn.params)
            .map_err(|error| SyncStoreError::Parse(error.to_string()))?;
        enrich_credential_references(&mut params, &self.credentials)?;
        serde_json::to_string(&params).map_err(|error| SyncStoreError::Parse(error.to_string()))
    }

    fn connection_params_with_local_credential_references(
        &self,
        params: &str,
    ) -> Result<String, SyncStoreError> {
        let mut params = serde_json::from_str::<serde_json::Value>(params)
            .map_err(|error| SyncStoreError::Parse(error.to_string()))?;
        resolve_credential_references(&mut params, &self.credentials)?;
        serde_json::to_string(&params).map_err(|error| SyncStoreError::Parse(error.to_string()))
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
        if local.is_none()
            && self
                .connections
                .get_by_cloud_id(&record.id)
                .map_err(repository_error)?
                .is_some_and(|connection| !connection.sync_enabled)
        {
            // 关闭同步的本地连接仍保留 cloud_id。不要因为远端历史记录
            // 不在本地快照中而重复下载或重新启用它。
            return Ok(());
        }

        let (mut conn, workspace_cloud_id) = self
            .service
            .read()
            .map_err(lock_error)?
            .decrypt_sync_data_connection_with_workspace_cloud_id(record)
            .map_err(sync_error)?;
        conn.params = self.connection_params_with_local_credential_references(&conn.params)?;
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
        workspace.last_synced_at = Some(record.updated_at / 1000);
        workspace.updated_at = Some(record.updated_at / 1000);

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

    fn apply_credential(
        &self,
        record: &CloudSyncData,
        local: Option<&PersonalSyncItemSnapshot>,
    ) -> Result<(), SyncStoreError> {
        if local.is_none()
            && self
                .credentials
                .get_by_cloud_id(&record.id)
                .map_err(repository_error)?
                .is_some_and(|credential| !credential.sync_enabled)
        {
            // 关闭同步的本地条目仍保留 cloud_id。不要因为远端历史密文
            // 不在本地快照中而重复下载或重新启用它。
            return Ok(());
        }

        let mut credential = self
            .service
            .read()
            .map_err(lock_error)?
            .decrypt_sync_data_credential(record)
            .map_err(sync_error)?;
        let local_id =
            local.and_then(|item| parse_prefixed_id(&item.local_id, CREDENTIAL_PREFIX).ok());
        if let Some(id) = local_id {
            let existing = load_required_credential(&self.credentials, id)?;
            credential.id = Some(id);
            credential.private_key_path = existing.private_key_path;
            credential.created_at = existing.created_at;
            self.credentials
                .update(&credential)
                .map_err(repository_error)?;
            self.credentials
                .update_sync_status_with_updated_at(
                    id,
                    Some(&record.id),
                    Some(record.updated_at / 1000),
                    record.updated_at / 1000,
                )
                .map_err(repository_error)
        } else {
            let id = self
                .credentials
                .insert(&mut credential)
                .map_err(repository_error)?;
            self.credentials
                .update_sync_status_with_updated_at(
                    id,
                    Some(&record.id),
                    Some(record.updated_at / 1000),
                    record.updated_at / 1000,
                )
                .map_err(repository_error)
        }
    }
}

#[async_trait]
impl PersonalSyncLocalSource for PersonalSyncLocalRepositorySource {
    async fn list_items(&self) -> Result<Vec<PersonalSyncItemSnapshot>, SyncStoreError> {
        let mut items = Vec::new();
        // 钥匙串必须先上传并取得 cloud_id，随后导出的连接才能写入稳定引用。
        for credential in self.credentials.list().map_err(repository_error)? {
            if credential.sync_enabled && credential.team_id.is_none() {
                items.push(self.credential_snapshot(&credential)?);
            }
        }
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
        if let Ok(id) = parse_prefixed_id(&item.local_id, CREDENTIAL_PREFIX) {
            return self.export_credential(&load_required_credential(&self.credentials, id)?);
        }
        if let Ok(id) = parse_prefixed_id(&item.local_id, WORKSPACE_PREFIX) {
            return self.export_workspace(&load_required_workspace(&self.workspaces, id)?);
        }
        Err(SyncStoreError::Parse(format!(
            "unsupported personal sync local id: {}",
            item.local_id
        )))
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
            data_type::CREDENTIAL => self.apply_credential(record, local),
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
        if let Ok(id) = parse_prefixed_id(local_id, CREDENTIAL_PREFIX) {
            return self
                .credentials
                .update_sync_status(id, Some(cloud_id), Some(synced_at))
                .map_err(repository_error);
        }
        if let Ok(id) = parse_prefixed_id(local_id, WORKSPACE_PREFIX) {
            return self
                .workspaces
                .update_sync_status(id, Some(cloud_id.to_string()), Some(synced_at))
                .map_err(repository_error);
        }
        Err(SyncStoreError::Parse(format!(
            "unsupported personal sync local id: {local_id}"
        )))
    }

    async fn delete_item(&self, item: &PersonalSyncItemSnapshot) -> Result<(), SyncStoreError> {
        if let Ok(id) = parse_prefixed_id(&item.local_id, CONNECTION_PREFIX) {
            return self.connections.delete(id).map_err(repository_error);
        }
        if let Ok(id) = parse_prefixed_id(&item.local_id, CREDENTIAL_PREFIX) {
            return match self
                .credentials
                .delete_checked(id)
                .map_err(repository_error)?
            {
                DeleteCredentialOutcome::Deleted | DeleteCredentialOutcome::NotFound => Ok(()),
                DeleteCredentialOutcome::Referenced(hits) => {
                    Err(SyncStoreError::Conflict(format!(
                        "credential {id} is still referenced by {} connection(s)",
                        hits.len()
                    )))
                }
            };
        }
        if let Ok(id) = parse_prefixed_id(&item.local_id, WORKSPACE_PREFIX) {
            return self.workspaces.delete(id).map_err(repository_error);
        }
        Err(SyncStoreError::Parse(format!(
            "unsupported personal sync local id: {}",
            item.local_id
        )))
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

fn load_required_credential(
    repository: &CredentialRepository,
    id: i64,
) -> Result<CredentialEntry, SyncStoreError> {
    repository
        .get(id)
        .map_err(repository_error)?
        .ok_or_else(|| SyncStoreError::Parse(format!("credential not found: {id}")))
}

fn enrich_credential_references(
    value: &mut serde_json::Value,
    repository: &CredentialRepository,
) -> Result<(), SyncStoreError> {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                enrich_credential_references(value, repository)?;
            }
        }
        serde_json::Value::Object(object) => {
            let local_id = object
                .get("credential_id")
                .and_then(serde_json::Value::as_i64);
            if let Some(local_id) = local_id
                && local_id > 0
            {
                let cloud_id = repository
                    .get_summary(local_id)
                    .map_err(repository_error)?
                    .and_then(|summary| summary.cloud_id);
                if let Some(cloud_id) = cloud_id {
                    object.insert(
                        "credential_cloud_id".to_string(),
                        serde_json::Value::String(cloud_id),
                    );
                } else {
                    // 不上传仅在源设备有效的整数 ID，避免目标设备误绑定同号条目。
                    object.insert(
                        "credential_id".to_string(),
                        serde_json::Value::Number(0.into()),
                    );
                    object.insert("credential_cloud_id".to_string(), serde_json::Value::Null);
                }
            }
            for nested in object.values_mut() {
                enrich_credential_references(nested, repository)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn resolve_credential_references(
    value: &mut serde_json::Value,
    repository: &CredentialRepository,
) -> Result<(), SyncStoreError> {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                resolve_credential_references(value, repository)?;
            }
        }
        serde_json::Value::Object(object) => {
            resolve_object_credential_reference(object, repository)?;
            for nested in object.values_mut() {
                resolve_credential_references(nested, repository)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn resolve_object_credential_reference(
    object: &mut serde_json::Map<String, serde_json::Value>,
    repository: &CredentialRepository,
) -> Result<(), SyncStoreError> {
    let cloud_id = object
        .get("credential_cloud_id")
        .and_then(serde_json::Value::as_str)
        .filter(|cloud_id| !cloud_id.is_empty());
    let Some(cloud_id) = cloud_id else {
        return Ok(());
    };
    let local_id = repository
        .get_by_cloud_id(cloud_id)
        .map_err(repository_error)?
        .and_then(|credential| credential.id)
        .unwrap_or_default();
    object.insert(
        "credential_id".to_string(),
        serde_json::Value::Number(local_id.into()),
    );
    Ok(())
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
