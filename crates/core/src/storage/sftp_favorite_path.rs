//! SFTP 常用远程目录存储。

use anyhow::Result;
use rusqlite::params;

use crate::storage::connection::SqliteConnection;
use crate::storage::manager::now;
use crate::storage::models::StoredConnection;

pub const MAX_SFTP_FAVORITE_PATHS: usize = 20;

#[derive(Clone)]
pub struct SftpFavoritePathRepository {
    conn: SqliteConnection,
}

pub fn sftp_favorite_connection_key(connection: &StoredConnection) -> String {
    if let Some(id) = connection.id {
        format!("local:{id}")
    } else if let Some(cloud_id) = connection.cloud_id.as_deref() {
        format!("cloud:{cloud_id}")
    } else {
        format!("name:{}", connection.name)
    }
}

impl SftpFavoritePathRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn list_paths(&self, connection_key: &str) -> Result<Vec<String>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT path FROM sftp_favorite_paths
                 WHERE connection_key = ?1
                 ORDER BY sort_order ASC, created_at ASC, id ASC",
            )?;
            let rows = stmt.query_map(params![connection_key], |row| row.get(0))?;
            let mut paths = Vec::new();
            for row in rows {
                paths.push(row?);
            }
            Ok(paths)
        })
    }

    pub fn add_path(
        &self,
        connection_id: Option<i64>,
        connection_key: &str,
        path: &str,
    ) -> Result<bool> {
        let Some(path) = normalize_sftp_favorite_path(path) else {
            return Ok(false);
        };

        if self.is_favorite(connection_key, &path)? {
            return Ok(false);
        }

        let ts = now();
        self.conn.with_connection(|conn| {
            let sort_order = next_sort_order(conn, connection_key)?;
            conn.execute(
                "INSERT INTO sftp_favorite_paths
                 (connection_id, connection_key, path, sort_order, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![connection_id, connection_key, path, sort_order, ts, ts],
            )?;
            prune_old_paths(conn, connection_key)?;
            Ok(true)
        })
    }

    pub fn remove_path(&self, connection_key: &str, path: &str) -> Result<bool> {
        let Some(path) = normalize_sftp_favorite_path(path) else {
            return Ok(false);
        };
        self.conn.with_connection(|conn| {
            let changed = conn.execute(
                "DELETE FROM sftp_favorite_paths WHERE connection_key = ?1 AND path = ?2",
                params![connection_key, path],
            )?;
            Ok(changed > 0)
        })
    }

    pub fn update_path(
        &self,
        connection_key: &str,
        old_path: &str,
        new_path: &str,
    ) -> Result<bool> {
        let Some(old_path) = normalize_sftp_favorite_path(old_path) else {
            return Ok(false);
        };
        let Some(new_path) = normalize_sftp_favorite_path(new_path) else {
            return Ok(false);
        };
        if old_path == new_path || self.is_favorite(connection_key, &new_path)? {
            return Ok(false);
        }

        let ts = now();
        self.conn.with_connection(|conn| {
            let changed = conn.execute(
                "UPDATE sftp_favorite_paths
                 SET path = ?3, updated_at = ?4
                 WHERE connection_key = ?1 AND path = ?2",
                params![connection_key, old_path, new_path, ts],
            )?;
            Ok(changed > 0)
        })
    }

    pub fn is_favorite(&self, connection_key: &str, path: &str) -> Result<bool> {
        let Some(path) = normalize_sftp_favorite_path(path) else {
            return Ok(false);
        };
        self.conn.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sftp_favorite_paths
                    WHERE connection_key = ?1 AND path = ?2
                 )",
                params![connection_key, path],
                |row| row.get(0),
            )?;
            Ok(exists == 1)
        })
    }
}

/// SFTP 收藏行的完整表示
///
/// 云同步需要按行枚举与按 id 写入，而既有的 `list_paths` 只按连接键返回路径字符串，
/// 无法支撑同步。这里补齐按行读写的入口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpFavoriteRow {
    pub id: i64,
    pub connection_id: Option<i64>,
    pub connection_key: String,
    pub path: String,
    pub sort_order: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

impl SftpFavoritePathRepository {
    /// 枚举全部收藏（跨连接），供云同步使用
    pub fn list_all(&self) -> Result<Vec<SftpFavoriteRow>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, connection_id, connection_key, path, sort_order, created_at, updated_at
                 FROM sftp_favorite_paths
                 ORDER BY connection_key ASC, sort_order ASC, id ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(SftpFavoriteRow {
                    id: row.get(0)?,
                    connection_id: row.get(1)?,
                    connection_key: row.get(2)?,
                    path: row.get(3)?,
                    sort_order: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        })
    }

    pub fn get_row(&self, id: i64) -> Result<Option<SftpFavoriteRow>> {
        self.list_all()
            .map(|rows| rows.into_iter().find(|row| row.id == id))
    }

    /// 按行插入（同步侧已解析好 connection_key），返回新 id
    pub fn insert_row(&self, row: &SftpFavoriteRow) -> Result<i64> {
        let Some(path) = normalize_sftp_favorite_path(&row.path) else {
            return Ok(0);
        };
        let ts = now();
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO sftp_favorite_paths
                 (connection_id, connection_key, path, sort_order, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    row.connection_id,
                    row.connection_key,
                    path,
                    row.sort_order,
                    ts,
                    ts
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    pub fn update_row(&self, row: &SftpFavoriteRow) -> Result<()> {
        let Some(path) = normalize_sftp_favorite_path(&row.path) else {
            return Ok(());
        };
        let ts = now();
        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE sftp_favorite_paths
                 SET connection_id = ?2,
                     connection_key = ?3,
                     path = ?4,
                     sort_order = ?5,
                     updated_at = ?6
                 WHERE id = ?1",
                params![
                    row.id,
                    row.connection_id,
                    row.connection_key,
                    path,
                    row.sort_order,
                    ts
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_row(&self, id: i64) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute("DELETE FROM sftp_favorite_paths WHERE id = ?1", params![id])?;
            Ok(())
        })
    }
}

pub fn normalize_sftp_favorite_path(path: &str) -> Option<String> {
    let mut path = path.trim().to_string();
    if path.is_empty() {
        return None;
    }

    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }

    Some(path)
}

fn next_sort_order(conn: &rusqlite::Connection, connection_key: &str) -> Result<i32> {
    let max: Option<i32> = conn.query_row(
        "SELECT MAX(sort_order) FROM sftp_favorite_paths WHERE connection_key = ?1",
        params![connection_key],
        |row| row.get(0),
    )?;
    Ok(max.unwrap_or(0) + 1)
}

fn prune_old_paths(conn: &rusqlite::Connection, connection_key: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM sftp_favorite_paths
         WHERE connection_key = ?1
           AND id NOT IN (
               SELECT id FROM sftp_favorite_paths
               WHERE connection_key = ?1
               ORDER BY sort_order DESC, created_at DESC, id DESC
               LIMIT ?2
           )",
        params![connection_key, MAX_SFTP_FAVORITE_PATHS as i64],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_SFTP_FAVORITE_PATHS, SftpFavoritePathRepository, sftp_favorite_connection_key,
    };
    use crate::storage::connection::SqliteConnection;
    use crate::storage::migration::run_migrations;
    use crate::storage::{ConnectionType, StoredConnection};
    use std::sync::atomic::{AtomicU64, Ordering};

    static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_repository() -> SftpFavoritePathRepository {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let db_path = std::env::temp_dir().join(format!(
            "onetcli-sftp-favorite-paths-{}-{unique}-{counter}.db",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&db_path);
        let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
        conn.with_connection(|conn| run_migrations(conn))
            .expect("run migrations");
        SftpFavoritePathRepository::new(conn)
    }

    #[test]
    fn sftp_favorite_paths_are_normalized_deduplicated_and_scoped() {
        let repo = test_repository();

        assert!(repo.add_path(None, "local:1", " /srv/app/ ").unwrap());
        assert!(!repo.add_path(None, "local:1", "/srv/app").unwrap());
        assert!(repo.add_path(None, "local:1", "/").unwrap());
        assert!(repo.add_path(None, "local:2", "/srv/app").unwrap());

        assert_eq!(
            vec!["/srv/app".to_string(), "/".to_string()],
            repo.list_paths("local:1").unwrap()
        );
        assert_eq!(
            vec!["/srv/app".to_string()],
            repo.list_paths("local:2").unwrap()
        );
        assert!(repo.is_favorite("local:1", "/srv/app/").unwrap());
        assert!(!repo.is_favorite("local:1", "   ").unwrap());
    }

    #[test]
    fn sftp_favorite_paths_can_be_removed() {
        let repo = test_repository();

        repo.add_path(None, "local:1", "/srv/app").unwrap();
        repo.add_path(None, "local:1", "/opt/data").unwrap();

        assert!(repo.remove_path("local:1", "/srv/app/").unwrap());
        assert!(!repo.remove_path("local:1", "/missing").unwrap());

        assert_eq!(
            vec!["/opt/data".to_string()],
            repo.list_paths("local:1").unwrap()
        );
    }

    #[test]
    fn sftp_favorite_paths_can_be_updated_without_duplicates() {
        let repo = test_repository();

        repo.add_path(None, "local:1", "/srv/app").unwrap();
        repo.add_path(None, "local:1", "/opt/data").unwrap();
        repo.add_path(None, "local:2", "/srv/app").unwrap();

        assert!(
            repo.update_path("local:1", "/srv/app/", " /srv/api/ ")
                .unwrap()
        );
        assert!(
            !repo
                .update_path("local:1", "/srv/api", "/opt/data")
                .unwrap()
        );
        assert!(!repo.update_path("local:1", "/srv/api", "   ").unwrap());

        assert_eq!(
            vec!["/srv/api".to_string(), "/opt/data".to_string()],
            repo.list_paths("local:1").unwrap()
        );
        assert_eq!(
            vec!["/srv/app".to_string()],
            repo.list_paths("local:2").unwrap()
        );
    }

    #[test]
    fn sftp_favorite_paths_keep_the_most_recent_limit() {
        let repo = test_repository();

        for index in 0..(MAX_SFTP_FAVORITE_PATHS + 2) {
            repo.add_path(None, "local:1", &format!("/path/{index}"))
                .unwrap();
        }

        let paths = repo.list_paths("local:1").unwrap();

        assert_eq!(MAX_SFTP_FAVORITE_PATHS, paths.len());
        assert_eq!("/path/2", paths.first().unwrap());
        assert_eq!(
            format!("/path/{}", MAX_SFTP_FAVORITE_PATHS + 1),
            *paths.last().unwrap()
        );
    }

    #[test]
    fn favorite_connection_key_prefers_local_id_then_cloud_id_then_name() {
        let mut conn = StoredConnection {
            ai_access_level: Default::default(),
            id: None,
            credential_revision: None,
            name: "prod".to_string(),
            connection_type: ConnectionType::SshSftp,
            params: "{}".to_string(),
            workspace_id: None,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
            preferred_open_mode: None,
        };

        assert_eq!("name:prod", sftp_favorite_connection_key(&conn));

        conn.cloud_id = Some("cloud-1".to_string());
        assert_eq!("cloud:cloud-1", sftp_favorite_connection_key(&conn));

        conn.id = Some(42);
        assert_eq!("local:42", sftp_favorite_connection_key(&conn));
    }
}
