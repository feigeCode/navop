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

/// 侧栏会话列表需要的一行，**不含** `snapshot_json` 本体。
///
/// 列表只用得到快照里的两个顶层字段（工作区归属、外部 agent 来源）。把整列
/// `snapshot_json` 读上来、再在调用侧反序列化，代价与快照体积成正比（实测本机
/// 未归档 356 行合计 76.8MB、单条最大 19.2MB；debug 下光反序列化就要 1.6s），
/// 而列表刷新跑在 UI 线程上且触发频繁（切会话 / 每轮结束 / 连接就绪都会跑）。
/// 因此这两个字段下推到 SQL 层用 `json_extract` 提取：快照本体不再进入进程内存。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionSummaryRow {
    pub id: i64,
    pub uid: String,
    pub title: String,
    pub archived: bool,
    pub created_at: i64,
    pub updated_at: i64,
    /// 快照里的 `workspace_root`。缺失 / 空串 / 快照不是合法 JSON 时为 `None`。
    pub workspace_root: Option<String>,
    /// 快照里的 `acp.agent_id`（这段会话由外部 agent 承载）。判定同上。
    pub acp_agent_id: Option<String>,
}

/// 正文命中的一条会话。
///
/// 只带「够在列表里认出这一条」的三样东西：会话 uid、标题、命中处上下文。
/// 不带消息序号 / 偏移——调用方拿到的是同一段查询词，跳过去以后由页面自己的
/// 搜索去定位高亮，比在这里维护一套「第几条消息第几个字」的坐标更不容易走样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionBodyMatch {
    pub uid: String,
    pub title: String,
    /// 命中处上下文：单行、空白已折叠、两端按需带省略号。
    pub snippet: String,
}

/// 片段每个方向取多少个字符。
const BODY_SNIPPET_RADIUS: usize = 48;

/// `LIKE` 一次最多扫多少行。命中率低时（大量假阳性）不能只按 limit 取行，
/// 否则返回条数会莫名其妙地少；多扫几倍再截断，代价仍然有界。
const BODY_SEARCH_SCAN_FACTOR: usize = 8;

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
            if existing.title == title && existing.snapshot_json == snapshot_json {
                return Ok(existing);
            }
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

    /// 列出摘要行（侧栏用），**不读** `snapshot_json` 本体。
    ///
    /// 过滤与排序与 [`Self::list_by_archived`] 完全一致，只是把来自快照的两个字段
    /// 下推到 SQL 层提取。调用侧因此不必把几十 MB 的快照 JSON 搬进内存、更不必逐行
    /// 反序列化。
    ///
    /// `json_valid` 守卫是必需的，不是防御性冗余：本表历史数据里存在
    /// `snapshot_json` 为空串或非合法 JSON 的行（早期 chat 会话），裸
    /// `json_extract` 遇到它们会直接报 `malformed JSON`，**整条查询失败**，
    /// 侧栏会话列表会整体变空。
    pub fn list_summary_rows_by_archived(
        &self,
        archived: bool,
    ) -> Result<Vec<AgentSessionSummaryRow>> {
        let archived = if archived { 1i32 } else { 0i32 };
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT
                   id,
                   COALESCE(uid, CAST(id AS TEXT)) AS uid,
                   name AS title,
                   archived,
                   created_at,
                   updated_at,
                   CASE WHEN json_valid(snapshot_json)
                        THEN json_extract(snapshot_json, '$.workspace_root') END AS workspace_root,
                   CASE WHEN json_valid(snapshot_json)
                        THEN json_extract(snapshot_json, '$.acp.agent_id') END AS acp_agent_id
                 FROM chat_sessions
                 WHERE archived = ?1
                 ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map(params![archived], |row| {
                let archived: i32 = row.get("archived")?;
                Ok(AgentSessionSummaryRow {
                    id: row.get("id")?,
                    uid: row.get("uid")?,
                    title: row.get("title")?,
                    archived: archived != 0,
                    created_at: row.get("created_at")?,
                    updated_at: row.get("updated_at")?,
                    workspace_root: row.get("workspace_root")?,
                    acp_agent_id: row.get("acp_agent_id")?,
                })
            })?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    /// 按正文子串搜会话（大小写不敏感，最近更新的在前）。
    ///
    /// 正文只活在 `snapshot_json` 里，没有可查询的独立列，所以先用 `LIKE` 让
    /// SQLite 粗筛行——这违反了「快照本体不进内存」那条约定的边角：只有筛出来的
    /// 行才把快照读上来。然后在 Rust 侧把 JSON 拆开、**按消息逐条**找命中：
    /// `LIKE` 定位不到片段边界，JSON 转义还会造成假阳性（`\n` 之类），逐条找
    /// 既拿到干净的上下文，也顺手丢掉这些假阳性。
    ///
    /// 只看消息文本（用户 / 助手 / 系统 / 摘要），不看工具调用与观测：那两类的
    /// JSON 又大又杂，参数里随便一个路径就能命中，噪音远大于用处。
    pub fn search_snapshots(
        &self,
        needle: &str,
        limit: usize,
    ) -> Result<Vec<AgentSessionBodyMatch>> {
        let needle = needle.trim();
        if needle.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let pattern = format!("%{}%", escape_like_pattern(needle));
        let scan_limit = (limit.saturating_mul(BODY_SEARCH_SCAN_FACTOR)) as i64;
        self.conn.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT COALESCE(uid, CAST(id AS TEXT)) AS uid, name AS title, snapshot_json
                 FROM chat_sessions
                 WHERE snapshot_json LIKE ?1 ESCAPE '\\'
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![pattern, scan_limit], |row| {
                Ok((
                    row.get::<_, String>("uid")?,
                    row.get::<_, String>("title")?,
                    row.get::<_, String>("snapshot_json")?,
                ))
            })?;
            let mut results = Vec::new();
            for row in rows {
                let (uid, title, snapshot_json) = row?;
                let Some(snippet) = snapshot_body_snippet(&snapshot_json, needle) else {
                    continue;
                };
                results.push(AgentSessionBodyMatch {
                    uid,
                    title,
                    snippet,
                });
                if results.len() >= limit {
                    break;
                }
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

/// 转义 `LIKE` 模式里的通配符：用户打进来的 `%` / `_` / `\` 是字面量。
fn escape_like_pattern(needle: &str) -> String {
    let mut escaped = String::with_capacity(needle.len());
    for ch in needle.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// 在快照的历史里找第一条含 `needle` 的消息，返回命中处的上下文片段。
fn snapshot_body_snippet(snapshot_json: &str, needle: &str) -> Option<String> {
    message_texts_from_snapshot(snapshot_json)
        .into_iter()
        .find_map(|text| snippet_around(&text, needle))
}

/// 快照里的消息文本（用户 / 助手 / 助手+reasoning / 系统 / 摘要）。
fn message_texts_from_snapshot(snapshot_json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(snapshot_json) else {
        return Vec::new();
    };
    let Some(history) = value.get("history").and_then(|history| history.as_array()) else {
        return Vec::new();
    };
    let mut texts = Vec::new();
    for item in history {
        match item {
            // `HistoryItem::Assistant` / `System` 这类单值变体序列化成裸字符串。
            serde_json::Value::String(text) => texts.push(text.clone()),
            serde_json::Value::Object(map) => {
                let text = ["User", "AssistantWithReasoning", "ContextSummary"]
                    .iter()
                    .find_map(|key| map.get(*key))
                    .and_then(|inner| inner.get("text"))
                    .and_then(|text| text.as_str());
                if let Some(text) = text {
                    texts.push(text.to_string());
                }
            }
            _ => {}
        }
    }
    texts
}

/// 命中处前后各取 [`BODY_SNIPPET_RADIUS`] 个字符，压成一行。
fn snippet_around(text: &str, needle: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let byte_index = lower.find(&needle.to_lowercase())?;
    // 大小写折叠在个别字符上会改变字节数，先把下标落到字符边界以内再按字符切片。
    let byte_index = floor_char_boundary(text, byte_index.min(text.len()));
    let match_start = text[..byte_index].chars().count();
    let match_len = needle.chars().count();
    let chars: Vec<char> = text.chars().collect();
    let start = match_start.saturating_sub(BODY_SNIPPET_RADIUS);
    let end = (match_start + match_len + BODY_SNIPPET_RADIUS).min(chars.len());
    // 片段是列表右侧的一行说明文字：换行与连续空白折叠成单个空格，否则会被
    // 截断成一团乱码。
    let mut snippet = chars[start..end]
        .iter()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if start > 0 {
        snippet.insert_str(0, "…");
    }
    if end < chars.len() {
        snippet.push('…');
    }
    Some(snippet)
}

/// 向前找到最近的字符边界：按字节下标切片时不能切在 UTF-8 中间。
fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
#[path = "body_search_tests.rs"]
mod body_search_tests;
