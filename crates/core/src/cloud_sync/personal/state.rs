use anyhow::Result as AnyhowResult;
use rusqlite::params;

use crate::storage::connection::SqliteConnection;

use super::SyncStoreHealth;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonalConflictType {
    BothModified,
    LocalDeletedRemoteModified,
    LocalModifiedRemoteDeleted,
}

impl PersonalConflictType {
    fn as_str(self) -> &'static str {
        match self {
            Self::BothModified => "both_modified",
            Self::LocalDeletedRemoteModified => "local_deleted_remote_modified",
            Self::LocalModifiedRemoteDeleted => "local_modified_remote_deleted",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "local_deleted_remote_modified" => Self::LocalDeletedRemoteModified,
            "local_modified_remote_deleted" => Self::LocalModifiedRemoteDeleted,
            _ => Self::BothModified,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalSyncConflict {
    pub backend_profile_id: String,
    pub record_id: String,
    pub data_type: String,
    pub conflict_type: PersonalConflictType,
    pub local_snapshot: Option<String>,
    pub remote_snapshot: Option<String>,
    pub detected_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalSyncStoredStatus {
    pub backend_profile_id: String,
    pub health: SyncStoreHealth,
    pub last_success_at: Option<i64>,
    pub last_error: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone)]
pub struct PersonalSyncConflictRepository {
    conn: SqliteConnection,
}

impl PersonalSyncConflictRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn upsert(&self, conflict: &PersonalSyncConflict) -> AnyhowResult<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO personal_sync_conflicts
                 (backend_profile_id, record_id, data_type, conflict_type, local_snapshot, remote_snapshot, detected_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(backend_profile_id, record_id) DO UPDATE SET
                   data_type = excluded.data_type,
                   conflict_type = excluded.conflict_type,
                   local_snapshot = excluded.local_snapshot,
                   remote_snapshot = excluded.remote_snapshot,
                   detected_at = excluded.detected_at",
                params![
                    conflict.backend_profile_id,
                    conflict.record_id,
                    conflict.data_type,
                    conflict.conflict_type.as_str(),
                    conflict.local_snapshot,
                    conflict.remote_snapshot,
                    conflict.detected_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn list(&self, backend_profile_id: &str) -> AnyhowResult<Vec<PersonalSyncConflict>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT backend_profile_id, record_id, data_type, conflict_type, local_snapshot, remote_snapshot, detected_at
                 FROM personal_sync_conflicts
                 WHERE backend_profile_id = ?1
                 ORDER BY detected_at DESC, record_id ASC",
            )?;
            let rows = stmt.query_map([backend_profile_id], |row| {
                Ok(PersonalSyncConflict {
                    backend_profile_id: row.get(0)?,
                    record_id: row.get(1)?,
                    data_type: row.get(2)?,
                    conflict_type: PersonalConflictType::from_str(row.get::<_, String>(3)?.as_str()),
                    local_snapshot: row.get(4)?,
                    remote_snapshot: row.get(5)?,
                    detected_at: row.get(6)?,
                })
            })?;

            let mut conflicts = Vec::new();
            for row in rows {
                conflicts.push(row?);
            }
            Ok(conflicts)
        })
    }

    pub fn delete(&self, backend_profile_id: &str, record_id: &str) -> AnyhowResult<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "DELETE FROM personal_sync_conflicts
                 WHERE backend_profile_id = ?1 AND record_id = ?2",
                params![backend_profile_id, record_id],
            )?;
            Ok(())
        })
    }
}

#[derive(Clone)]
pub struct PersonalSyncStatusRepository {
    conn: SqliteConnection,
}

impl PersonalSyncStatusRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn save(&self, status: &PersonalSyncStoredStatus) -> AnyhowResult<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO personal_sync_status
                 (backend_profile_id, health, last_success_at, last_error, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(backend_profile_id) DO UPDATE SET
                   health = excluded.health,
                   last_success_at = excluded.last_success_at,
                   last_error = excluded.last_error,
                   updated_at = excluded.updated_at",
                params![
                    status.backend_profile_id,
                    status.health.as_str(),
                    status.last_success_at,
                    status.last_error,
                    status.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn get(&self, backend_profile_id: &str) -> AnyhowResult<Option<PersonalSyncStoredStatus>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT backend_profile_id, health, last_success_at, last_error, updated_at
                 FROM personal_sync_status
                 WHERE backend_profile_id = ?1",
            )?;
            let mut rows = stmt.query([backend_profile_id])?;
            let Some(row) = rows.next()? else {
                return Ok(None);
            };

            Ok(Some(PersonalSyncStoredStatus {
                backend_profile_id: row.get(0)?,
                health: SyncStoreHealth::from_str(row.get::<_, String>(1)?.as_str()),
                last_success_at: row.get(2)?,
                last_error: row.get(3)?,
                updated_at: row.get(4)?,
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::cloud_sync::models::data_type;
    use crate::cloud_sync::personal::{
        PersonalConflictType, PersonalSyncConflict, PersonalSyncConflictRepository,
        PersonalSyncStatusRepository, PersonalSyncStoredStatus, SyncStoreHealth,
    };
    use crate::storage::connection::SqliteConnection;
    use crate::storage::migration::run_migrations;

    #[test]
    fn conflict_repository_round_trips_paused_conflict() {
        let temp = tempfile::tempdir().expect("tempdir");
        let conn = SqliteConnection::open(temp.path().join("test.db")).expect("sqlite");
        conn.with_connection(|conn| run_migrations(conn))
            .expect("migrations run");
        let repo = PersonalSyncConflictRepository::new(conn);
        let conflict = PersonalSyncConflict {
            backend_profile_id: "personal".to_string(),
            record_id: "record-1".to_string(),
            data_type: data_type::CONNECTION.to_string(),
            conflict_type: PersonalConflictType::BothModified,
            local_snapshot: Some("local".to_string()),
            remote_snapshot: Some("remote".to_string()),
            detected_at: 100,
        };

        repo.upsert(&conflict).expect("conflict stored");
        let loaded = repo.list("personal").expect("conflicts list");

        assert_eq!(vec![conflict], loaded);
    }

    #[test]
    fn conflict_repository_deletes_resolved_conflict() {
        let temp = tempfile::tempdir().expect("tempdir");
        let conn = SqliteConnection::open(temp.path().join("test.db")).expect("sqlite");
        conn.with_connection(|conn| run_migrations(conn))
            .expect("migrations run");
        let repo = PersonalSyncConflictRepository::new(conn);
        let conflict = PersonalSyncConflict {
            backend_profile_id: "personal".to_string(),
            record_id: "record-1".to_string(),
            data_type: data_type::CONNECTION.to_string(),
            conflict_type: PersonalConflictType::BothModified,
            local_snapshot: Some("local".to_string()),
            remote_snapshot: Some("remote".to_string()),
            detected_at: 100,
        };

        repo.upsert(&conflict).expect("conflict stored");
        repo.delete("personal", "record-1")
            .expect("conflict deleted");

        assert!(repo.list("personal").expect("conflicts list").is_empty());
    }

    #[test]
    fn status_repository_persists_last_success_and_pause_reason() {
        let temp = tempfile::tempdir().expect("tempdir");
        let conn = SqliteConnection::open(temp.path().join("test.db")).expect("sqlite");
        conn.with_connection(|conn| run_migrations(conn))
            .expect("migrations run");
        let repo = PersonalSyncStatusRepository::new(conn);
        let status = PersonalSyncStoredStatus {
            backend_profile_id: "personal".to_string(),
            health: SyncStoreHealth::PausedAfterRepeatedFailures,
            last_success_at: Some(120),
            last_error: Some("git auth required".to_string()),
            updated_at: 130,
        };

        repo.save(&status).expect("status stored");
        let loaded = repo
            .get("personal")
            .expect("status loads")
            .expect("status exists");

        assert_eq!(status, loaded);
    }
}
