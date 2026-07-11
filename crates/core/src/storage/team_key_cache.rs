use anyhow::Result;
use rusqlite::params;

use crate::cloud_sync::CloudAccountScope;
use crate::storage::connection::SqliteConnection;
use crate::storage::manager::now;

#[derive(Debug, Clone)]
pub struct TeamKeyCache {
    pub scope: CloudAccountScope,
    pub team_id: String,
    pub team_name: String,
    pub key_version: u32,
    pub cached_key_version: Option<u32>,
    pub key_verification: Option<String>,
    pub encrypted_team_key: Option<String>,
    pub last_verified_at: Option<i64>,
    pub updated_at: i64,
    pub role: Option<String>,
}

#[derive(Clone)]
pub struct TeamKeyCacheRepository {
    conn: SqliteConnection,
}

impl TeamKeyCacheRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn get(&self, scope: &CloudAccountScope, team_id: &str) -> Result<Option<TeamKeyCache>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT cloud_environment, user_id, team_id, team_name, key_version,
                        cached_key_version, key_verification, encrypted_team_key,
                        last_verified_at, updated_at, role
                   FROM team_key_cache
                  WHERE cloud_environment = ?1 AND user_id = ?2 AND team_id = ?3",
            )?;
            let mut rows = stmt.query(params![scope.environment, scope.user_id, team_id])?;
            Ok(rows.next()?.map(cache_from_row).transpose()?)
        })
    }

    pub fn upsert(&self, cache: &TeamKeyCache) -> Result<()> {
        let ts = now();
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO team_key_cache (
                    cloud_environment, user_id, team_id, team_name, key_version,
                    cached_key_version, key_verification, encrypted_team_key,
                    last_verified_at, updated_at, role
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(cloud_environment, user_id, team_id) DO UPDATE SET
                    team_name = excluded.team_name,
                    key_version = excluded.key_version,
                    cached_key_version = excluded.cached_key_version,
                    key_verification = excluded.key_verification,
                    encrypted_team_key = excluded.encrypted_team_key,
                    last_verified_at = excluded.last_verified_at,
                    updated_at = excluded.updated_at,
                    role = excluded.role",
                params![
                    cache.scope.environment,
                    cache.scope.user_id,
                    cache.team_id,
                    cache.team_name,
                    cache.key_version as i64,
                    cache.cached_key_version.map(i64::from),
                    cache.key_verification,
                    cache.encrypted_team_key,
                    cache.last_verified_at,
                    ts,
                    cache.role,
                ],
            )?;
            Ok(())
        })
    }

    pub fn list(&self, scope: &CloudAccountScope) -> Result<Vec<TeamKeyCache>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT cloud_environment, user_id, team_id, team_name, key_version,
                        cached_key_version, key_verification, encrypted_team_key,
                        last_verified_at, updated_at, role
                   FROM team_key_cache
                  WHERE cloud_environment = ?1 AND user_id = ?2
                  ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map(params![scope.environment, scope.user_id], cache_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        })
    }

    pub fn delete(&self, scope: &CloudAccountScope, team_id: &str) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "DELETE FROM team_key_cache
                  WHERE cloud_environment = ?1 AND user_id = ?2 AND team_id = ?3",
                params![scope.environment, scope.user_id, team_id],
            )?;
            Ok(())
        })
    }
}

fn cache_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TeamKeyCache> {
    Ok(TeamKeyCache {
        scope: CloudAccountScope::new(row.get::<_, String>(0)?, row.get::<_, String>(1)?),
        team_id: row.get(2)?,
        team_name: row.get(3)?,
        key_version: row.get::<_, i64>(4)? as u32,
        cached_key_version: row.get::<_, Option<i64>>(5)?.map(|value| value as u32),
        key_verification: row.get(6)?,
        encrypted_team_key: row.get(7)?,
        last_verified_at: row.get(8)?,
        updated_at: row.get(9)?,
        role: row.get(10)?,
    })
}
