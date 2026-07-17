use crate::onetcli_app::{GlobalHomePage, GlobalTabContainer};
use anyhow::{Context as _, Result, bail};
use gpui::{App, AppContext, Window};
use gpui_component::{WindowExt, notification::Notification};
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::storage::{
    ConnectionRepository, ConnectionType, DatabaseType, DbConnectionConfig, GlobalStorageState,
    StoredConnection, traits::Repository,
};
use one_core::tab_container::{TabItem, TabOpenMode};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) enum FileOpenInput {
    Path(PathBuf),
    Url(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OpenFileKind {
    Database(DatabaseType),
    Markdown,
}

struct PreparedOpenFile {
    path: PathBuf,
    kind: OpenFileKind,
}

struct FileOpenNotification;

struct PersistedFileDatabase {
    connection: StoredConnection,
    created: bool,
}

trait FileDatabaseRepository {
    fn list(&self) -> Result<Vec<StoredConnection>>;
    fn insert(&self, connection: &mut StoredConnection) -> Result<i64>;
}

impl FileDatabaseRepository for ConnectionRepository {
    fn list(&self) -> Result<Vec<StoredConnection>> {
        <Self as Repository>::list(self)
    }

    fn insert(&self, connection: &mut StoredConnection) -> Result<i64> {
        <Self as Repository>::insert(self, connection)
    }
}

pub(crate) fn open_input(input: FileOpenInput, window: &mut Window, cx: &mut App) {
    if let Err(error) = try_open_input(input, window, cx) {
        tracing::warn!(error = %error, "failed to open associated file");
        window.push_notification(
            Notification::error(format!("无法打开文件：{error:#}"))
                .id::<FileOpenNotification>()
                .autohide(true),
            cx,
        );
    }
}

fn try_open_input(input: FileOpenInput, window: &mut Window, cx: &mut App) -> Result<()> {
    let Some(prepared) = prepare_open_file(input)? else {
        return Ok(());
    };
    window.activate_window();
    match prepared.kind {
        OpenFileKind::Database(database_type) => {
            open_database_file(prepared.path, database_type, window, cx)
        }
        OpenFileKind::Markdown => {
            open_markdown_file(prepared.path, window, cx);
            Ok(())
        }
    }
}

fn prepare_open_file(input: FileOpenInput) -> Result<Option<PreparedOpenFile>> {
    let path = match input {
        FileOpenInput::Path(path) => path,
        FileOpenInput::Url(url) => local_path_from_file_url(&url)?,
    };
    let Some(kind) = classify_supported_path(&path) else {
        return Ok(None);
    };
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .context("resolve current directory for associated file")?
            .join(path)
    };
    let metadata = std::fs::metadata(&absolute)
        .with_context(|| format!("读取文件信息失败: {}", absolute.display()))?;
    if !metadata.is_file() {
        bail!("路径不是文件: {}", absolute.display());
    }
    let path = match kind {
        OpenFileKind::Database(_) => absolute
            .canonicalize()
            .with_context(|| format!("规范化数据库文件路径失败: {}", absolute.display()))?,
        OpenFileKind::Markdown => absolute,
    };
    Ok(Some(PreparedOpenFile { path, kind }))
}

fn classify_supported_path(path: &Path) -> Option<OpenFileKind> {
    let extension = path.extension()?.to_str()?;
    if extension.eq_ignore_ascii_case("db") {
        Some(OpenFileKind::Database(DatabaseType::SQLite))
    } else if extension.eq_ignore_ascii_case("duckdb") {
        Some(OpenFileKind::Database(DatabaseType::DuckDB))
    } else if extension.eq_ignore_ascii_case("md") {
        Some(OpenFileKind::Markdown)
    } else {
        None
    }
}

fn local_path_from_file_url(raw: &str) -> Result<PathBuf> {
    let url = url::Url::parse(raw).with_context(|| format!("解析文件 URL 失败: {raw}"))?;
    if url.scheme() != "file" {
        bail!("不是本地文件 URL: {raw}");
    }
    url.to_file_path()
        .map_err(|_| anyhow::anyhow!("无法将文件 URL 转换成本地路径: {raw}"))
}

fn open_database_file(
    path: PathBuf,
    database_type: DatabaseType,
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    let repository = cx
        .global::<GlobalStorageState>()
        .storage
        .get::<ConnectionRepository>()
        .context("ConnectionRepository not found")?;
    let persisted = persist_file_database(repository.as_ref(), &path, database_type)?;
    if persisted.created
        && let Some(notifier) = get_notifier(cx)
    {
        let connection = persisted.connection.clone();
        notifier.update(cx, |_, cx| {
            cx.emit(ConnectionDataEvent::ConnectionCreated { connection });
        });
    }
    let home = cx
        .try_global::<GlobalHomePage>()
        .map(|global| global.home_page.clone())
        .context("home page is not ready")?;
    let connection = persisted.connection;
    window.defer(cx, move |window, cx| {
        home.update(cx, |home, cx| {
            extension_runtime::database_driver_install::open_database_connection_with_driver_guard(
                home,
                connection,
                None,
                TabOpenMode::Activate,
                window,
                cx,
            );
        });
    });
    Ok(())
}

fn open_markdown_file(path: PathBuf, window: &mut Window, cx: &mut App) {
    let Some(tab_container) = cx
        .try_global::<GlobalTabContainer>()
        .map(|global| global.primary_pane())
    else {
        return;
    };
    let tab_id = format!("markdown-file-{}", stable_file_key(&path));
    window.defer(cx, move |window, cx| {
        let tab_id_for_create = tab_id.clone();
        tab_container.update(cx, |tabs, cx| {
            tabs.activate_or_add_tab_lazy(
                tab_id,
                move |window, cx| {
                    let notes =
                        cx.new(|cx| notes::NotesView::new_for_markdown_file(path, window, cx));
                    TabItem::new(tab_id_for_create, "file-open", notes)
                },
                window,
                cx,
            );
        });
    });
}

fn persist_file_database(
    repository: &impl FileDatabaseRepository,
    path: &Path,
    database_type: DatabaseType,
) -> Result<PersistedFileDatabase> {
    if let Some(connection) = repository
        .list()?
        .into_iter()
        .find(|connection| file_database_matches(connection, path, &database_type))
    {
        return Ok(PersistedFileDatabase {
            connection,
            created: false,
        });
    }

    let mut connection = database_connection_for_file(path, database_type);
    repository.insert(&mut connection)?;
    Ok(PersistedFileDatabase {
        connection,
        created: true,
    })
}

fn file_database_matches(
    connection: &StoredConnection,
    path: &Path,
    database_type: &DatabaseType,
) -> bool {
    if connection.connection_type != ConnectionType::Database {
        return false;
    }
    let Ok(config) = connection.to_db_connection() else {
        return false;
    };
    config.database_type == *database_type
        && paths_refer_to_same_file(Path::new(&config.host), path)
}

fn paths_refer_to_same_file(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    if cfg!(target_os = "windows") {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

fn database_connection_for_file(path: &Path, database_type: DatabaseType) -> StoredConnection {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let config = DbConnectionConfig {
        id: String::new(),
        database_type,
        name: name.clone(),
        host: path.to_string_lossy().into_owned(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        extra_params: Default::default(),
    };
    let mut connection = StoredConnection::new_database(name, config, None);
    // Local file paths are machine-specific, so persistence should remain local.
    connection.sync_enabled = false;
    connection
}

fn stable_file_key(path: &Path) -> String {
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use one_core::storage::{DatabaseType, StoredConnection};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    #[test]
    fn supported_extensions_route_to_the_expected_editor() {
        assert_eq!(
            Some(OpenFileKind::Database(DatabaseType::SQLite)),
            classify_supported_path(Path::new("orders.db"))
        );
        assert_eq!(
            Some(OpenFileKind::Database(DatabaseType::DuckDB)),
            classify_supported_path(Path::new("warehouse.DUCKDB"))
        );
        assert_eq!(
            Some(OpenFileKind::Markdown),
            classify_supported_path(Path::new("README.Md"))
        );
        assert_eq!(None, classify_supported_path(Path::new("notes.txt")));
    }

    #[test]
    fn file_urls_are_decoded_to_local_paths() {
        assert_eq!(
            PathBuf::from("/tmp/Navop notes/README.md"),
            local_path_from_file_url("file:///tmp/Navop%20notes/README.md").unwrap()
        );
        assert!(local_path_from_file_url("https://example.com/README.md").is_err());
    }

    #[test]
    fn file_database_connections_are_persisted_once_and_reused() -> Result<()> {
        let repository = FakeRepository::default();

        let first = persist_file_database(
            &repository,
            Path::new("/tmp/orders.db"),
            DatabaseType::SQLite,
        )?;
        let reopened = persist_file_database(
            &repository,
            Path::new("/tmp/orders.db"),
            DatabaseType::SQLite,
        )?;

        assert!(first.created);
        assert!(!reopened.created);
        assert_eq!(first.connection.id, reopened.connection.id);
        assert_eq!(1, repository.connections.lock().unwrap().len());
        assert_eq!("/tmp/orders.db", first.connection.to_db_connection()?.host);
        assert_eq!("orders.db", first.connection.name);
        assert!(!first.connection.sync_enabled);
        Ok(())
    }

    #[test]
    fn different_database_files_create_distinct_saved_connections() -> Result<()> {
        let repository = FakeRepository::default();

        let sqlite = persist_file_database(
            &repository,
            Path::new("/tmp/orders.db"),
            DatabaseType::SQLite,
        )?;
        let duckdb = persist_file_database(
            &repository,
            Path::new("/tmp/warehouse.duckdb"),
            DatabaseType::DuckDB,
        )?;

        assert_ne!(sqlite.connection.id, duckdb.connection.id);
        assert_eq!(2, repository.connections.lock().unwrap().len());
        Ok(())
    }

    #[test]
    fn stable_file_keys_deduplicate_the_same_path() {
        assert_eq!(
            stable_file_key(Path::new("/tmp/README.md")),
            stable_file_key(Path::new("/tmp/README.md"))
        );
        assert_ne!(
            stable_file_key(Path::new("/tmp/README.md")),
            stable_file_key(Path::new("/tmp/OTHER.md"))
        );
    }

    #[derive(Default)]
    struct FakeRepository {
        connections: Mutex<Vec<StoredConnection>>,
    }

    impl FileDatabaseRepository for FakeRepository {
        fn list(&self) -> Result<Vec<StoredConnection>> {
            Ok(self.connections.lock().unwrap().clone())
        }

        fn insert(&self, connection: &mut StoredConnection) -> Result<i64> {
            let mut connections = self.connections.lock().unwrap();
            let id = connections.len() as i64 + 1;
            connection.id = Some(id);
            connections.push(connection.clone());
            Ok(id)
        }
    }
}
