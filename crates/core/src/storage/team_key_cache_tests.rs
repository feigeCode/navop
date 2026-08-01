use rusqlite::{Connection, params};

use crate::cloud_sync::team_scope::CloudAccountScope;
use crate::storage::connection::SqliteConnection;
use crate::storage::migration::run_migrations;
use crate::storage::{TeamKeyCache, TeamKeyCacheRepository};

const PRE_SCOPE_MIGRATIONS: &[&str] = &[
    "20260225000001",
    "20260315000001",
    "20260317000001",
    "20260610000001",
    "20260618000001",
    "20260623000001",
    "20260626000001",
    "20260630000001",
    "20260630000002",
    "20260704000001",
    "20260704000002",
    "20260705000001",
    "20260707000001",
];

#[test]
fn repository_isolates_same_team_id_by_environment_and_user() {
    let connection =
        SqliteConnection::open_with_pool_size(temp_db_path("scope"), 1).expect("open sqlite");
    connection
        .with_connection(run_migrations)
        .expect("run migrations");
    let repo = TeamKeyCacheRepository::new(connection);
    let alice_prod = CloudAccountScope::new("https://project.supabase.co/", "alice");
    let bob_prod = CloudAccountScope::new("https://project.supabase.co", "bob");
    let alice_stage = CloudAccountScope::new("https://stage.supabase.co", "alice");

    repo.upsert(&cache(&alice_prod, "team-1", 4, Some(4)))
        .expect("insert alice cache");
    repo.upsert(&cache(&bob_prod, "team-1", 4, Some(4)))
        .expect("insert bob cache");

    assert_eq!(1, repo.list(&alice_prod).expect("list alice").len());
    assert_eq!(1, repo.list(&bob_prod).expect("list bob").len());
    assert!(repo.list(&alice_stage).expect("list stage").is_empty());
    assert_eq!(
        "alice-secret",
        repo.get(&alice_prod, "team-1")
            .expect("get alice")
            .expect("alice cache")
            .encrypted_team_key
            .as_deref()
            .expect("alice secret")
    );
}

#[test]
fn scoped_cache_migration_discards_unsafe_unscoped_secrets() {
    let connection = Connection::open_in_memory().expect("open memory sqlite");
    seed_pre_scope_database(&connection);

    run_migrations(&connection).expect("run scoped migration");

    let secret_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM team_key_cache", [], |row| row.get(0))
        .expect("count migrated cache");
    assert_eq!(0, secret_count);
    connection
        .execute(
            "INSERT INTO team_key_cache (
                cloud_environment, user_id, team_id, team_name, key_version,
                cached_key_version, key_verification, encrypted_team_key,
                last_verified_at, updated_at, role
             ) VALUES (?1, ?2, ?3, 'Platform', 1, 1, 'verify', 'secret', 1, 1, 'member')",
            params!["https://project.supabase.co", "alice", "team-1"],
        )
        .expect("insert alice scoped cache");
    connection
        .execute(
            "INSERT INTO team_key_cache (
                cloud_environment, user_id, team_id, team_name, key_version,
                cached_key_version, key_verification, encrypted_team_key,
                last_verified_at, updated_at, role
             ) VALUES (?1, ?2, ?3, 'Platform', 1, 1, 'verify', 'secret', 1, 1, 'member')",
            params!["https://project.supabase.co", "bob", "team-1"],
        )
        .expect("insert bob scoped cache");
}

fn cache(
    scope: &CloudAccountScope,
    team_id: &str,
    key_version: u32,
    cached_key_version: Option<u32>,
) -> TeamKeyCache {
    TeamKeyCache {
        scope: scope.clone(),
        team_id: team_id.to_string(),
        team_name: "Platform".to_string(),
        key_version,
        cached_key_version,
        key_verification: Some("verification".to_string()),
        encrypted_team_key: Some(format!("{}-secret", scope.user_id)),
        last_verified_at: Some(123),
        updated_at: 200,
        role: Some("member".to_string()),
    }
}

fn seed_pre_scope_database(connection: &Connection) {
    connection
        .execute_batch(
            "CREATE TABLE _migrations (
                version TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
             );
             CREATE TABLE team_key_cache (
                team_id TEXT PRIMARY KEY,
                team_name TEXT NOT NULL,
                key_version INTEGER NOT NULL DEFAULT 0,
                encrypted_team_key TEXT,
                last_verified_at INTEGER,
                updated_at INTEGER NOT NULL,
                role TEXT,
                key_verification TEXT
             );
             CREATE TABLE quick_commands (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT,
                command TEXT NOT NULL,
                description TEXT,
                pinned INTEGER NOT NULL DEFAULT 0,
                sort_order INTEGER NOT NULL DEFAULT 0,
                connection_id INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                group_name TEXT,
                group_color TEXT
             );
             CREATE TABLE connections (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL
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
                updated_at INTEGER NOT NULL
             );
             INSERT INTO team_key_cache VALUES (
                'team-1', 'Platform', 1, 'unsafe-secret', 1, 1, 'admin', 'verify'
             );",
        )
        .expect("seed old cache");
    for version in PRE_SCOPE_MIGRATIONS {
        connection
            .execute(
                "INSERT INTO _migrations (version, applied_at) VALUES (?1, 1)",
                [version],
            )
            .expect("mark old migration");
    }
}

fn temp_db_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "navop-team-key-cache-{label}-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ))
}
