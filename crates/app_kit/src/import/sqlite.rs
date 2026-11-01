//! 从全量产品（或同构）sqlite 库只读读取连接。
//!
//! 只读打开，避免误改用户现有数据；列用 `PRAGMA table_info` 自适应，方便跨
//! 版本读取（老库缺 `workspace_id`、`remark` 等列也能导入）。

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use chrono::Utc;
use rusqlite::{Connection, OpenFlags};

use super::bundle::{BUNDLE_VERSION, BundleConnection, BundleWorkspace, ConnectionBundle};
/// 三个产品共用的数据库文件名（与全量产品 `get_db_path` 同源，仅测试断言用）。
#[cfg(test)]
use one_core::app_dirs::DATABASE_FILE_NAME as DB_FILE_NAME;

/// sqlite 文件头魔数，用来区分 `one-hub.db` 与迁移包 JSON。
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// 全量产品的 app id，写入迁移包便于追溯来源。
const SOURCE_APP_ID: &str = "navop";

/// 全量产品可能的数据目录下的数据库文件；按优先级排序并去重。
///
/// 目录规则统一由 [`one_core::app_dirs::full_product_database_candidates`] 提供，
/// 与全量产品自己的 `config_dir` 解析保持同一套名字（`navop` / 历史 `one-hub` /
/// 开发 `~/.navop-dev`）：独立产品的目录名已被换成 `.navop-data-dev` 之类，不能
/// 按当前身份解析。
pub fn full_product_candidates() -> Vec<PathBuf> {
    one_core::app_dirs::full_product_database_candidates()
}

/// 探测本机可用于迁移的全量产品数据库。
///
/// 优先返回真正带连接的那个库：全量产品目录里可能残留刚被独立产品初始化出来
/// 的空库（0 条连接），若只按「能读通」判断就会导入 0 条连接。多个库都非空时
/// 取最近修改的那个（用户最后在用的数据），都为空时才退回第一个可读库。
pub fn detect_full_product() -> Option<PathBuf> {
    pick_candidate(full_product_candidates())
}

fn pick_candidate(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    let mut freshest: Option<(std::time::SystemTime, PathBuf)> = None;
    let mut fallback = None;
    for path in candidates {
        if !is_readable_db(&path) {
            continue;
        }
        let Ok(bundle) = read_bundle_from_sqlite(&path) else {
            continue;
        };
        if bundle.connections.is_empty() {
            fallback.get_or_insert(path);
            continue;
        }
        let modified = database_modified(&path);
        if freshest.as_ref().is_none_or(|(best, _)| modified > *best) {
            freshest = Some((modified, path));
        }
    }
    freshest.map(|(_, path)| path).or(fallback)
}

/// 库文件（含 WAL 旁文件）的最后修改时间。
///
/// WAL 模式下新写入可能只落在 `one-hub.db-wal`，主库文件的 mtime 会偏旧。
fn database_modified(path: &Path) -> std::time::SystemTime {
    let mut newest = std::time::SystemTime::UNIX_EPOCH;
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    for candidate in [path.to_path_buf(), PathBuf::from(wal)] {
        if let Ok(modified) = std::fs::metadata(&candidate).and_then(|meta| meta.modified())
            && modified > newest
        {
            newest = modified;
        }
    }
    newest
}

fn is_readable_db(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.len() > 0)
        .unwrap_or(false)
}

/// 文件头是否为 sqlite 数据库。
///
/// 「选择文件」同时接受 `one-hub.db` 与迁移包 JSON，靠文件头区分，比看扩展名可靠
///（用户常把 `one-hub.db` 改名或没有扩展名）。
pub fn is_sqlite_database(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 16];
    file.read_exact(&mut header).is_ok() && &header == SQLITE_MAGIC
}

/// 只读读取数据库里的连接与工作区，组装成迁移包。
pub fn read_bundle_from_sqlite(path: &Path) -> Result<ConnectionBundle> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("无法只读打开数据库：{}", path.display()))?;
    let (workspaces, workspace_names) = read_workspaces(&connection)?;
    let connections = read_connections(&connection, &workspace_names)?;
    Ok(ConnectionBundle {
        version: BUNDLE_VERSION,
        exported_at: Utc::now().timestamp(),
        app_id: Some(SOURCE_APP_ID.to_string()),
        workspaces,
        connections,
    })
}

fn read_workspaces(conn: &Connection) -> Result<(Vec<BundleWorkspace>, HashMap<i64, String>)> {
    let columns = table_columns(conn, "workspaces")?;
    if !columns.iter().any(|column| column == "name") {
        return Ok((Vec::new(), HashMap::new()));
    }
    let sql = format!(
        "SELECT {}, {}, {}, {} FROM workspaces ORDER BY {}",
        column_expr(&columns, "id", "rowid"),
        column_expr(&columns, "name", "NULL"),
        column_expr(&columns, "color", "NULL"),
        column_expr(&columns, "icon", "NULL"),
        column_expr(&columns, "id", "rowid"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut workspaces = Vec::new();
    let mut names = HashMap::new();
    for row in rows {
        let (id, name, color, icon) = row?;
        let Some(name) = non_empty(name) else {
            continue;
        };
        names.insert(id, name.clone());
        workspaces.push(BundleWorkspace { name, color, icon });
    }
    Ok((workspaces, names))
}

fn read_connections(
    conn: &Connection,
    workspace_names: &HashMap<i64, String>,
) -> Result<Vec<BundleConnection>> {
    let columns = table_columns(conn, "connections")?;
    anyhow::ensure!(!columns.is_empty(), "数据库缺少 connections 表");
    let sql = format!(
        "SELECT {}, {}, {}, {}, {}, {}, {} FROM connections ORDER BY {}",
        column_expr(&columns, "name", "NULL"),
        column_expr(&columns, "connection_type", "NULL"),
        column_expr(&columns, "params", "NULL"),
        column_expr(&columns, "workspace_id", "NULL"),
        column_expr(&columns, "selected_databases", "NULL"),
        column_expr(&columns, "remark", "NULL"),
        column_expr(&columns, "sync_enabled", "1"),
        column_expr(&columns, "id", "rowid"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(RawConnection {
            name: row.get(0)?,
            connection_type: row.get(1)?,
            params: row.get(2)?,
            workspace_id: row.get(3)?,
            selected_databases: row.get(4)?,
            remark: row.get(5)?,
            sync_enabled: row.get(6)?,
        })
    })?;
    let mut connections = Vec::new();
    for row in rows {
        let raw = row?;
        connections.push(BundleConnection {
            name: non_empty(raw.name).unwrap_or_else(|| "未命名连接".to_string()),
            connection_type: raw.connection_type.unwrap_or_default(),
            params: raw.params.unwrap_or_default(),
            workspace: raw
                .workspace_id
                .and_then(|id| workspace_names.get(&id).cloned()),
            remark: non_empty(raw.remark),
            selected_databases: non_empty(raw.selected_databases),
            sync_enabled: raw.sync_enabled.unwrap_or(1) != 0,
        });
    }
    Ok(connections)
}

struct RawConnection {
    name: Option<String>,
    connection_type: Option<String>,
    params: Option<String>,
    workspace_id: Option<i64>,
    selected_databases: Option<String>,
    remark: Option<String>,
    sync_enabled: Option<i64>,
}

/// 读取表的列名；表不存在时返回空列表。
fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// 列存在则引用该列，否则用兜底表达式（`NULL` / `1` / `rowid`）。
fn column_expr(columns: &[String], name: &str, fallback: &str) -> String {
    if columns.iter().any(|column| column == name) {
        format!("\"{name}\"")
    } else {
        fallback.to_string()
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(path: &Path) -> Connection {
        Connection::open(path).expect("打开临时库失败")
    }

    struct TempDb(PathBuf);

    impl TempDb {
        fn create(sql: &str) -> Self {
            // 进程内计数器避免并行测试在同一微秒取到同名临时库（见 MEMORY 的 flaky 约定）。
            static TEMP_DB_COUNTER: std::sync::atomic::AtomicUsize =
                std::sync::atomic::AtomicUsize::new(0);
            let counter = TEMP_DB_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let unique = format!(
                "navop-import-{}-{}-{}.db",
                std::process::id(),
                Utc::now().timestamp_nanos_opt().unwrap_or_default(),
                counter
            );
            let path = std::env::temp_dir().join(unique);
            let conn = open(&path);
            conn.execute_batch(sql).expect("建表失败");
            Self(path)
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn candidates_are_unique_and_point_at_database_files() {
        let candidates = full_product_candidates();
        assert!(!candidates.is_empty());
        assert!(candidates.iter().all(|path| path.ends_with(DB_FILE_NAME)));
        let unique: std::collections::HashSet<_> = candidates.iter().collect();
        assert_eq!(unique.len(), candidates.len());
        // 与全量产品的目录规则一致：现行 `navop`、开发 `.navop-dev`、历史 `one-hub`。
        for name in ["navop", ".navop-dev", "one-hub"] {
            assert!(
                candidates
                    .iter()
                    .any(|path| path.components().any(|part| part.as_os_str() == name)),
                "候选里应当包含 {name} 目录：{candidates:?}"
            );
        }
    }

    #[test]
    fn reads_legacy_schema_without_optional_columns() {
        let db = TempDb::create(
            "CREATE TABLE connections (id INTEGER PRIMARY KEY, name TEXT, connection_type TEXT, params TEXT);
             INSERT INTO connections (name, connection_type, params) VALUES ('主库', 'Database', '{\"host\":\"127.0.0.1\"}');",
        );
        let bundle = read_bundle_from_sqlite(&db.0).expect("读取失败");
        assert_eq!(bundle.version, BUNDLE_VERSION);
        assert_eq!(bundle.app_id.as_deref(), Some("navop"));
        assert!(bundle.workspaces.is_empty());
        assert_eq!(bundle.connections.len(), 1);
        let connection = &bundle.connections[0];
        assert_eq!(connection.name, "主库");
        assert_eq!(connection.connection_type, "Database");
        assert_eq!(connection.params, r#"{"host":"127.0.0.1"}"#);
        assert_eq!(connection.workspace, None);
        assert_eq!(connection.remark, None);
        assert!(connection.sync_enabled);
    }

    #[test]
    fn resolves_workspace_names_and_flags() {
        let db = TempDb::create(
            "CREATE TABLE workspaces (id INTEGER PRIMARY KEY, name TEXT, color TEXT, icon TEXT);
             CREATE TABLE connections (
                 id INTEGER PRIMARY KEY, name TEXT, connection_type TEXT, params TEXT,
                 workspace_id INTEGER, selected_databases TEXT, remark TEXT, sync_enabled INTEGER
             );
             INSERT INTO workspaces (id, name, color, icon) VALUES (7, '生产', '#ff0000', 'server');
             INSERT INTO connections (name, connection_type, params, workspace_id, remark, sync_enabled)
                 VALUES ('主库', 'Database', '{}', 7, '只读', 0);
             INSERT INTO connections (name, connection_type, params, workspace_id)
                 VALUES ('孤儿', 'Database', '{}', 99);",
        );
        let bundle = read_bundle_from_sqlite(&db.0).expect("读取失败");
        assert_eq!(bundle.workspaces.len(), 1);
        assert_eq!(bundle.workspaces[0].name, "生产");
        assert_eq!(bundle.workspaces[0].color.as_deref(), Some("#ff0000"));
        assert_eq!(bundle.connections[0].workspace.as_deref(), Some("生产"));
        assert_eq!(bundle.connections[0].remark.as_deref(), Some("只读"));
        assert!(!bundle.connections[0].sync_enabled);
        assert_eq!(bundle.connections[1].workspace, None);
    }

    #[test]
    fn rejects_database_without_connections_table() {
        let db = TempDb::create("CREATE TABLE unrelated (id INTEGER PRIMARY KEY);");
        let error = read_bundle_from_sqlite(&db.0).expect_err("应当报错");
        assert!(error.to_string().contains("connections"));
    }

    #[test]
    fn detect_skips_missing_files() {
        assert!(!is_readable_db(
            &std::env::temp_dir().join("definitely-missing.db")
        ));
        assert!(!is_sqlite_database(
            &std::env::temp_dir().join("definitely-missing.db")
        ));
    }

    #[test]
    fn recognizes_sqlite_header_only() {
        let db = TempDb::create("CREATE TABLE connections (id INTEGER PRIMARY KEY);");
        assert!(is_sqlite_database(&db.0));

        let json_dir = std::env::temp_dir();
        let json_path = json_dir.join(format!(
            "navop-import-bundle-{}-{}.json",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&json_path, r#"{"version":1,"connections":[]}"#).expect("写迁移包失败");
        assert!(!is_sqlite_database(&json_path));
        let _ = std::fs::remove_file(&json_path);
    }

    #[test]
    fn pick_candidate_prefers_database_with_connections() {
        let empty = TempDb::create("CREATE TABLE connections (id INTEGER PRIMARY KEY, name TEXT);");
        let filled = TempDb::create(
            "CREATE TABLE connections (id INTEGER PRIMARY KEY, name TEXT, params TEXT);
             INSERT INTO connections (name, params) VALUES ('主库', '{}');",
        );
        let picked = pick_candidate(vec![empty.0.clone(), filled.0.clone()]).expect("应当选出一个");
        assert_eq!(picked, filled.0);

        // 只有空库时也要给用户一个可用路径，由导入结果去提示 0 条。
        let only_empty = pick_candidate(vec![empty.0.clone()]).expect("应当退回到空库");
        assert_eq!(only_empty, empty.0);
        assert_eq!(pick_candidate(Vec::new()), None);
    }

    #[test]
    fn pick_candidate_prefers_the_most_recently_used_database() {
        let older = TempDb::create(
            "CREATE TABLE connections (id INTEGER PRIMARY KEY, name TEXT, params TEXT);
             INSERT INTO connections (name, params) VALUES ('旧', '{}');",
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        let newer = TempDb::create(
            "CREATE TABLE connections (id INTEGER PRIMARY KEY, name TEXT, params TEXT);
             INSERT INTO connections (name, params) VALUES ('新', '{}');",
        );

        assert_eq!(
            pick_candidate(vec![older.0.clone(), newer.0.clone()]),
            Some(newer.0.clone())
        );
        // 候选顺序颠倒也要选「最近在用」的那个，而不是列表里的第一个。
        assert_eq!(
            pick_candidate(vec![newer.0.clone(), older.0.clone()]),
            Some(newer.0.clone())
        );
    }

    #[test]
    fn pick_candidate_skips_unreadable_files() {
        let missing = std::env::temp_dir().join("navop-import-missing-candidate.db");
        let db = TempDb::create("CREATE TABLE connections (id INTEGER PRIMARY KEY, name TEXT);");
        assert_eq!(
            pick_candidate(vec![missing, db.0.clone()]),
            Some(db.0.clone())
        );
    }
}
