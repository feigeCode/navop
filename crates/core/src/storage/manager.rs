use super::connection::SqliteConnection;
use super::migration::run_migrations;
use anyhow::Result;
use dashmap::DashMap;
use gpui::{App, Global};
use std::any::{Any, TypeId};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct StorageManager {
    conn: SqliteConnection,
    repositories: Arc<DashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

pub struct GlobalStorageState {
    pub storage: StorageManager,
}

impl Global for GlobalStorageState {}

impl Clone for GlobalStorageState {
    fn clone(&self) -> Self {
        GlobalStorageState {
            storage: self.storage.clone(),
        }
    }
}

impl StorageManager {
    pub fn new() -> Result<Self> {
        let db_path = get_db_path()?;
        std::fs::create_dir_all(db_path.parent().unwrap())?;
        let conn = SqliteConnection::open(&db_path)?;

        conn.with_connection(|c| {
            run_migrations(c)?;
            Ok(())
        })?;

        let manager = Self {
            conn,
            repositories: Arc::new(DashMap::new()),
        };
        Ok(manager)
    }

    pub fn new_with_connection(conn: SqliteConnection) -> Self {
        Self {
            conn,
            repositories: Arc::new(DashMap::new()),
        }
    }

    pub fn connection(&self) -> SqliteConnection {
        self.conn.clone()
    }

    pub fn register<R>(&self, repo: R)
    where
        R: 'static + Send + Sync,
    {
        let type_id = TypeId::of::<R>();
        self.repositories.insert(type_id, Arc::new(repo));
    }

    pub fn get<R>(&self) -> Option<Arc<R>>
    where
        R: 'static + Send + Sync,
    {
        self.repositories
            .get(&TypeId::of::<R>())
            .and_then(|v| v.clone().downcast::<R>().ok())
    }
}

impl Clone for StorageManager {
    fn clone(&self) -> Self {
        Self {
            conn: self.conn.clone(),
            repositories: Arc::clone(&self.repositories),
        }
    }
}

pub fn get_db_path() -> Result<PathBuf> {
    let config_dir = get_config_dir()?;
    Ok(config_dir.join(crate::app_dirs::DATABASE_FILE_NAME))
}

pub fn get_config_dir() -> Result<PathBuf> {
    crate::app_dirs::config_dir()
}

pub fn get_download_dir() -> Option<PathBuf> {
    dirs::download_dir()
}

pub fn get_queries_dir() -> Result<PathBuf> {
    let config_dir = get_config_dir()?;
    // 设置里给了 SQL 查询目录就整体切过去（issue #112 第 3 点），否则用配置目录下的 queries。
    let configured = crate::settings::AppSettings::peek_sql_queries_dir();
    let queries_dir = resolve_queries_dir(&config_dir, &configured);
    if !queries_dir.exists() {
        std::fs::create_dir_all(&queries_dir)?;
    }
    Ok(queries_dir)
}

/// 解析查询目录：设置为空（或只有空白）时回退到 `config_dir/queries`。
pub(crate) fn resolve_queries_dir(config_dir: &Path, configured: &str) -> PathBuf {
    let configured = configured.trim();
    if configured.is_empty() {
        return config_dir.join("queries");
    }
    PathBuf::from(configured)
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间不应早于 UNIX 纪元")
        .as_secs() as i64
}

pub fn init(cx: &mut App) -> Result<()> {
    let storage = StorageManager::new()?;
    cx.set_global(GlobalStorageState { storage });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::resolve_queries_dir;
    use std::path::{Path, PathBuf};

    #[test]
    fn queries_dir_falls_back_to_config_dir_when_unset() {
        let config_dir = Path::new("/tmp/navop-config");
        assert_eq!(
            resolve_queries_dir(config_dir, ""),
            PathBuf::from("/tmp/navop-config/queries")
        );
        assert_eq!(
            resolve_queries_dir(config_dir, "   "),
            PathBuf::from("/tmp/navop-config/queries")
        );
    }

    #[test]
    fn queries_dir_uses_the_configured_root() {
        let config_dir = Path::new("/tmp/navop-config");
        assert_eq!(
            resolve_queries_dir(config_dir, " /data/navop/queries "),
            PathBuf::from("/data/navop/queries")
        );
    }
}
