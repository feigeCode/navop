use anyhow::Result;
use rusqlite::params;

use crate::cloud_sync::CloudAccountScope;
use crate::storage::connection::SqliteConnection;
use crate::storage::manager::now;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamMembershipState {
    Active,
    Departed,
    Unknown,
}

impl TeamMembershipState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Departed => "departed",
            Self::Unknown => "unknown",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "active" => Self::Active,
            "departed" => Self::Departed,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TeamMembershipCache {
    pub scope: CloudAccountScope,
    pub team_id: String,
    pub team_name: String,
    pub role: Option<String>,
    pub state: TeamMembershipState,
    pub last_seen_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Clone)]
pub struct TeamMembershipCacheRepository {
    conn: SqliteConnection,
}

impl TeamMembershipCacheRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn get(
        &self,
        scope: &CloudAccountScope,
        team_id: &str,
    ) -> Result<Option<TeamMembershipCache>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT cloud_environment, user_id, team_id, team_name, role,
                        membership_state, last_seen_at, updated_at
                   FROM team_membership_cache
                  WHERE cloud_environment = ?1 AND user_id = ?2 AND team_id = ?3",
            )?;
            let mut rows = stmt.query(params![scope.environment, scope.user_id, team_id])?;
            Ok(rows.next()?.map(cache_from_row).transpose()?)
        })
    }

    pub fn upsert(&self, cache: &TeamMembershipCache) -> Result<()> {
        let ts = now();
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO team_membership_cache (
                    cloud_environment, user_id, team_id, team_name, role,
                    membership_state, last_seen_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(cloud_environment, user_id, team_id) DO UPDATE SET
                    team_name = excluded.team_name,
                    role = excluded.role,
                    membership_state = excluded.membership_state,
                    last_seen_at = excluded.last_seen_at,
                    updated_at = excluded.updated_at",
                params![
                    cache.scope.environment,
                    cache.scope.user_id,
                    cache.team_id,
                    cache.team_name,
                    cache.role,
                    cache.state.as_str(),
                    cache.last_seen_at,
                    ts,
                ],
            )?;
            Ok(())
        })
    }

    pub fn list(&self, scope: &CloudAccountScope) -> Result<Vec<TeamMembershipCache>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT cloud_environment, user_id, team_id, team_name, role,
                        membership_state, last_seen_at, updated_at
                   FROM team_membership_cache
                  WHERE cloud_environment = ?1 AND user_id = ?2
                  ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map(params![scope.environment, scope.user_id], cache_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        })
    }
}

fn cache_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TeamMembershipCache> {
    Ok(TeamMembershipCache {
        scope: CloudAccountScope::new(row.get::<_, String>(0)?, row.get::<_, String>(1)?),
        team_id: row.get(2)?,
        team_name: row.get(3)?,
        role: row.get(4)?,
        state: TeamMembershipState::from_str(&row.get::<_, String>(5)?),
        last_seen_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migration::run_migrations;

    #[test]
    fn migration_backfills_scoped_team_key_cache() {
        let connection = rusqlite::Connection::open_in_memory().expect("open sqlite");
        connection
            .execute_batch(
                "CREATE TABLE team_key_cache (
                    cloud_environment TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    team_id TEXT NOT NULL,
                    team_name TEXT NOT NULL,
                    key_version INTEGER NOT NULL DEFAULT 0,
                    cached_key_version INTEGER,
                    key_verification TEXT,
                    encrypted_team_key TEXT,
                    last_verified_at INTEGER,
                    updated_at INTEGER NOT NULL,
                    role TEXT,
                    PRIMARY KEY (cloud_environment, user_id, team_id)
                );
                INSERT INTO team_key_cache (
                    cloud_environment, user_id, team_id, team_name, updated_at, role
                ) VALUES ('cloud-a', 'user-1', 'team-1', 'Platform', 123, 'admin');",
            )
            .expect("seed scoped team key cache");

        connection
            .execute_batch(include_str!(
                "../../migrations/20260714000001_team_membership_cache.sql"
            ))
            .expect("run membership migration");

        let row = connection
            .query_row(
                "SELECT team_name, role, membership_state, last_seen_at
                   FROM team_membership_cache
                  WHERE cloud_environment = 'cloud-a'
                    AND user_id = 'user-1'
                    AND team_id = 'team-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .expect("read membership backfill");

        assert_eq!(
            (
                "Platform".to_string(),
                Some("admin".to_string()),
                "active".to_string(),
                Some(123),
            ),
            row
        );
    }

    #[test]
    fn repository_retains_departed_team_display_metadata() {
        let path = std::env::temp_dir().join(format!(
            "navop-team-membership-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let connection = SqliteConnection::open_with_pool_size(path, 1).expect("open sqlite");
        connection
            .with_connection(run_migrations)
            .expect("run migrations");
        let repo = TeamMembershipCacheRepository::new(connection);
        let scope = CloudAccountScope::new("https://project.supabase.co", "user-1");
        repo.upsert(&TeamMembershipCache {
            scope: scope.clone(),
            team_id: "team-1".to_string(),
            team_name: "Platform".to_string(),
            role: None,
            state: TeamMembershipState::Departed,
            last_seen_at: Some(100),
            updated_at: 200,
        })
        .expect("save membership");

        let loaded = repo
            .get(&scope, "team-1")
            .expect("read membership")
            .expect("membership exists");

        assert_eq!("Platform", loaded.team_name);
        assert_eq!(TeamMembershipState::Departed, loaded.state);
        assert_eq!(None, loaded.role);
        assert_eq!(Some(100), loaded.last_seen_at);
    }
}
