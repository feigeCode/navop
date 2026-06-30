use anyhow::Result;
use gpui::SharedString;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::storage::connection::SqliteConnection;
use crate::storage::now;
use crate::storage::row_mapping::FromSqliteRow;
use crate::storage::traits::Repository;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: i64,
    pub uid: String,
    pub title: String,
    pub snapshot_json: String,
    pub archived: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl FromSqliteRow for AgentSession {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let archived: i32 = row.get("archived")?;
        Ok(AgentSession {
            id: row.get("id")?,
            uid: row.get("uid")?,
            title: row.get("title")?,
            snapshot_json: row.get("snapshot_json")?,
            archived: archived != 0,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

impl crate::storage::traits::Entity for AgentSession {
    fn id(&self) -> Option<i64> {
        Some(self.id)
    }

    fn created_at(&self) -> i64 {
        self.created_at
    }

    fn updated_at(&self) -> i64 {
        self.updated_at
    }
}

impl AgentSession {
    pub fn new(uid: String, title: String, snapshot_json: String) -> Self {
        let now = now();
        Self {
            id: 0,
            uid,
            title,
            snapshot_json,
            archived: false,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: i64,
    pub name: String,
    pub provider_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl FromSqliteRow for ChatSession {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(ChatSession {
            id: row.get("id")?,
            name: row.get("name")?,
            provider_id: row.get("provider_id")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

impl crate::storage::traits::Entity for ChatSession {
    fn id(&self) -> Option<i64> {
        Some(self.id)
    }

    fn created_at(&self) -> i64 {
        self.created_at
    }

    fn updated_at(&self) -> i64 {
        self.updated_at
    }
}

impl ChatSession {
    pub fn new(name: String, provider_id: String) -> Self {
        let now = now();
        Self {
            id: 0,
            name,
            provider_id,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: i64,
    pub session_id: i64,
    pub role: String,
    pub content: String,
    pub created_at: i64,
}

impl FromSqliteRow for ChatMessage {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(ChatMessage {
            id: row.get("id")?,
            session_id: row.get("session_id")?,
            role: row.get("role")?,
            content: row.get("content")?,
            created_at: row.get("created_at")?,
        })
    }
}

impl crate::storage::traits::Entity for ChatMessage {
    fn id(&self) -> Option<i64> {
        Some(self.id)
    }

    fn created_at(&self) -> i64 {
        self.created_at
    }

    fn updated_at(&self) -> i64 {
        self.created_at
    }
}

impl ChatMessage {
    pub fn new(session_id: i64, role: String, content: String) -> Self {
        Self {
            id: 0,
            session_id,
            role,
            content,
            created_at: now(),
        }
    }

    pub fn user(session_id: i64, content: String) -> Self {
        Self::new(session_id, "user".to_string(), content)
    }

    pub fn assistant(session_id: i64, content: String) -> Self {
        Self::new(session_id, "assistant".to_string(), content)
    }

    pub fn system(session_id: i64, content: String) -> Self {
        Self::new(session_id, "system".to_string(), content)
    }
}

#[derive(Clone)]
pub struct SessionRepository {
    conn: SqliteConnection,
}

impl SessionRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }
}

impl Repository for SessionRepository {
    type Entity = ChatSession;

    fn entity_type(&self) -> SharedString {
        SharedString::from("ChatSession")
    }

    fn insert(&self, item: &mut Self::Entity) -> Result<i64> {
        let name = item.name.clone();
        let provider_id = item.provider_id.clone();
        let created_at = item.created_at;
        let updated_at = item.updated_at;

        let id = self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO chat_sessions (name, provider_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params![name, provider_id, created_at, updated_at],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        item.id = id;
        Ok(id)
    }

    fn update(&self, item: &Self::Entity) -> Result<()> {
        let id = item.id;
        let name = item.name.clone();
        let provider_id = item.provider_id.clone();
        let updated_at = now();

        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE chat_sessions SET name = ?1, provider_id = ?2, updated_at = ?3 WHERE id = ?4",
                params![name, provider_id, updated_at, id],
            )?;
            Ok(())
        })
    }

    fn delete(&self, id: i64) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute("DELETE FROM chat_sessions WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    fn get(&self, id: i64) -> Result<Option<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id, name, provider_id, created_at, updated_at FROM chat_sessions WHERE id = ?1")?;
            let mut rows = stmt.query(params![id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(ChatSession::from_row(row)?))
            } else {
                Ok(None)
            }
        })
    }

    fn list(&self) -> Result<Vec<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id, name, provider_id, created_at, updated_at FROM chat_sessions ORDER BY updated_at DESC")?;
            let rows = stmt.query_map([], |row| ChatSession::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    fn count(&self) -> Result<i64> {
        self.conn.with_connection(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM chat_sessions", [], |row| row.get(0))?;
            Ok(count)
        })
    }

    fn exists(&self, id: i64) -> Result<bool> {
        self.conn.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM chat_sessions WHERE id = ?1)",
                params![id],
                |row| row.get(0),
            )?;
            Ok(exists == 1)
        })
    }
}

impl SessionRepository {
    pub fn list_by_provider(&self, provider_id: &str) -> Result<Vec<ChatSession>> {
        let provider_id = provider_id.to_string();
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id, name, provider_id, created_at, updated_at FROM chat_sessions WHERE provider_id = ?1 ORDER BY updated_at DESC")?;
            let rows = stmt.query_map(params![provider_id], |row| ChatSession::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }
}

#[derive(Clone)]
pub struct AgentSessionRepository {
    conn: SqliteConnection,
}

const AGENT_SESSION_KIND: &str = "agent";
const AGENT_PROVIDER_ID: &str = "agent_runtime";

impl AgentSessionRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn save_snapshot(
        &self,
        uid: &str,
        title: &str,
        snapshot_json: &str,
    ) -> Result<AgentSession> {
        let uid = uid.to_string();
        let title = title.to_string();
        let snapshot_json = snapshot_json.to_string();
        if let Some(existing) = self.get_by_uid(&uid)? {
            let updated_at = now();
            self.conn.with_connection(|conn| {
                conn.execute(
                    "UPDATE chat_sessions
                     SET name = ?1, snapshot_json = ?2, updated_at = ?3
                     WHERE id = ?4",
                    params![title, snapshot_json, updated_at, existing.id],
                )?;
                Ok(())
            })?;
            return self
                .get_by_uid(&uid)?
                .ok_or_else(|| anyhow::anyhow!("Agent session disappeared after update: {uid}"));
        }

        let now = now();
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO chat_sessions
                 (name, provider_id, session_kind, uid, snapshot_json, archived, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6)",
                params![
                    title,
                    AGENT_PROVIDER_ID,
                    AGENT_SESSION_KIND,
                    uid,
                    snapshot_json,
                    now
                ],
            )?;
            let mut stmt = conn.prepare(
                "SELECT
                   id,
                   COALESCE(uid, CAST(id AS TEXT)) AS uid,
                   name AS title,
                   COALESCE(snapshot_json, '') AS snapshot_json,
                   archived,
                   created_at,
                   updated_at
                 FROM chat_sessions WHERE session_kind = ?1 AND uid = ?2",
            )?;
            let session = stmt.query_row(params![AGENT_SESSION_KIND, uid], AgentSession::from_row)?;
            Ok(session)
        })
    }

    pub fn get_by_uid(&self, uid: &str) -> Result<Option<AgentSession>> {
        let uid = uid.to_string();
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT
                   id,
                   COALESCE(uid, CAST(id AS TEXT)) AS uid,
                   name AS title,
                   COALESCE(snapshot_json, '') AS snapshot_json,
                   archived,
                   created_at,
                   updated_at
                 FROM chat_sessions
                 WHERE (session_kind = ?1 AND uid = ?2)
                    OR (session_kind != ?1 AND CAST(id AS TEXT) = ?2)
                 ORDER BY CASE WHEN session_kind = ?1 THEN 0 ELSE 1 END
                 LIMIT 1",
            )?;
            let mut rows = stmt.query(params![AGENT_SESSION_KIND, uid])?;
            if let Some(row) = rows.next()? {
                Ok(Some(AgentSession::from_row(row)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn list_by_archived(&self, archived: bool) -> Result<Vec<AgentSession>> {
        let archived = if archived { 1i32 } else { 0i32 };
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT
                   id,
                   COALESCE(uid, CAST(id AS TEXT)) AS uid,
                   name AS title,
                   COALESCE(snapshot_json, '') AS snapshot_json,
                   archived,
                   created_at,
                   updated_at
                 FROM chat_sessions
                 WHERE archived = ?1
                 ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map(params![archived], AgentSession::from_row)?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    pub fn delete_by_uid(&self, uid: &str) -> Result<()> {
        let uid = uid.to_string();
        self.conn.with_connection(|conn| {
            conn.execute(
                "DELETE FROM chat_sessions
                 WHERE (session_kind = ?1 AND uid = ?2)
                    OR (session_kind != ?1 AND CAST(id AS TEXT) = ?2)",
                params![AGENT_SESSION_KIND, uid],
            )?;
            Ok(())
        })
    }

    pub fn rename_by_uid(&self, uid: &str, title: &str) -> Result<bool> {
        self.update_title(uid, title).map(|count| count > 0)
    }

    pub fn set_archived_by_uid(&self, uid: &str, archived: bool) -> Result<bool> {
        let uid = uid.to_string();
        let archived = if archived { 1i32 } else { 0i32 };
        let updated_at = now();
        self.conn.with_connection(|conn| {
            let count = conn.execute(
                "UPDATE chat_sessions
                 SET archived = ?1, updated_at = ?2
                 WHERE (session_kind = ?3 AND uid = ?4)
                    OR (session_kind != ?3 AND CAST(id AS TEXT) = ?4)",
                params![archived, updated_at, AGENT_SESSION_KIND, uid],
            )?;
            Ok(count > 0)
        })
    }

    pub fn is_legacy_chat_uid(&self, uid: &str) -> Result<bool> {
        let uid = uid.to_string();
        self.conn.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM chat_sessions
                    WHERE session_kind != ?1 AND CAST(id AS TEXT) = ?2
                )",
                params![AGENT_SESSION_KIND, uid],
                |row| row.get(0),
            )?;
            Ok(exists == 1)
        })
    }

    fn update_title(&self, uid: &str, title: &str) -> Result<usize> {
        let uid = uid.to_string();
        let title = title.to_string();
        let updated_at = now();
        self.conn.with_connection(|conn| {
            let count = conn.execute(
                "UPDATE chat_sessions
                 SET name = ?1, updated_at = ?2
                 WHERE (session_kind = ?3 AND uid = ?4)
                    OR (session_kind != ?3 AND CAST(id AS TEXT) = ?4)",
                params![title, updated_at, AGENT_SESSION_KIND, uid],
            )?;
            Ok(count)
        })
    }
}

impl Repository for AgentSessionRepository {
    type Entity = AgentSession;

    fn entity_type(&self) -> SharedString {
        SharedString::from("AgentSession")
    }

    fn insert(&self, item: &mut Self::Entity) -> Result<i64> {
        let session = self.save_snapshot(&item.uid, &item.title, &item.snapshot_json)?;
        item.id = session.id;
        item.created_at = session.created_at;
        item.updated_at = session.updated_at;
        Ok(session.id)
    }

    fn update(&self, item: &Self::Entity) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE chat_sessions
                 SET uid = ?1, name = ?2, snapshot_json = ?3, archived = ?4, updated_at = ?5
                 WHERE id = ?6 AND session_kind = ?7",
                params![
                    item.uid,
                    item.title,
                    item.snapshot_json,
                    if item.archived { 1i32 } else { 0i32 },
                    now(),
                    item.id,
                    AGENT_SESSION_KIND
                ],
            )?;
            Ok(())
        })
    }

    fn delete(&self, id: i64) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "DELETE FROM chat_sessions WHERE id = ?1 AND session_kind = ?2",
                params![id, AGENT_SESSION_KIND],
            )?;
            Ok(())
        })
    }

    fn get(&self, id: i64) -> Result<Option<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT
                   id,
                   COALESCE(uid, CAST(id AS TEXT)) AS uid,
                   name AS title,
                   COALESCE(snapshot_json, '') AS snapshot_json,
                   archived,
                   created_at,
                   updated_at
                 FROM chat_sessions WHERE id = ?1 AND session_kind = ?2",
            )?;
            let mut rows = stmt.query(params![id, AGENT_SESSION_KIND])?;
            if let Some(row) = rows.next()? {
                Ok(Some(AgentSession::from_row(row)?))
            } else {
                Ok(None)
            }
        })
    }

    fn list(&self) -> Result<Vec<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT
                   id,
                   COALESCE(uid, CAST(id AS TEXT)) AS uid,
                   name AS title,
                   COALESCE(snapshot_json, '') AS snapshot_json,
                   archived,
                   created_at,
                   updated_at
                 FROM chat_sessions
                 ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map([], AgentSession::from_row)?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    fn count(&self) -> Result<i64> {
        self.conn.with_connection(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM chat_sessions", [], |row| row.get(0))?;
            Ok(count)
        })
    }

    fn exists(&self, id: i64) -> Result<bool> {
        self.conn.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM chat_sessions WHERE id = ?1 AND session_kind = ?2
                )",
                params![id, AGENT_SESSION_KIND],
                |row| row.get(0),
            )?;
            Ok(exists == 1)
        })
    }
}

#[derive(Clone)]
pub struct MessageRepository {
    conn: SqliteConnection,
}

impl MessageRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }
}

impl Repository for MessageRepository {
    type Entity = ChatMessage;

    fn entity_type(&self) -> SharedString {
        SharedString::from("ChatMessage")
    }

    fn insert(&self, item: &mut Self::Entity) -> Result<i64> {
        let session_id = item.session_id;
        let role = item.role.clone();
        let content = item.content.clone();
        let created_at = item.created_at;

        let id = self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO chat_messages (session_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![session_id, role, content, created_at],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        item.id = id;
        Ok(id)
    }

    fn update(&self, item: &Self::Entity) -> Result<()> {
        let id = item.id;
        let session_id = item.session_id;
        let role = item.role.clone();
        let content = item.content.clone();

        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE chat_messages SET session_id = ?1, role = ?2, content = ?3 WHERE id = ?4",
                params![session_id, role, content, id],
            )?;
            Ok(())
        })
    }

    fn delete(&self, id: i64) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute("DELETE FROM chat_messages WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    fn get(&self, id: i64) -> Result<Option<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, role, content, created_at FROM chat_messages WHERE id = ?1",
            )?;
            let mut rows = stmt.query(params![id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(ChatMessage::from_row(row)?))
            } else {
                Ok(None)
            }
        })
    }

    fn list(&self) -> Result<Vec<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id, session_id, role, content, created_at FROM chat_messages ORDER BY created_at ASC")?;
            let rows = stmt.query_map([], |row| ChatMessage::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    fn count(&self) -> Result<i64> {
        self.conn.with_connection(|conn| {
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM chat_messages", [], |row| row.get(0))?;
            Ok(count)
        })
    }

    fn exists(&self, id: i64) -> Result<bool> {
        self.conn.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM chat_messages WHERE id = ?1)",
                params![id],
                |row| row.get(0),
            )?;
            Ok(exists == 1)
        })
    }
}

impl MessageRepository {
    pub fn list_by_session(&self, session_id: i64) -> Result<Vec<ChatMessage>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id, session_id, role, content, created_at FROM chat_messages WHERE session_id = ?1 ORDER BY created_at ASC")?;
            let rows = stmt.query_map(params![session_id], |row| ChatMessage::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    pub fn list_recent(&self, limit: i32) -> Result<Vec<ChatMessage>> {
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id, session_id, role, content, created_at FROM chat_messages ORDER BY created_at DESC LIMIT ?1")?;
            let rows = stmt.query_map(params![limit], |row| ChatMessage::from_row(row))?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    pub fn delete_by_session(&self, session_id: i64) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "DELETE FROM chat_messages WHERE session_id = ?1",
                params![session_id],
            )?;
            Ok(())
        })
    }
}
