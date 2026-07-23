use anyhow::Result;
use gpui::{App, SharedString};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::storage::connection::SqliteConnection;
use crate::storage::manager::{GlobalStorageState, now};
use crate::storage::models::has_decrypt_failure_in_sensitive_fields;
use crate::storage::quick_command::QuickCommandRepository;
use crate::storage::row_mapping::FromSqliteRow;
use crate::storage::sftp_favorite_path::SftpFavoritePathRepository;
use crate::storage::team_key_cache::TeamKeyCacheRepository;
use crate::storage::team_membership_cache::TeamMembershipCacheRepository;
use crate::storage::terminal_command_history::TerminalCommandHistoryRepository;
use crate::storage::traits::Repository;
use crate::storage::{ConnectionType, StoredConnection, Workspace};

struct ConnectionRow {
    id: i64,
    name: String,
    connection_type: String,
    params: String,
    workspace_id: Option<i64>,
    selected_databases: Option<String>,
    remark: Option<String>,
    sync_enabled: bool,
    cloud_id: Option<String>,
    last_synced_at: Option<i64>,
    last_used_at: Option<i64>,
    sort_order: Option<i32>,
    created_at: i64,
    updated_at: i64,
    team_id: Option<String>,
    owner_id: Option<String>,
}

impl FromSqliteRow for ConnectionRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(ConnectionRow {
            id: row.get("id")?,
            name: row.get("name")?,
            connection_type: row.get("connection_type")?,
            params: row.get("params")?,
            workspace_id: row.get("workspace_id")?,
            selected_databases: row.get("selected_databases")?,
            remark: row.get("remark")?,
            sync_enabled: row
                .get::<_, i64>("sync_enabled")
                .map(|v| v != 0)
                .unwrap_or(true),
            cloud_id: row.get("cloud_id")?,
            last_synced_at: row.get("last_synced_at")?,
            last_used_at: row.get("last_used_at")?,
            sort_order: row.get("sort_order").unwrap_or(Some(0)),
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            team_id: row.get("team_id").unwrap_or(None),
            owner_id: row.get("owner_id").unwrap_or(None),
        })
    }
}

impl From<ConnectionRow> for StoredConnection {
    fn from(row: ConnectionRow) -> Self {
        let mut conn = StoredConnection {
            id: Some(row.id),
            name: row.name,
            connection_type: ConnectionType::from_str(&row.connection_type),
            params: row.params,
            workspace_id: row.workspace_id,
            selected_databases: row.selected_databases,
            remark: row.remark,
            sync_enabled: row.sync_enabled,
            cloud_id: row.cloud_id,
            last_synced_at: row.last_synced_at,
            last_used_at: row.last_used_at,
            sort_order: row.sort_order,
            created_at: Some(row.created_at),
            updated_at: Some(row.updated_at),
            team_id: row.team_id,
            owner_id: row.owner_id,
        };
        // 从数据库读取后自动解密敏感字段
        conn.params = conn.decrypt_params();
        conn
    }
}

struct WorkspaceRow {
    id: i64,
    name: String,
    color: Option<String>,
    icon: Option<String>,
    parent_id: Option<i64>,
    created_at: i64,
    updated_at: i64,
    cloud_id: Option<String>,
    last_synced_at: Option<i64>,
    sort_order: Option<i32>,
}

impl FromSqliteRow for WorkspaceRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(WorkspaceRow {
            id: row.get("id")?,
            name: row.get("name")?,
            color: row.get("color")?,
            icon: row.get("icon")?,
            parent_id: row.get("parent_id")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            cloud_id: row.get("cloud_id")?,
            last_synced_at: row.get("last_synced_at").unwrap_or(None),
            sort_order: row.get("sort_order").unwrap_or(Some(0)),
        })
    }
}

impl From<WorkspaceRow> for Workspace {
    fn from(row: WorkspaceRow) -> Self {
        Workspace {
            id: Some(row.id),
            name: row.name,
            color: row.color,
            icon: row.icon,
            parent_id: row.parent_id,
            created_at: Some(row.created_at),
            updated_at: Some(row.updated_at),
            cloud_id: row.cloud_id,
            last_synced_at: row.last_synced_at,
            sort_order: row.sort_order,
        }
    }
}

#[derive(Clone)]
pub struct ConnectionRepository {
    conn: SqliteConnection,
}

impl ConnectionRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn upsert_cloud_connection(&self, item: &mut StoredConnection) -> Result<()> {
        let cloud_id = item
            .cloud_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Cloud connection requires cloud_id"))?;
        let connection_type = item.connection_type.to_string();
        let encrypted_params = item.encrypt_params();
        let sync_enabled = i64::from(item.sync_enabled);
        let ts = now();
        let id = self.conn.with_connection(|conn| {
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                TransactionBehavior::Immediate,
            )?;
            let existing_id = tx
                .query_row(
                    "SELECT id FROM connections WHERE cloud_id = ?1 ORDER BY id LIMIT 1",
                    params![cloud_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            let id = if let Some(id) = existing_id {
                tx.execute(
                    "UPDATE connections SET name = ?1, connection_type = ?2, params = ?3, workspace_id = ?4, selected_databases = ?5, remark = ?6, sync_enabled = ?7, cloud_id = ?8, last_synced_at = ?9, team_id = ?10, owner_id = ?11, updated_at = ?12 WHERE id = ?13",
                    params![item.name, connection_type, encrypted_params, item.workspace_id, item.selected_databases, item.remark, sync_enabled, cloud_id, item.last_synced_at, item.team_id, item.owner_id, ts, id],
                )?;
                id
            } else {
                tx.execute(
                    "INSERT INTO connections (name, connection_type, params, workspace_id, selected_databases, remark, sync_enabled, cloud_id, last_synced_at, team_id, owner_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![item.name, connection_type, encrypted_params, item.workspace_id, item.selected_databases, item.remark, sync_enabled, cloud_id, item.last_synced_at, item.team_id, item.owner_id, ts, ts],
                )?;
                tx.last_insert_rowid()
            };
            tx.commit()?;
            Ok(id)
        })?;
        item.id = Some(id);
        item.created_at.get_or_insert(ts);
        item.updated_at = Some(ts);
        Ok(())
    }

    pub fn update_workspace(&self, id: i64, workspace_id: Option<i64>) -> Result<i64> {
        let ts = now();
        self.conn.with_connection(|conn| {
            let updated = conn.execute(
                "UPDATE connections SET workspace_id = ?1, updated_at = ?2 WHERE id = ?3",
                params![workspace_id, ts, id],
            )?;
            anyhow::ensure!(updated == 1, "Connection {id} not found");
            Ok(ts)
        })
    }
}

impl Repository for ConnectionRepository {
    type Entity = StoredConnection;

    fn entity_type(&self) -> SharedString {
        SharedString::from("Connection")
    }

    fn insert(&self, item: &mut Self::Entity) -> Result<i64> {
        let name = item.name.clone();
        let connection_type = item.connection_type.to_string();
        let params_str = item.encrypt_params();
        let workspace_id = item.workspace_id;
        let selected_databases = item.selected_databases.clone();
        let remark = item.remark.clone();
        let sync_enabled = if item.sync_enabled { 1i64 } else { 0i64 };
        let cloud_id = item.cloud_id.clone();
        let last_synced_at = item.last_synced_at;
        let team_id = item.team_id.clone();
        let owner_id = item.owner_id.clone();
        let ts = now();

        let id = self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO connections (name, connection_type, params, workspace_id, selected_databases, remark, sync_enabled, cloud_id, last_synced_at, team_id, owner_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![name, connection_type, params_str, workspace_id, selected_databases, remark, sync_enabled, cloud_id, last_synced_at, team_id, owner_id, ts, ts],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        item.id = Some(id);
        item.created_at = Some(ts);
        item.updated_at = Some(ts);

        Ok(id)
    }

    fn update(&self, item: &Self::Entity) -> Result<()> {
        let id = item
            .id
            .ok_or_else(|| anyhow::anyhow!("Cannot update without ID"))?;
        let name = item.name.clone();
        let connection_type = item.connection_type.to_string();
        let params_str = item.encrypt_params();
        let workspace_id = item.workspace_id;
        let selected_databases = item.selected_databases.clone();
        let remark = item.remark.clone();
        let sync_enabled = if item.sync_enabled { 1i64 } else { 0i64 };
        let cloud_id = item.cloud_id.clone();
        let last_synced_at = item.last_synced_at;
        let team_id = item.team_id.clone();
        let owner_id = item.owner_id.clone();
        let ts = now();

        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET name = ?1, connection_type = ?2, params = ?3, workspace_id = ?4, selected_databases = ?5, remark = ?6, sync_enabled = ?7, cloud_id = ?8, last_synced_at = ?9, team_id = ?10, owner_id = ?11, updated_at = ?12 WHERE id = ?13",
                params![name, connection_type, params_str, workspace_id, selected_databases, remark, sync_enabled, cloud_id, last_synced_at, team_id, owner_id, ts, id],
            )?;
            Ok(())
        })
    }

    fn delete(&self, id: i64) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute("DELETE FROM connections WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    fn get(&self, id: i64) -> Result<Option<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, connection_type, params, workspace_id, selected_databases, remark, sync_enabled, cloud_id, last_synced_at, last_used_at, sort_order, created_at, updated_at, team_id, owner_id FROM connections WHERE id = ?1",
            )?;
            let mut rows = stmt.query(params![id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(ConnectionRow::from_row(row)?.into()))
            } else {
                Ok(None)
            }
        })
    }

    fn list(&self) -> Result<Vec<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, connection_type, params, workspace_id, selected_databases, remark, sync_enabled, cloud_id, last_synced_at, last_used_at, sort_order, created_at, updated_at, team_id, owner_id FROM connections ORDER BY COALESCE(last_used_at, updated_at, created_at) DESC, id DESC",
            )?;
            let rows = stmt.query_map([], |row| ConnectionRow::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?.into());
            }
            Ok(results)
        })
    }

    fn count(&self) -> Result<i64> {
        self.conn.with_connection(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM connections", [], |row| row.get(0))?;
            Ok(count)
        })
    }

    fn exists(&self, id: i64) -> Result<bool> {
        self.conn.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM connections WHERE id = ?1)",
                params![id],
                |row| row.get(0),
            )?;
            Ok(exists == 1)
        })
    }
}

impl ConnectionRepository {
    pub fn list_by_workspace(&self, workspace_id: Option<i64>) -> Result<Vec<StoredConnection>> {
        self.conn.with_connection(|conn| {
            let sql = if workspace_id.is_some() {
                "SELECT id, name, connection_type, params, workspace_id, selected_databases, remark, sync_enabled, cloud_id, last_synced_at, last_used_at, sort_order, created_at, updated_at, team_id, owner_id FROM connections WHERE workspace_id = ?1 ORDER BY COALESCE(last_used_at, updated_at, created_at) DESC, id DESC"
            } else {
                "SELECT id, name, connection_type, params, workspace_id, selected_databases, remark, sync_enabled, cloud_id, last_synced_at, last_used_at, sort_order, created_at, updated_at, team_id, owner_id FROM connections WHERE workspace_id IS NULL ORDER BY COALESCE(last_used_at, updated_at, created_at) DESC, id DESC"
            };
            let mut stmt = conn.prepare(sql)?;

            let mut results = Vec::new();
            if let Some(wid) = workspace_id {
                let rows = stmt.query_map(params![wid], |row| ConnectionRow::from_row(row))?;
                for row in rows {
                    results.push(row?.into());
                }
            } else {
                let rows = stmt.query_map([], |row| ConnectionRow::from_row(row))?;
                for row in rows {
                    results.push(row?.into());
                }
            }
            Ok(results)
        })
    }

    /// 更新连接的同步状态
    ///
    /// 同步成功后调用，设置 cloud_id 和 last_synced_at
    pub fn update_sync_status(
        &self,
        id: i64,
        cloud_id: Option<String>,
        last_synced_at: Option<i64>,
    ) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET cloud_id = ?1, last_synced_at = ?2 WHERE id = ?3",
                params![cloud_id, last_synced_at, id],
            )?;
            Ok(())
        })
    }

    pub fn update_sync_status_with_updated_at(
        &self,
        id: i64,
        cloud_id: Option<String>,
        last_synced_at: Option<i64>,
        updated_at: i64,
    ) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET cloud_id = ?1, last_synced_at = ?2, updated_at = ?3 WHERE id = ?4",
                params![cloud_id, last_synced_at, updated_at, id],
            )?;
            Ok(())
        })
    }

    /// 记录连接最近一次被打开的时间，不影响内容更新时间和云同步判断。
    pub fn touch_last_used(&self, id: i64) -> Result<()> {
        let ts = now();
        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET last_used_at = ?1 WHERE id = ?2",
                params![ts, id],
            )?;
            Ok(())
        })
    }

    /// 暂停连接拖拽排序：当前连接列表以 LRU 为准，后续重新设计手动排序与 LRU 的关系后再启用。
    #[allow(dead_code)]
    pub fn update_sort_orders(&self, orders: &[(i64, i32)]) -> Result<()> {
        self.conn.with_connection(|conn| {
            for (id, sort_order) in orders {
                conn.execute(
                    "UPDATE connections SET sort_order = ?1 WHERE id = ?2",
                    params![sort_order, id],
                )?;
            }
            Ok(())
        })
    }

    /// 查询需要同步的连接（sync_enabled=true 且 cloud_id 为空或 updated_at > last_synced_at）
    pub fn list_pending_sync(&self) -> Result<Vec<StoredConnection>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, connection_type, params, workspace_id, selected_databases, remark, sync_enabled, cloud_id, last_synced_at, last_used_at, sort_order, created_at, updated_at, team_id, owner_id
                 FROM connections
                 WHERE sync_enabled = 1 AND (cloud_id IS NULL OR updated_at > COALESCE(last_synced_at, 0))
                 ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map([], |row| ConnectionRow::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?.into());
            }
            Ok(results)
        })
    }

    /// 根据 cloud_id 查询连接
    pub fn get_by_cloud_id(&self, cloud_id: &str) -> Result<Option<StoredConnection>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, connection_type, params, workspace_id, selected_databases, remark, sync_enabled, cloud_id, last_synced_at, last_used_at, sort_order, created_at, updated_at, team_id, owner_id
                 FROM connections WHERE cloud_id = ?1",
            )?;
            let mut rows = stmt.query(params![cloud_id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(ConnectionRow::from_row(row)?.into()))
            } else {
                Ok(None)
            }
        })
    }

    /// 检测启用同步的连接中是否存在解密失败的数据。
    ///
    /// 返回值为 (id, name) 列表，便于上层记录日志与阻断同步。
    pub fn list_sync_decrypt_failures(&self) -> Result<Vec<(i64, String)>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, params FROM connections WHERE sync_enabled = 1 ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map([], |row| {
                let id: i64 = row.get("id")?;
                let name: String = row.get("name")?;
                let params: String = row.get("params")?;
                Ok((id, name, params))
            })?;

            let mut failures = Vec::new();
            for row in rows {
                let (id, name, params) = row?;
                if has_decrypt_failure_in_sensitive_fields(&params) {
                    failures.push((id, name));
                }
            }
            Ok(failures)
        })
    }

    /// 按团队 ID 查询连接
    pub fn list_by_team(&self, team_id: &str) -> Result<Vec<StoredConnection>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, connection_type, params, workspace_id, selected_databases, remark, sync_enabled, cloud_id, last_synced_at, last_used_at, sort_order, created_at, updated_at, team_id, owner_id FROM connections WHERE team_id = ?1 ORDER BY COALESCE(last_used_at, updated_at, created_at) DESC, id DESC",
            )?;
            let rows = stmt.query_map(params![team_id], |row| ConnectionRow::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?.into());
            }
            Ok(results)
        })
    }

    /// 查询个人连接（team_id 为 NULL）
    pub fn list_personal(&self) -> Result<Vec<StoredConnection>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, connection_type, params, workspace_id, selected_databases, remark, sync_enabled, cloud_id, last_synced_at, last_used_at, sort_order, created_at, updated_at, team_id, owner_id FROM connections WHERE team_id IS NULL ORDER BY COALESCE(last_used_at, updated_at, created_at) DESC, id DESC",
            )?;
            let rows = stmt.query_map([], |row| ConnectionRow::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?.into());
            }
            Ok(results)
        })
    }
}

#[derive(Clone)]
pub struct WorkspaceRepository {
    conn: SqliteConnection,
}

impl WorkspaceRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn update_from_cloud(&self, item: &Workspace) -> Result<()> {
        let id = item
            .id
            .ok_or_else(|| anyhow::anyhow!("Cannot update without ID"))?;
        let name = item.name.clone();
        let color = item.color.clone();
        let icon = item.icon.clone();
        let cloud_id = item.cloud_id.clone();
        let last_synced_at = item.last_synced_at;
        let sort_order = item.sort_order;
        let updated_at = item.updated_at.unwrap_or_else(now);

        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE workspaces SET name = ?1, color = ?2, icon = ?3, cloud_id = ?4, last_synced_at = ?5, sort_order = COALESCE(?6, sort_order), updated_at = ?7 WHERE id = ?8",
                params![name, color, icon, cloud_id, last_synced_at, sort_order, updated_at, id],
            )?;
            Ok(())
        })
    }

    /// 更新工作空间的云端同步状态
    pub fn update_cloud_id(&self, local_id: i64, cloud_id: Option<String>) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE workspaces SET cloud_id = ?1 WHERE id = ?2",
                params![cloud_id, local_id],
            )?;
            Ok(())
        })
    }

    /// 更新工作空间的云端同步状态和最后同步时间。
    pub fn update_sync_status(
        &self,
        local_id: i64,
        cloud_id: Option<String>,
        last_synced_at: Option<i64>,
    ) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE workspaces SET cloud_id = ?1, last_synced_at = ?2 WHERE id = ?3",
                params![cloud_id, last_synced_at, local_id],
            )?;
            Ok(())
        })
    }

    pub fn update_sort_orders(&self, orders: &[(i64, i32)]) -> Result<()> {
        self.conn.with_connection(|conn| {
            let tx = conn.unchecked_transaction()?;
            let ts = now();
            for (id, sort_order) in orders {
                tx.execute(
                    "UPDATE workspaces SET sort_order = ?1, updated_at = ?2 WHERE id = ?3",
                    params![sort_order, ts, id],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    fn next_sort_order(&self) -> Result<i32> {
        self.conn.with_connection(|conn| {
            let max_order: Option<i32> =
                conn.query_row("SELECT MAX(sort_order) FROM workspaces", [], |row| {
                    row.get(0)
                })?;
            Ok(max_order.unwrap_or(-1) + 1)
        })
    }
}

impl Repository for WorkspaceRepository {
    type Entity = Workspace;

    fn entity_type(&self) -> SharedString {
        SharedString::from("Workspace")
    }

    fn insert(&self, item: &mut Self::Entity) -> Result<i64> {
        let name = item.name.clone();
        let color = item.color.clone();
        let icon = item.icon.clone();
        let parent_id = item.parent_id;
        let cloud_id = item.cloud_id.clone();
        let last_synced_at = item.last_synced_at;
        let sort_order = item.sort_order.unwrap_or(self.next_sort_order()?);
        let ts = now();

        let id = self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO workspaces (name, color, icon, parent_id, cloud_id, last_synced_at, sort_order, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![name, color, icon, parent_id, cloud_id, last_synced_at, sort_order, ts, ts],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        item.id = Some(id);
        item.sort_order = Some(sort_order);
        item.created_at = Some(ts);
        item.updated_at = Some(ts);

        Ok(id)
    }

    fn update(&self, item: &Self::Entity) -> Result<()> {
        let id = item
            .id
            .ok_or_else(|| anyhow::anyhow!("Cannot update without ID"))?;
        let name = item.name.clone();
        let color = item.color.clone();
        let icon = item.icon.clone();
        let parent_id = item.parent_id;
        let cloud_id = item.cloud_id.clone();
        let last_synced_at = item.last_synced_at;
        let sort_order = item.sort_order;
        let ts = now();

        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE workspaces SET name = ?1, color = ?2, icon = ?3, parent_id = ?4, cloud_id = ?5, last_synced_at = ?6, sort_order = COALESCE(?7, sort_order), updated_at = ?8 WHERE id = ?9",
                params![name, color, icon, parent_id, cloud_id, last_synced_at, sort_order, ts, id],
            )?;
            Ok(())
        })
    }

    fn delete(&self, id: i64) -> Result<()> {
        self.conn.with_connection(|conn| {
            let transaction = conn.unchecked_transaction()?;
            transaction.execute(
                "UPDATE connections SET workspace_id = NULL WHERE workspace_id = ?1",
                params![id],
            )?;
            transaction.execute(
                "UPDATE workspaces SET parent_id = NULL WHERE parent_id = ?1",
                params![id],
            )?;
            transaction.execute("DELETE FROM workspaces WHERE id = ?1", params![id])?;
            transaction.commit()?;
            Ok(())
        })
    }

    fn get(&self, id: i64) -> Result<Option<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id, name, color, icon, parent_id, created_at, updated_at, cloud_id, last_synced_at, sort_order FROM workspaces WHERE id = ?1")?;
            let mut rows = stmt.query(params![id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(WorkspaceRow::from_row(row)?.into()))
            } else {
                Ok(None)
            }
        })
    }

    fn list(&self) -> Result<Vec<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id, name, color, icon, parent_id, created_at, updated_at, cloud_id, last_synced_at, sort_order FROM workspaces ORDER BY sort_order ASC, updated_at DESC, id DESC")?;
            let rows = stmt.query_map([], |row| WorkspaceRow::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?.into());
            }
            Ok(results)
        })
    }

    fn count(&self) -> Result<i64> {
        self.conn.with_connection(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM workspaces", [], |row| row.get(0))?;
            Ok(count)
        })
    }

    fn exists(&self, id: i64) -> Result<bool> {
        self.conn.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id = ?1)",
                params![id],
                |row| row.get(0),
            )?;
            Ok(exists == 1)
        })
    }
}

/// 待删除云端记录
#[derive(Debug, Clone)]
pub struct PendingCloudDeletion {
    pub id: Option<i64>,
    pub cloud_id: String,
    pub entity_type: String,
    pub created_at: i64,
}

/// 待删除云端记录仓库
#[derive(Clone)]
pub struct PendingCloudDeletionRepository {
    conn: SqliteConnection,
}

impl PendingCloudDeletionRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    /// 添加待删除记录
    pub fn add(&self, cloud_id: &str, entity_type: &str) -> Result<()> {
        let ts = now();
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO pending_cloud_deletions (cloud_id, entity_type, created_at) VALUES (?1, ?2, ?3)",
                params![cloud_id, entity_type, ts],
            )?;
            Ok(())
        })
    }

    /// 获取所有待删除的连接
    pub fn list_connections(&self) -> Result<Vec<PendingCloudDeletion>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, cloud_id, entity_type, created_at FROM pending_cloud_deletions WHERE entity_type = 'connection'"
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(PendingCloudDeletion {
                    id: row.get(0)?,
                    cloud_id: row.get(1)?,
                    entity_type: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    /// 获取所有待删除的工作空间
    pub fn list_workspaces(&self) -> Result<Vec<PendingCloudDeletion>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, cloud_id, entity_type, created_at FROM pending_cloud_deletions WHERE entity_type = 'workspace'"
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(PendingCloudDeletion {
                    id: row.get(0)?,
                    cloud_id: row.get(1)?,
                    entity_type: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    /// 删除记录（同步成功后调用）
    pub fn remove(&self, cloud_id: &str) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "DELETE FROM pending_cloud_deletions WHERE cloud_id = ?1",
                params![cloud_id],
            )?;
            Ok(())
        })
    }

    /// 检查 cloud_id 是否在待删除列表中
    pub fn is_pending(&self, cloud_id: &str) -> Result<bool> {
        self.conn.with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM pending_cloud_deletions WHERE cloud_id = ?1",
                params![cloud_id],
                |row| row.get(0),
            )?;
            Ok(count > 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectionRepository, WorkspaceRepository};
    use crate::storage::connection::SqliteConnection;
    use crate::storage::migration::run_migrations;
    use crate::storage::models::{SshAuthMethod, SshParams};
    use crate::storage::traits::Repository;
    use crate::storage::{StoredConnection, Workspace};
    use rusqlite::params;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};

    static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_repository() -> (SqliteConnection, ConnectionRepository) {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let db_path = std::env::temp_dir().join(format!(
            "onetcli-connection-repository-{}-{unique}-{counter}.db",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&db_path);
        let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
        conn.with_connection(|conn| run_migrations(conn))
            .expect("run migrations");
        let repo = ConnectionRepository::new(conn.clone());
        (conn, repo)
    }

    fn ssh_connection(name: &str) -> StoredConnection {
        StoredConnection::new_ssh(
            name.to_string(),
            SshParams {
                host: format!("{name}.example.com"),
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
                os_id: None,
                icon: None,
            },
            None,
        )
    }

    fn workspace(name: &str) -> Workspace {
        Workspace::new(name.to_string())
    }

    #[test]
    fn workspace_list_uses_manual_sort_order() {
        let (conn, _) = test_repository();
        let repo = WorkspaceRepository::new(conn);
        let mut first = workspace("first");
        let mut second = workspace("second");
        let mut third = workspace("third");
        let first_id = repo.insert(&mut first).unwrap();
        let second_id = repo.insert(&mut second).unwrap();
        let third_id = repo.insert(&mut third).unwrap();

        repo.update_sort_orders(&[(third_id, 0), (first_id, 1), (second_id, 2)])
            .unwrap();

        let listed_ids = repo
            .list()
            .unwrap()
            .into_iter()
            .map(|workspace| workspace.id)
            .collect::<Vec<_>>();

        assert_eq!(
            vec![Some(third_id), Some(first_id), Some(second_id)],
            listed_ids
        );
    }

    #[test]
    fn workspace_repository_preserves_parent_hierarchy() {
        let (conn, _) = test_repository();
        let repo = WorkspaceRepository::new(conn);
        let mut parent = workspace("parent");
        let parent_id = repo.insert(&mut parent).unwrap();
        let mut child = workspace("child");
        child.parent_id = Some(parent_id);
        let child_id = repo.insert(&mut child).unwrap();

        assert_eq!(
            Some(parent_id),
            repo.get(child_id).unwrap().unwrap().parent_id
        );

        repo.delete(parent_id).unwrap();

        assert_eq!(None, repo.get(child_id).unwrap().unwrap().parent_id);
    }

    #[test]
    fn workspace_cloud_update_does_not_flatten_local_hierarchy() {
        let (conn, _) = test_repository();
        let repo = WorkspaceRepository::new(conn);
        let mut parent = workspace("parent");
        let parent_id = repo.insert(&mut parent).unwrap();
        let mut child = workspace("child");
        child.parent_id = Some(parent_id);
        let child_id = repo.insert(&mut child).unwrap();

        let mut cloud_child = repo.get(child_id).unwrap().unwrap();
        cloud_child.parent_id = None;
        cloud_child.name = "cloud child".to_string();
        repo.update_from_cloud(&cloud_child).unwrap();

        let stored = repo.get(child_id).unwrap().unwrap();
        assert_eq!("cloud child", stored.name);
        assert_eq!(Some(parent_id), stored.parent_id);
    }

    #[test]
    fn workspace_update_persists_sort_order() {
        let (conn, _) = test_repository();
        let repo = WorkspaceRepository::new(conn);
        let mut first = workspace("first");
        first.sort_order = Some(0);
        let first_id = repo.insert(&mut first).unwrap();
        let mut second = workspace("second");
        second.sort_order = Some(1);
        let second_id = repo.insert(&mut second).unwrap();

        second.sort_order = Some(-1);
        repo.update(&second).unwrap();

        let listed_ids = repo
            .list()
            .unwrap()
            .into_iter()
            .map(|workspace| workspace.id)
            .collect::<Vec<_>>();

        assert_eq!(vec![Some(second_id), Some(first_id)], listed_ids);
    }

    #[test]
    fn list_orders_by_recent_use_without_touching_updated_at() {
        let (conn, repo) = test_repository();
        let mut old_connection = ssh_connection("old");
        let old_id = repo.insert(&mut old_connection).unwrap();
        let mut new_connection = ssh_connection("new");
        let new_id = repo.insert(&mut new_connection).unwrap();

        conn.with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET created_at = ?1, updated_at = ?1 WHERE id = ?2",
                params![1000i64, old_id],
            )?;
            conn.execute(
                "UPDATE connections SET created_at = ?1, updated_at = ?1 WHERE id = ?2",
                params![2000i64, new_id],
            )?;
            Ok(())
        })
        .unwrap();

        assert_eq!(
            Some(new_id),
            repo.list().unwrap().first().and_then(|c| c.id)
        );

        repo.touch_last_used(old_id).unwrap();
        let listed = repo.list().unwrap();

        assert_eq!(Some(old_id), listed.first().and_then(|c| c.id));
        let (updated_at, last_used_at): (i64, Option<i64>) = conn
            .with_connection(|conn| {
                Ok(conn.query_row(
                    "SELECT updated_at, last_used_at FROM connections WHERE id = ?1",
                    params![old_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?)
            })
            .unwrap();
        assert_eq!(1000, updated_at);
        assert!(last_used_at.is_some());
    }

    #[test]
    fn list_ignores_legacy_sort_order_for_recent_use() {
        let (conn, repo) = test_repository();
        let mut old_connection = ssh_connection("old");
        let old_id = repo.insert(&mut old_connection).unwrap();
        let mut new_connection = ssh_connection("new");
        let new_id = repo.insert(&mut new_connection).unwrap();

        conn.with_connection(|conn| {
            conn.execute(
                "UPDATE connections SET created_at = ?1, updated_at = ?1, sort_order = ?2 WHERE id = ?3",
                params![1000i64, 0i32, old_id],
            )?;
            conn.execute(
                "UPDATE connections SET created_at = ?1, updated_at = ?1, sort_order = ?2 WHERE id = ?3",
                params![2000i64, 100i32, new_id],
            )?;
            Ok(())
        })
        .unwrap();

        assert_eq!(
            Some(new_id),
            repo.list().unwrap().first().and_then(|c| c.id)
        );
    }

    #[test]
    fn connection_workspace_update_preserves_connection_params() {
        let (conn, repo) = test_repository();
        let workspace_repo = WorkspaceRepository::new(conn.clone());
        let mut workspace = workspace("production");
        let workspace_id = workspace_repo.insert(&mut workspace).expect("workspace");
        let mut connection = ssh_connection("primary");
        let connection_id = repo.insert(&mut connection).expect("connection");
        let params_before = raw_connection_params(&conn, connection_id);

        repo.update_workspace(connection_id, Some(workspace_id))
            .expect("assign workspace");

        let assigned = repo.get(connection_id).expect("read").expect("connection");
        assert_eq!(Some(workspace_id), assigned.workspace_id);
        assert_eq!(params_before, raw_connection_params(&conn, connection_id));

        repo.update_workspace(connection_id, None)
            .expect("clear workspace");
        let unassigned = repo.get(connection_id).expect("read").expect("connection");
        assert_eq!(None, unassigned.workspace_id);
    }

    fn raw_connection_params(conn: &SqliteConnection, connection_id: i64) -> String {
        conn.with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT params FROM connections WHERE id = ?1",
                params![connection_id],
                |row| row.get(0),
            )?)
        })
        .expect("raw connection params")
    }

    #[test]
    fn cloud_download_upsert_serializes_duplicate_inserts() {
        let (_, repo) = test_repository();
        let barrier = Arc::new(Barrier::new(3));
        let handles = ["first", "second"].map(|name| {
            let repo = repo.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut connection = ssh_connection(name);
                connection.cloud_id = Some("shared-cloud-id".to_string());
                barrier.wait();
                repo.upsert_cloud_connection(&mut connection)
                    .expect("cloud download persists");
            })
        });

        barrier.wait();
        for handle in handles {
            handle.join().expect("download thread joins");
        }

        assert_eq!(1, repo.count().expect("connection count"));
        assert!(
            repo.get_by_cloud_id("shared-cloud-id")
                .expect("connection read")
                .is_some()
        );
    }
}

pub fn init(cx: &mut App) {
    let storage_state = cx.global::<GlobalStorageState>();
    let storage = storage_state.storage.clone();

    let conn = storage.connection();
    let conn_repo = ConnectionRepository::new(conn.clone());
    let workspace_repo = WorkspaceRepository::new(conn.clone());
    let quick_cmd_repo = QuickCommandRepository::new(conn.clone());
    let sftp_favorite_path_repo = SftpFavoritePathRepository::new(conn.clone());
    let terminal_command_history_repo = TerminalCommandHistoryRepository::new(conn.clone());
    let pending_deletion_repo = PendingCloudDeletionRepository::new(conn.clone());
    let team_key_cache_repo = TeamKeyCacheRepository::new(conn.clone());
    let team_membership_cache_repo = TeamMembershipCacheRepository::new(conn.clone());
    let personal_conflict_repo =
        crate::cloud_sync::personal::PersonalSyncConflictRepository::new(conn.clone());
    let personal_status_repo =
        crate::cloud_sync::personal::PersonalSyncStatusRepository::new(conn.clone());

    storage.register(workspace_repo);
    storage.register(conn_repo);
    storage.register(quick_cmd_repo);
    storage.register(sftp_favorite_path_repo);
    storage.register(terminal_command_history_repo);
    storage.register(pending_deletion_repo);
    storage.register(team_key_cache_repo);
    storage.register(team_membership_cache_repo);
    storage.register(personal_conflict_repo);
    storage.register(personal_status_repo);
}
