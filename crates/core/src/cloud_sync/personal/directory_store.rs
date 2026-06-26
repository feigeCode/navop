use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::Serialize;

use crate::cloud_sync::models::CloudSyncData;

use super::{
    APP_ID, PERSONAL_PROFILE_ID, PersonalSyncManifest, PersonalSyncStore, SUPPORTED_SCHEMA_VERSION,
    SyncDeviceId, SyncPackageLayout, SyncStoreError, SyncStoreLock, SyncStoreStatus, SyncTombstone,
};

#[derive(Debug, Clone)]
pub struct DirectorySyncStore {
    layout: SyncPackageLayout,
}

impl DirectorySyncStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            layout: SyncPackageLayout::new(root),
        }
    }

    fn initialize_package(&self) -> Result<(), SyncStoreError> {
        fs::create_dir_all(self.layout.records_dir())?;
        fs::create_dir_all(self.layout.tombstones_dir())?;
        fs::create_dir_all(self.layout.state_dir())?;

        if !self.layout.manifest_path().exists() {
            write_json_atomically(&self.layout.manifest_path(), &default_manifest())?;
        }

        self.read_manifest()?.validate()
    }

    fn read_manifest(&self) -> Result<PersonalSyncManifest, SyncStoreError> {
        let bytes = fs::read(self.layout.manifest_path())?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn read_record(&self, path: &Path) -> Result<CloudSyncData, SyncStoreError> {
        let bytes = fs::read(path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn existing_record(&self, id: &str) -> Result<Option<CloudSyncData>, SyncStoreError> {
        for entry in record_type_dirs(&self.layout)? {
            let path = entry.join(format!("{id}.json"));
            if path.exists() {
                return Ok(Some(self.read_record(&path)?));
            }
        }
        Ok(None)
    }

    fn ensure_expected_version(
        &self,
        id: &str,
        expected: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        let Some(expected) = expected else {
            return Ok(());
        };
        match self.existing_record(id)? {
            Some(record) if record.version == expected => Ok(()),
            _ => Err(SyncStoreError::Conflict(format!(
                "stale version for record {id}"
            ))),
        }
    }
}

#[async_trait]
impl PersonalSyncStore for DirectorySyncStore {
    fn backend_id(&self) -> &'static str {
        "folder"
    }

    async fn probe(&self) -> Result<SyncStoreStatus, SyncStoreError> {
        self.initialize_package()?;
        Ok(SyncStoreStatus::ready())
    }

    async fn list_records(
        &self,
        data_type: Option<&str>,
        since: Option<i64>,
    ) -> Result<Vec<CloudSyncData>, SyncStoreError> {
        self.initialize_package()?;
        let mut records = Vec::new();
        for dir in record_type_dirs(&self.layout)? {
            read_matching_records(&mut records, &dir, data_type, since, self)?;
        }
        records.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(records)
    }

    async fn upsert_record(
        &self,
        record: &CloudSyncData,
        expected_version: Option<u32>,
    ) -> Result<CloudSyncData, SyncStoreError> {
        self.initialize_package()?;
        self.ensure_expected_version(&record.id, expected_version)?;
        write_json_atomically(
            &self.layout.record_path(&record.data_type, &record.id),
            record,
        )?;
        Ok(record.clone())
    }

    async fn tombstone_record(
        &self,
        id: &str,
        expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        self.initialize_package()?;
        self.ensure_expected_version(id, expected_version)?;
        let Some(mut record) = self.existing_record(id)? else {
            return Err(SyncStoreError::Conflict(format!("missing record {id}")));
        };

        record.deleted_at = Some(now_millis());
        write_json_atomically(&self.layout.record_path(&record.data_type, id), &record)?;
        write_json_atomically(&self.layout.tombstone_path(id), &tombstone_from(&record))?;
        Ok(())
    }

    async fn acquire_lock(&self, owner: &SyncDeviceId) -> Result<SyncStoreLock, SyncStoreError> {
        Ok(SyncStoreLock {
            owner: owner.clone(),
        })
    }
}

fn default_manifest() -> PersonalSyncManifest {
    let now = now_millis();
    PersonalSyncManifest {
        schema_version: SUPPORTED_SCHEMA_VERSION,
        app: APP_ID.to_string(),
        profile_id: PERSONAL_PROFILE_ID.to_string(),
        created_at: now,
        updated_at: now,
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn record_type_dirs(layout: &SyncPackageLayout) -> Result<Vec<PathBuf>, SyncStoreError> {
    if !layout.records_dir().exists() {
        return Ok(Vec::new());
    }

    let mut dirs = Vec::new();
    for entry in fs::read_dir(layout.records_dir())? {
        let path = entry?.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    Ok(dirs)
}

fn read_matching_records(
    records: &mut Vec<CloudSyncData>,
    dir: &Path,
    data_type: Option<&str>,
    since: Option<i64>,
    store: &DirectorySyncStore,
) -> Result<(), SyncStoreError> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        push_if_matches(records, store.read_record(&path)?, data_type, since);
    }
    Ok(())
}

fn push_if_matches(
    records: &mut Vec<CloudSyncData>,
    record: CloudSyncData,
    data_type: Option<&str>,
    since: Option<i64>,
) {
    if data_type.is_some_and(|target| record.data_type != target) {
        return;
    }
    if since.is_some_and(|timestamp| record.updated_at < timestamp) {
        return;
    }
    records.push(record);
}

fn tombstone_from(record: &CloudSyncData) -> SyncTombstone {
    SyncTombstone {
        id: record.id.clone(),
        data_type: record.data_type.clone(),
        deleted_at: record.deleted_at.unwrap_or_else(now_millis),
        version: record.version,
        checksum: record.checksum.clone(),
    }
}

fn write_json_atomically(path: &Path, value: &impl Serialize) -> Result<(), SyncStoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| SyncStoreError::Io("missing parent directory".to_string()))?;
    fs::create_dir_all(parent)?;
    let temp_path = path.with_extension("tmp");
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(&temp_path, bytes)?;
    fs::rename(&temp_path, path)?;
    Ok(())
}
