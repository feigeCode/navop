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
            id: None,
            name: "prod".to_string(),
            connection_type: ConnectionType::SshSftp,
            params: "{}".to_string(),
            workspace_id: None,
            selected_databases: None,
            remark: None,
            sync_enabled: true,
            cloud_id: None,
            last_synced_at: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
        };

        assert_eq!("name:prod", sftp_favorite_connection_key(&conn));

        conn.cloud_id = Some("cloud-1".to_string());
        assert_eq!("cloud:cloud-1", sftp_favorite_connection_key(&conn));

        conn.id = Some(42);
        assert_eq!("local:42", sftp_favorite_connection_key(&conn));
    }
}
