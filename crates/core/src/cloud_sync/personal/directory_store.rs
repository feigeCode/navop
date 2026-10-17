use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::cloud_sync::models::CloudSyncData;

use super::{
    APP_ID, PERSONAL_PROFILE_ID, PersonalSyncManifest, PersonalSyncStore, SUPPORTED_SCHEMA_VERSION,
    SyncDeviceId, SyncPackageLayout, SyncStoreError, SyncStoreLock, SyncStoreStatus, SyncTombstone,
};

/// 锁文件的过期时间。
///
/// 一次 pass 正常是秒级；超过这个时间还没释放，就认为持有者已经崩了，允许抢占。
/// 只按 TTL 判断（不查 pid 存活）是为了避开平台相关的进程探活；代价是崩在 pass 中间
/// 最多会让后续同步等这么久，换来的是实现足够简单、可验证。
const LOCK_STALE_AFTER: Duration = Duration::from_secs(300);

/// 锁文件内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DirectorySyncLockFile {
    owner: String,
    /// 每次获取都不同。释放时用它确认「这把锁还是我的」——TTL 过期后锁可能已经被
    /// 别的实例抢走，那时删掉文件会把对方的互斥一起破坏。
    nonce: String,
    pid: u32,
    acquired_at: i64,
}

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

    fn existing_record(
        &self,
        data_type: &str,
        id: &str,
    ) -> Result<Option<CloudSyncData>, SyncStoreError> {
        let path = self.layout.record_path(data_type, id);
        if path.exists() {
            return Ok(Some(self.read_record(&path)?));
        }
        Ok(None)
    }

    fn ensure_expected_version(
        &self,
        data_type: &str,
        id: &str,
        expected: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        let Some(expected) = expected else {
            return Ok(());
        };
        match self.existing_record(data_type, id)? {
            Some(record) if record.version == expected => Ok(()),
            _ => Err(SyncStoreError::Conflict(format!(
                "stale version for record {id}"
            ))),
        }
    }

    /// 抢占同步锁。
    ///
    /// 目录后端（以及复用它的 Git 后端）可能被同一台机器上的两个 navop 实例指向，
    /// 甚至被两台机器通过 Dropbox / iCloud 共享同一个目录；那时两边会并发地做
    /// 「读记录 → 改 → 写回」，光靠单文件原子写保不住版本。这里用锁文件让后到者
    /// 直接让路（[`SyncStoreError::LockTimeout`]），下一轮再试。
    ///
    /// 优先用 `create_new` 原子创建；已经被占时只有**确认过期**才抢占，抢占用
    /// 「临时文件 + rename」再回读 nonce 确认 —— 两个实例同时判断出「已过期」时，
    /// 只有回读到自己的 nonce 的那个才算抢到。
    fn acquire_directory_lock(
        &self,
        owner: &SyncDeviceId,
    ) -> Result<SyncStoreLock, SyncStoreError> {
        fs::create_dir_all(self.layout.root_dir())?;
        let path = self.layout.lock_path();
        let nonce = super::new_lock_nonce();
        let lock = DirectorySyncLockFile {
            owner: owner.0.clone(),
            nonce: nonce.clone(),
            pid: std::process::id(),
            acquired_at: now_millis(),
        };

        match create_lock_file(&path, &lock) {
            Ok(()) => return Ok(owned_directory_lock(owner, path, nonce)),
            Err(LockAttemptError::AlreadyHeld) => {}
            Err(LockAttemptError::Io(error)) => return Err(error),
        }

        if !lock_is_stale(&path) {
            return Err(SyncStoreError::LockTimeout);
        }
        write_json_atomically(&path, &lock)?;
        if read_lock_nonce(&path).as_deref() != Some(nonce.as_str()) {
            // 有人和我们同时抢：让路，别两边都以为自己在同步。
            return Err(SyncStoreError::LockTimeout);
        }
        Ok(owned_directory_lock(owner, path, nonce))
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
        let existing = self.existing_record(&record.data_type, &record.id)?;
        self.ensure_expected_version(&record.data_type, &record.id, expected_version)?;
        let stored = next_stored_record(record.clone(), existing.as_ref());
        write_json_atomically(
            &self.layout.record_path(&stored.data_type, &stored.id),
            &stored,
        )?;
        Ok(stored)
    }

    async fn tombstone_record(
        &self,
        data_type: &str,
        id: &str,
        expected_version: Option<u32>,
    ) -> Result<(), SyncStoreError> {
        self.initialize_package()?;
        self.ensure_expected_version(data_type, id, expected_version)?;
        let Some(mut record) = self.existing_record(data_type, id)? else {
            return Err(SyncStoreError::Conflict(format!(
                "missing {data_type} record {id}"
            )));
        };

        record.deleted_at = Some(now_millis());
        record.updated_at = now_millis();
        record.version = record.version.saturating_add(1);
        write_json_atomically(&self.layout.record_path(&record.data_type, id), &record)?;
        write_json_atomically(
            &self.layout.tombstone_path(&record.data_type, id),
            &tombstone_from(&record),
        )?;
        Ok(())
    }

    async fn acquire_lock(&self, owner: &SyncDeviceId) -> Result<SyncStoreLock, SyncStoreError> {
        self.acquire_directory_lock(owner)
    }
}

/// 抢锁失败的原因：已被占用（可继续判断是否过期），或真实的 IO 错误。
enum LockAttemptError {
    AlreadyHeld,
    Io(SyncStoreError),
}

/// 原子创建锁文件：`create_new` 保证「文件已存在」时一定失败，不会覆盖别人的锁。
fn create_lock_file(path: &Path, lock: &DirectorySyncLockFile) -> Result<(), LockAttemptError> {
    let bytes =
        serde_json::to_vec_pretty(lock).map_err(|error| LockAttemptError::Io(error.into()))?;
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(LockAttemptError::AlreadyHeld);
        }
        Err(error) => return Err(LockAttemptError::Io(error.into())),
    };
    file.write_all(&bytes)
        .map_err(|error| LockAttemptError::Io(error.into()))
}

fn read_lock_nonce(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice::<DirectorySyncLockFile>(&bytes)
        .ok()
        .map(|lock| lock.nonce)
}

/// 锁是否已经过期。
///
/// 锁文件读不到或解析不出来时一律当作「已过期」：坏文件不该把同步永久卡死。
fn lock_is_stale(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return true;
    };
    match serde_json::from_slice::<DirectorySyncLockFile>(&bytes) {
        Ok(lock) => {
            let age = now_millis().saturating_sub(lock.acquired_at);
            age >= LOCK_STALE_AFTER.as_millis() as i64
        }
        Err(_) => true,
    }
}

/// 构造一个「drop 时释放」的锁句柄。
///
/// 释放前先确认锁文件里的 nonce 还是自己的：TTL 过期后锁可能已经被别的实例抢走，
/// 那时删掉文件会把对方的互斥破坏掉。
fn owned_directory_lock(owner: &SyncDeviceId, path: PathBuf, nonce: String) -> SyncStoreLock {
    SyncStoreLock::with_release(owner.clone(), move || {
        if read_lock_nonce(&path).as_deref() != Some(nonce.as_str()) {
            return;
        }
        if let Err(error) = fs::remove_file(&path) {
            tracing::warn!(error = %error, "failed to release personal sync lock file");
        }
    })
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

fn next_stored_record(
    mut record: CloudSyncData,
    existing: Option<&CloudSyncData>,
) -> CloudSyncData {
    if let Some(existing) = existing {
        record.version = existing.version.saturating_add(1);
    } else {
        record.version = record.version.max(1);
    }
    record.updated_at = now_millis();
    record
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
    // 没有 id 的记录无法被寻址（既不能用它做匹配，也不能更新/删除它）。
    // 历史版本曾把「未知 id」写成空串，于是所有同类记录塌缩成同一个无名
    // `.json`；这里直接忽略，避免那份残破内容在每次同步时被当成新数据拉回本地。
    if record.id.trim().is_empty() || record.data_type.trim().is_empty() {
        return;
    }
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
