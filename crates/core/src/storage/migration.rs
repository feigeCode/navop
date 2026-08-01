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

        for (version, _) in &MIGRATIONS[..MIGRATIONS.len() - 1] {
            conn.execute(
                "INSERT INTO _migrations (version, applied_at) VALUES (?1, ?2)",
                params![version, 1i64],
            )
            .expect("mark preceding migration as applied");
        }

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
}
