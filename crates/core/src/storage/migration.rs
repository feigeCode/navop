use anyhow::Result;
use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[
    (
        "20260225000001",
        include_str!("../../migrations/20260225000001_init.sql"),
    ),
    (
        "20260315000001",
        include_str!("../../migrations/20260315000001_team_sync.sql"),
    ),
    (
        "20260317000001",
        include_str!("../../migrations/20260317000001_connection_owner.sql"),
    ),
    (
        "20260610000001",
        include_str!("../../migrations/20260610000001_sftp_favorite_paths.sql"),
    ),
    (
        "20260618000001",
        include_str!("../../migrations/20260618000001_connection_last_used.sql"),
    ),
    (
        "20260623000001",
        include_str!("../../migrations/20260623000001_connection_sort_order.sql"),
    ),
    (
        "20260626000001",
        include_str!("../../migrations/20260626000001_personal_sync.sql"),
    ),
    (
        "20260630000001",
        include_str!("../../migrations/20260630000001_agent_sessions.sql"),
    ),
    (
        "20260630000002",
        include_str!("../../migrations/20260630000002_team_key_verification_cache.sql"),
    ),
    (
        "20260704000001",
        include_str!("../../migrations/20260704000001_workspace_sort_order.sql"),
    ),
    (
        "20260704000002",
        include_str!("../../migrations/20260704000002_workspace_last_synced_at.sql"),
    ),
    (
        "20260705000001",
        include_str!("../../migrations/20260705000001_terminal_command_history.sql"),
    ),
    (
        "20260707000001",
        include_str!("../../migrations/20260707000001_quick_command_grouping.sql"),
    ),
    (
        "20260711000001",
        include_str!("../../migrations/20260711000001_scoped_team_key_cache.sql"),
    ),
    (
        "20260714000001",
        include_str!("../../migrations/20260714000001_team_membership_cache.sql"),
    ),
    (
        "20260720000001",
        include_str!("../../migrations/20260720000001_default_quick_commands.sql"),
    ),
    (
        "20260721000001",
        include_str!("../../migrations/20260721000001_workspace_hierarchy.sql"),
    ),
    (
        "20260727000001",
        include_str!("../../migrations/20260727000001_connection_credential_revision.sql"),
    ),
    (
        "20260810000001",
        include_str!("../../migrations/20260810000001_workspace_sidebar_collapsed.sql"),
    ),
    (
        "20260810000002",
        include_str!("../../migrations/20260810000002_workspace_sibling_names.sql"),
    ),
    (
        "20260813000001",
        include_str!("../../migrations/20260813000001_credential_vault.sql"),
    ),
    (
        "20260814000001",
        include_str!("../../migrations/20260814000001_personal_sync_conflict_type_key.sql"),
    ),
    (
        "20260816000001",
        include_str!("../../migrations/20260816000001_credential_ssh_expect.sql"),
    ),
    (
        "20260817000001",
        include_str!("../../migrations/20260817000001_quick_command_shortcut.sql"),
    ),
    (
        "20260817000002",
        include_str!("../../migrations/20260817000002_sql_execution_history.sql"),
    ),
    (
        "20260818000001",
        include_str!("../../migrations/20260818000001_remove_credential_kind.sql"),
    ),
];

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            version TEXT PRIMARY KEY,
            applied_at INTEGER NOT NULL
        );",
    )?;

    for (version, sql) in MIGRATIONS {
        let applied: i64 = conn.query_row(
            "SELECT COUNT(*) FROM _migrations WHERE version = ?1",
            [version],
            |row| row.get(0),
        )?;

        if applied == 0 {
            if let Err(e) = conn.execute_batch(sql) {
                let err_msg = e.to_string();
                if err_msg.contains("duplicate column name") {
                    tracing::warn!(
                        "Migration {} skipped (column already exists): {}",
                        version,
                        err_msg
                    );
                } else {
                    return Err(e.into());
                }
            }

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs() as i64;

            conn.execute(
                "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                rusqlite::params![version, now],
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MIGRATIONS, run_migrations};
    use rusqlite::{Connection, params};

    fn mark_all_migrations_except(conn: &Connection, target_version: &str) {
        let mut found_target = false;
        for (version, _) in MIGRATIONS {
            if *version == target_version {
                found_target = true;
                continue;
            }
            conn.execute(
                "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                params![version, 1i64],
            )
            .expect("mark unrelated migration as applied");
        }
        assert!(
            found_target,
            "migration {target_version} must be registered"
        );
    }

    #[test]
    fn credential_revision_migration_backfills_existing_connections() {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(
            "CREATE TABLE _migrations (
                version TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );
            CREATE TABLE connections (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL
            );
            INSERT INTO connections (name) VALUES ('existing');",
        )
        .expect("create pre-migration schema");

        mark_all_migrations_except(&conn, "20260727000001");

        run_migrations(&conn).expect("run credential revision migration");

        let revision: i64 = conn
            .query_row(
                "SELECT credential_revision FROM connections WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("read backfilled revision");
        assert_eq!(1, revision);
        assert!(
            conn.execute(
                "UPDATE connections SET credential_revision = 0 WHERE id = 1",
                [],
            )
            .is_err(),
            "the positive revision invariant must be enforced by SQLite"
        );
    }

    #[test]
    fn workspace_sidebar_collapsed_migration_defaults_existing_rows_to_expanded() {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(
            "CREATE TABLE _migrations (
                version TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );
            CREATE TABLE workspaces (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL
            );
            INSERT INTO workspaces (name) VALUES ('existing');",
        )
        .expect("create pre-migration schema");

        mark_all_migrations_except(&conn, "20260810000001");
        run_migrations(&conn).expect("run workspace sidebar collapsed migration");

        let collapsed: i64 = conn
            .query_row(
                "SELECT sidebar_collapsed FROM workspaces WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("read workspace sidebar collapsed state");
        assert_eq!(0, collapsed);

        run_migrations(&conn).expect("rerun migrations");
    }

    #[test]
    fn workspace_sibling_name_migration_scopes_uniqueness_to_parent() {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
            CREATE TABLE _migrations (
                version TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );
            CREATE TABLE workspaces (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                color TEXT,
                icon TEXT,
                cloud_id TEXT,
                last_synced_at INTEGER,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                parent_id INTEGER REFERENCES workspaces(id) ON DELETE SET NULL,
                sidebar_collapsed INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_workspaces_name ON workspaces(name);
            CREATE INDEX idx_workspaces_cloud_id ON workspaces(cloud_id);
            CREATE INDEX idx_workspaces_parent_id ON workspaces(parent_id);
            CREATE TABLE connections (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                workspace_id INTEGER REFERENCES workspaces(id) ON DELETE SET NULL
            );
            INSERT INTO workspaces
                (id, name, sort_order, created_at, updated_at, parent_id, sidebar_collapsed)
            VALUES
                (1, 'first parent', 0, 1, 1, NULL, 0),
                (2, 'second parent', 1, 1, 1, NULL, 0),
                (3, 'servers', 2, 1, 1, 1, 1);
            INSERT INTO connections (workspace_id) VALUES (3);",
        )
        .expect("create pre-migration schema");

        mark_all_migrations_except(&conn, "20260810000002");
        run_migrations(&conn).expect("run workspace sibling name migration");

        conn.execute(
            "INSERT INTO workspaces
                (name, sort_order, created_at, updated_at, parent_id)
             VALUES ('servers', 3, 1, 1, 2)",
            [],
        )
        .expect("allow the same name under a different parent");
        conn.execute(
            "INSERT INTO workspaces
                (name, sort_order, created_at, updated_at, parent_id)
             VALUES ('servers', 4, 1, 1, NULL)",
            [],
        )
        .expect("allow a root workspace to share a child workspace name");
        assert!(
            conn.execute(
                "INSERT INTO workspaces
                    (name, sort_order, created_at, updated_at, parent_id)
                 VALUES ('servers', 5, 1, 1, 1)",
                [],
            )
            .is_err(),
            "duplicate sibling names must remain rejected"
        );
        assert!(
            conn.execute(
                "INSERT INTO workspaces
                    (name, sort_order, created_at, updated_at, parent_id)
                 VALUES ('servers', 6, 1, 1, NULL)",
                [],
            )
            .is_err(),
            "duplicate root workspace names must remain rejected"
        );
        assert!(
            conn.execute("UPDATE workspaces SET parent_id = 1 WHERE id = 4", [],)
                .is_err(),
            "moving a workspace must enforce the target parent's name scope"
        );

        let connection_workspace_id: i64 = conn
            .query_row(
                "SELECT workspace_id FROM connections WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("read preserved connection workspace");
        assert_eq!(3, connection_workspace_id);
        assert_eq!(
            1,
            conn.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .expect("read foreign key mode")
        );
        assert_eq!(
            0,
            conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            },)
                .expect("check foreign keys")
        );
        assert_eq!(
            (Some(1), 1),
            conn.query_row(
                "SELECT parent_id, sidebar_collapsed FROM workspaces WHERE id = 3",
                [],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, i64>(1)?)),
            )
            .expect("read preserved workspace hierarchy and collapsed state")
        );

        run_migrations(&conn).expect("rerun migrations");
    }

    #[test]
    fn personal_sync_conflict_migration_scopes_identity_to_data_type() {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute_batch(
            "CREATE TABLE _migrations (
                version TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );
            CREATE TABLE personal_sync_conflicts (
                backend_profile_id TEXT NOT NULL,
                record_id TEXT NOT NULL,
                data_type TEXT NOT NULL,
                conflict_type TEXT NOT NULL,
                local_snapshot TEXT,
                remote_snapshot TEXT,
                detected_at INTEGER NOT NULL,
                PRIMARY KEY (backend_profile_id, record_id)
            );
            INSERT INTO personal_sync_conflicts
                (backend_profile_id, record_id, data_type, conflict_type, detected_at)
            VALUES
                ('personal', 'shared-cloud-id', 'connection', 'both_modified', 1);",
        )
        .expect("create pre-migration personal sync schema");

        mark_all_migrations_except(&conn, "20260814000001");
        run_migrations(&conn).expect("run personal sync conflict identity migration");

        conn.execute(
            "INSERT INTO personal_sync_conflicts
                (backend_profile_id, record_id, data_type, conflict_type, detected_at)
             VALUES
                ('personal', 'shared-cloud-id', 'credential', 'both_modified', 2)",
            [],
        )
        .expect("allow the same cloud id for a different data type");
        assert_eq!(
            2,
            conn.query_row(
                "SELECT COUNT(*) FROM personal_sync_conflicts
                 WHERE backend_profile_id = 'personal' AND record_id = 'shared-cloud-id'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("count isolated conflicts")
        );
        assert!(
            conn.execute(
                "INSERT INTO personal_sync_conflicts
                    (backend_profile_id, record_id, data_type, conflict_type, detected_at)
                 VALUES
                    ('personal', 'shared-cloud-id', 'credential', 'both_modified', 3)",
                [],
            )
            .is_err(),
            "duplicate conflicts of the same data type must remain rejected"
        );

        run_migrations(&conn).expect("rerun migrations");
    }
}
