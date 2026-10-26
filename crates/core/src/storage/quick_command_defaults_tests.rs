use rusqlite::Connection;

use crate::storage::migration::run_migrations;

#[test]
fn migration_installs_grouped_global_quick_commands() {
    let connection = Connection::open_in_memory().expect("open memory sqlite");

    run_migrations(&connection).expect("run migrations");

    assert_eq!(30, count_commands(&connection));
    assert_eq!(0, count_unscoped_or_ungrouped_commands(&connection));
    assert_eq!(
        vec!["Docker", "Files", "Git", "Network", "System"],
        group_names(&connection)
    );
    for command in ["pwd", "df -h", "ip addr", "git status", "docker ps"] {
        assert_eq!(1, count_command(&connection, command), "missing {command}");
    }
}

#[test]
fn migration_is_idempotent() {
    let connection = Connection::open_in_memory().expect("open memory sqlite");

    run_migrations(&connection).expect("run initial migrations");
    run_migrations(&connection).expect("run migrations again");

    assert_eq!(30, count_commands(&connection));
    assert_eq!(30, count_distinct_commands(&connection));
}

#[test]
fn migration_preserves_an_existing_global_command() {
    let connection = Connection::open_in_memory().expect("open memory sqlite");
    run_migrations(&connection).expect("run initial migrations");
    connection
        .execute(
            "UPDATE quick_commands SET name = 'My PWD', command = ' PWD ' WHERE command = 'pwd'",
            [],
        )
        .expect("customize existing command");
    connection
        .execute(
            "DELETE FROM _migrations WHERE version = '20260720000001'",
            [],
        )
        .expect("make default migration runnable again");

    run_migrations(&connection).expect("rerun default migration");

    assert_eq!(30, count_commands(&connection));
    let name: String = connection
        .query_row(
            "SELECT name FROM quick_commands WHERE LOWER(TRIM(command)) = 'pwd'",
            [],
            |row| row.get(0),
        )
        .expect("read customized command");
    assert_eq!("My PWD", name);
}

#[test]
fn migration_adds_nullable_quick_command_shortcut_column() {
    let connection = Connection::open_in_memory().expect("open memory sqlite");

    run_migrations(&connection).expect("run migrations");

    assert_eq!(
        1,
        connection
            .query_row(
                "SELECT COUNT(*)
                 FROM pragma_table_info('quick_commands')
                 WHERE name = 'shortcut' AND type = 'TEXT'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("count shortcut columns")
    );
    assert_eq!(
        0,
        connection
            .query_row(
                "SELECT COUNT(*) FROM quick_commands WHERE shortcut IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("count default shortcuts")
    );
}

fn count_commands(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT COUNT(*) FROM quick_commands", [], |row| row.get(0))
        .expect("count quick commands")
}

fn count_distinct_commands(connection: &Connection) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(DISTINCT command) FROM quick_commands",
            [],
            |row| row.get(0),
        )
        .expect("count distinct quick commands")
}

fn count_unscoped_or_ungrouped_commands(connection: &Connection) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM quick_commands
             WHERE connection_id IS NOT NULL OR TRIM(COALESCE(group_name, '')) = ''",
            [],
            |row| row.get(0),
        )
        .expect("count invalid default commands")
}

fn group_names(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT DISTINCT group_name FROM quick_commands ORDER BY group_name")
        .expect("prepare group query");
    statement
        .query_map([], |row| row.get(0))
        .expect("query groups")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect groups")
}

fn count_command(connection: &Connection, command: &str) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM quick_commands WHERE command = ?1",
            [command],
            |row| row.get(0),
        )
        .expect("count command")
}

#[test]
fn is_builtin_backfill_keeps_user_commands_syncable() {
    use crate::storage::migration::MIGRATIONS;
    use rusqlite::params;

    let connection = Connection::open_in_memory().expect("open memory sqlite");

    connection
        .execute_batch(
            "CREATE TABLE _migrations (
                version TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );",
        )
        .expect("create migrations table");

    // 手工构造三类命令：
    // 1. 与内置同名同文本的默认命令（应标记 is_builtin=1）；
    // 2. 用户自建的、与内置同文本但不同名的命令（应保持 is_builtin=0 参与同步）；
    // 3. 用户自建且置顶的同名同文本命令（应保持 is_builtin=0）。
    fn mark_all_migrations_except(conn: &Connection, targets: &[&str]) {
        let mut found: Vec<&str> = Vec::new();
        for (version, _) in MIGRATIONS {
            if targets.contains(version) {
                found.push(version);
                continue;
            }
            conn.execute(
                "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                params![version, 1i64],
            )
            .expect("mark unrelated migration as applied");
        }
        assert_eq!(
            found.len(),
            targets.len(),
            "所有目标迁移都必须已注册: 找到 {found:?}, 期望 {targets:?}"
        );
    }

    mark_all_migrations_except(&connection, &["20261014000001", "20260720000001"]);
    connection
        .execute_batch(
            "CREATE TABLE quick_commands (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                command TEXT NOT NULL,
                description TEXT,
                pinned INTEGER NOT NULL DEFAULT 0,
                sort_order INTEGER NOT NULL DEFAULT 0,
                connection_id INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                group_name TEXT,
                group_color TEXT,
                shortcut TEXT
            );
            INSERT INTO quick_commands (name, command, pinned, created_at, updated_at) VALUES
                ('Disk Usage', 'df -h', 0, 1, 1),
                ('我的磁盘检查', 'df -h', 0, 1, 1),
                ('Disk Usage', 'df -h', 1, 1, 1);",
        )
        .expect("create pre-migration quick commands");

    run_migrations(&connection).expect("run is_builtin migration");

    let builtin_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM quick_commands WHERE is_builtin = 1",
            [],
            |row| row.get(0),
        )
        .expect("count builtin commands");
    // 20260720000001 按 command 文本 NOT EXISTS 判重：预置的 'df -h' 挡住默认
    // 'Disk Usage'，其余 29 条默认照插 → 内置标记 = 29 默认 + 预置第 1 条（同名同文本未置顶）。
    assert_eq!(
        builtin_count, 30,
        "29 条默认 + 1 条同名同文本预置应标记内置"
    );
    let total: i64 = connection
        .query_row("SELECT COUNT(*) FROM quick_commands", [], |row| row.get(0))
        .expect("count total");
    assert_eq!(total, 32, "3 预置 + 29 条默认");

    let user_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM quick_commands
             WHERE is_builtin = 0 AND (name = '我的磁盘检查' OR pinned = 1)",
            [],
            |row| row.get(0),
        )
        .expect("count user commands");
    assert_eq!(
        user_rows, 2,
        "用户自建命令（不同名或已置置顶）必须保持可同步"
    );
}
