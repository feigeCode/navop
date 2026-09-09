use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const DEFAULT_POOL_SIZE: usize = 4;

struct PoolInner {
    connections: Vec<Connection>,
    path: PathBuf,
}

pub struct SqliteConnection {
    inner: Arc<Mutex<PoolInner>>,
}

impl Clone for SqliteConnection {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

pub struct PooledConnection<'a> {
    conn: Option<Connection>,
    pool: &'a SqliteConnection,
}

impl Deref for PooledConnection<'_> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        self.conn.as_ref().expect("connection already returned")
    }
}

impl DerefMut for PooledConnection<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.conn.as_mut().expect("connection already returned")
    }
}

impl Drop for PooledConnection<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            if let Ok(mut guard) = self.pool.inner.lock() {
                guard.connections.push(conn);
            }
        }
    }
}

fn create_connection(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )?;

    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;

    // WAL 需要创建并写入 -wal/-shm 文件。在杀毒软件实时防护、同步盘或不支持
    // 共享内存的文件系统上，这一步可能返回 "disk I/O error"（SQLITE_IOERR）。
    // 此处按尽力而为处理：失败时回退到 SQLite 默认的 DELETE journal mode，
    // 连接仍然可用，避免整个存储初始化失败。
    if let Err(error) = conn.execute_batch("PRAGMA journal_mode = WAL;") {
        tracing::warn!(
            %error,
            "failed to enable SQLite WAL journal mode; falling back to the default journal mode"
        );
    }

    Ok(conn)
}

impl SqliteConnection {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_pool_size(path, DEFAULT_POOL_SIZE)
    }

    pub fn open_with_pool_size(path: impl AsRef<Path>, pool_size: usize) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut connections = Vec::with_capacity(pool_size);

        for _ in 0..pool_size {
            connections.push(create_connection(&path)?);
        }

        Ok(Self {
            inner: Arc::new(Mutex::new(PoolInner { connections, path })),
        })
    }

    fn get_connection(&self) -> Result<PooledConnection<'_>> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let conn = if let Some(conn) = guard.connections.pop() {
            conn
        } else {
            create_connection(&guard.path)?
        };
        drop(guard);

        Ok(PooledConnection {
            conn: Some(conn),
            pool: self,
        })
    }

    pub fn with_connection<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self.get_connection()?;
        f(&conn)
    }

    pub fn with_connection_mut<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Connection) -> Result<T>,
    {
        let mut conn = self.get_connection()?;
        f(&mut conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pooled_connection_opens_and_persists_writes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("pooled.db");
        let pool = SqliteConnection::open(&path).expect("open pooled connection");

        pool.with_connection(|conn| {
            conn.execute_batch("CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('ok');")?;
            let value: String = conn.query_row("SELECT v FROM t", [], |row| row.get(0))?;
            assert_eq!("ok", value);
            Ok(())
        })
        .expect("write and read through the pool");
    }

    #[test]
    fn journal_mode_is_wal_or_falls_back_to_delete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("journal.db");
        let pool = SqliteConnection::open(&path).expect("open connection");

        pool.with_connection(|conn| {
            let mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
            assert!(
                mode.eq_ignore_ascii_case("wal") || mode.eq_ignore_ascii_case("delete"),
                "journal mode must be WAL or a supported fallback, got {mode:?}"
            );
            Ok(())
        })
        .expect("query journal mode");
    }

    #[cfg(unix)]
    #[test]
    fn wal_unavailable_falls_back_without_failing_the_connection() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("readonly-dir.db");
        std::fs::write(&path, b"").expect("pre-create db file");
        let original = std::fs::metadata(&path).expect("db metadata").permissions();
        let writable = original.mode() & 0o200 != 0;
        if !writable {
            return;
        }

        // 让目录只读：数据库文件可以打开，但 WAL 需要在目录里创建 -wal/-shm 文件，
        // 这一步会失败并返回 SQLITE_IOERR（正是线上 "disk I/O error" 的来源）。
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500))
            .expect("make directory read-only");

        let pool = SqliteConnection::open(&path).expect("open must fall back instead of failing");

        pool.with_connection(|conn| {
            let mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
            assert_eq!("delete", mode.to_ascii_lowercase());
            Ok(())
        })
        .expect("connection usable without WAL");
    }
}
