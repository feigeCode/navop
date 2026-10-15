//! 同步基线与同步结果状态
//!
//! `sync_item_state`：逐条同步基线（cloud_id / checksum / 变更时间 / 上次同步时间）。
//! 把这些信息从业务表外置，使新增类型无需 `ALTER TABLE`，文件系统型数据也有归属。
//!
//! `sync_type_status`：按数据类型聚合的同步结果，供设置页每行展示状态（Q12）。

use anyhow::Result;
use rusqlite::params;

use crate::storage::connection::SqliteConnection;

/// 一条数据的同步基线
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncItemState {
    pub scope_env: String,
    pub user_id: String,
    pub data_type: String,
    pub local_key: String,
    pub cloud_id: Option<String>,
    pub checksum: String,
    /// 本地内容变更时间（秒）。无时间戳的类型填 checksum 变更时刻。
    pub content_ts: i64,
    pub last_synced_at: Option<i64>,
    /// 上次同步成功时的载荷快照，仅设置类 Singleton 用（Q8 三方合并的 base）
    pub base_payload: Option<String>,
}

impl SyncItemState {
    pub fn new(
        scope_env: impl Into<String>,
        user_id: impl Into<String>,
        data_type: impl Into<String>,
        local_key: impl Into<String>,
    ) -> Self {
        Self {
            scope_env: scope_env.into(),
            user_id: user_id.into(),
            data_type: data_type.into(),
            local_key: local_key.into(),
            cloud_id: None,
            checksum: String::new(),
            content_ts: 0,
            last_synced_at: None,
            base_payload: None,
        }
    }
}

/// 某个数据类型的同步结果（设置页状态列的数据来源）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncTypeStatus {
    pub scope_env: String,
    pub user_id: String,
    pub data_type: String,
    pub last_success_at: Option<i64>,
    pub last_error_code: Option<String>,
    pub last_error_at: Option<i64>,
    pub last_error_msg: Option<String>,
}

impl SyncTypeStatus {
    /// 最近是否有未解决的失败：失败时间晚于上次成功时间才算「最近失败」。
    pub fn has_recent_error(&self) -> bool {
        match (self.last_error_at, self.last_success_at) {
            (Some(err), Some(ok)) => err > ok,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }
}

#[derive(Clone)]
pub struct SyncItemStateRepository {
    conn: SqliteConnection,
}

impl SyncItemStateRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn list(&self, scope_env: &str, user_id: &str) -> Result<Vec<SyncItemState>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT scope_env, user_id, data_type, local_key, cloud_id, checksum,
                        content_ts, last_synced_at, base_payload
                 FROM sync_item_state
                 WHERE scope_env = ?1 AND user_id = ?2",
            )?;
            let rows = stmt.query_map(params![scope_env, user_id], |row| {
                Ok(SyncItemState {
                    scope_env: row.get(0)?,
                    user_id: row.get(1)?,
                    data_type: row.get(2)?,
                    local_key: row.get(3)?,
                    cloud_id: row.get(4)?,
                    checksum: row.get(5)?,
                    content_ts: row.get(6)?,
                    last_synced_at: row.get(7)?,
                    base_payload: row.get(8)?,
                })
            })?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        })
    }

    pub fn get(
        &self,
        scope_env: &str,
        user_id: &str,
        data_type: &str,
        local_key: &str,
    ) -> Result<Option<SyncItemState>> {
        self.list(scope_env, user_id).map(|items| {
            items
                .into_iter()
                .find(|item| item.data_type == data_type && item.local_key == local_key)
        })
    }

    pub fn upsert(&self, state: &SyncItemState) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO sync_item_state (
                    scope_env, user_id, data_type, local_key, cloud_id,
                    checksum, content_ts, last_synced_at, base_payload
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (scope_env, user_id, data_type, local_key) DO UPDATE SET
                    cloud_id = excluded.cloud_id,
                    checksum = excluded.checksum,
                    content_ts = excluded.content_ts,
                    last_synced_at = excluded.last_synced_at,
                    base_payload = excluded.base_payload",
                params![
                    state.scope_env,
                    state.user_id,
                    state.data_type,
                    state.local_key,
                    state.cloud_id,
                    state.checksum,
                    state.content_ts,
                    state.last_synced_at,
                    state.base_payload,
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete(
        &self,
        scope_env: &str,
        user_id: &str,
        data_type: &str,
        local_key: &str,
    ) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "DELETE FROM sync_item_state
                 WHERE scope_env = ?1 AND user_id = ?2 AND data_type = ?3 AND local_key = ?4",
                params![scope_env, user_id, data_type, local_key],
            )?;
            Ok(())
        })
    }

    /// 回收「业务数据已不存在」的基线行。
    ///
    /// 只在确认该类型本轮完整枚举过之后调用，且必须排除正在删除传播中的条目。
    pub fn delete_missing(
        &self,
        scope_env: &str,
        user_id: &str,
        data_type: &str,
        alive_keys: &[String],
    ) -> Result<usize> {
        let existing = self.list(scope_env, user_id)?;
        let mut removed = 0;
        for item in existing
            .into_iter()
            .filter(|item| item.data_type == data_type)
            .filter(|item| !alive_keys.contains(&item.local_key))
        {
            self.delete(scope_env, user_id, data_type, &item.local_key)?;
            removed += 1;
        }
        Ok(removed)
    }
}

#[derive(Clone)]
pub struct SyncTypeStatusRepository {
    conn: SqliteConnection,
}

impl SyncTypeStatusRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn list(&self, scope_env: &str, user_id: &str) -> Result<Vec<SyncTypeStatus>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT scope_env, user_id, data_type, last_success_at,
                        last_error_code, last_error_at, last_error_msg
                 FROM sync_type_status
                 WHERE scope_env = ?1 AND user_id = ?2",
            )?;
            let rows = stmt.query_map(params![scope_env, user_id], |row| {
                Ok(SyncTypeStatus {
                    scope_env: row.get(0)?,
                    user_id: row.get(1)?,
                    data_type: row.get(2)?,
                    last_success_at: row.get(3)?,
                    last_error_code: row.get(4)?,
                    last_error_at: row.get(5)?,
                    last_error_msg: row.get(6)?,
                })
            })?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        })
    }

    pub fn get(
        &self,
        scope_env: &str,
        user_id: &str,
        data_type: &str,
    ) -> Result<Option<SyncTypeStatus>> {
        self.list(scope_env, user_id)
            .map(|items| items.into_iter().find(|item| item.data_type == data_type))
    }

    /// 记录一轮成功：写入成功时间并清空此前的失败（问题已解决）。
    pub fn mark_success(&self, scope_env: &str, user_id: &str, data_type: &str) -> Result<()> {
        let now = now_seconds();
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO sync_type_status (
                    scope_env, user_id, data_type, last_success_at,
                    last_error_code, last_error_at, last_error_msg
                 ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, NULL)
                 ON CONFLICT (scope_env, user_id, data_type) DO UPDATE SET
                    last_success_at = excluded.last_success_at,
                    last_error_code = NULL,
                    last_error_at = NULL,
                    last_error_msg = NULL",
                params![scope_env, user_id, data_type, now],
            )?;
            Ok(())
        })
    }

    pub fn mark_error(
        &self,
        scope_env: &str,
        user_id: &str,
        data_type: &str,
        code: &str,
        message: &str,
    ) -> Result<()> {
        let now = now_seconds();
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO sync_type_status (
                    scope_env, user_id, data_type, last_success_at,
                    last_error_code, last_error_at, last_error_msg
                 ) VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6)
                 ON CONFLICT (scope_env, user_id, data_type) DO UPDATE SET
                    last_error_code = excluded.last_error_code,
                    last_error_at = excluded.last_error_at,
                    last_error_msg = excluded.last_error_msg",
                params![scope_env, user_id, data_type, code, now, message],
            )?;
            Ok(())
        })
    }
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
