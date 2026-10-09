//! 未发送的输入草稿（每会话一条）。
//!
//! 与 `chat_sessions.snapshot_json` 里的草稿分工：快照那份随消息一起写盘，
//! 只有「会话已有历史」时才存在——刚开的新会话里写了半句话，快照没地方放。
//! 这张表独立于历史，**行的存在本身**就是「这条会话有草稿」，会话列表要标记
//! 时按 uid 直接查，不必把每条快照反序列化一遍（列表路径上这是致命的）。
//!
//! 附件（图片 base64）由调用侧序列化成 JSON 存进 `attachments`：这里只当
//! 不透明字符串搬运，图片格式怎么变都不用改这一层。

use anyhow::Result;
use rusqlite::{Row, params};

use crate::storage::connection::SqliteConnection;

/// 一条输入草稿。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerDraft {
    /// 会话 uid（与 `chat_sessions.uid` 对齐）。
    pub session_uid: String,
    /// 输入框里的文字原样。
    pub text: String,
    /// 附件 JSON 数组（图片 base64）。
    pub attachments: String,
    /// 最后编辑时刻（Unix 秒）。
    pub updated_at: i64,
}

impl ComposerDraft {
    pub fn new(
        session_uid: impl Into<String>,
        text: impl Into<String>,
        attachments: impl Into<String>,
        updated_at: i64,
    ) -> Self {
        Self {
            session_uid: session_uid.into(),
            text: text.into(),
            attachments: attachments.into(),
            updated_at,
        }
    }

    /// 文字与附件都空——什么都不留。
    ///
    /// 空草稿不写行：否则「清空输入框」会留下一行空记录，会话列表就会给一条
    /// 什么都没写的会话标上「有草稿」。
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.attachments_is_empty()
    }

    fn attachments_is_empty(&self) -> bool {
        matches!(self.attachments.trim(), "" | "[]")
    }
}

#[derive(Clone)]
pub struct ComposerDraftRepository {
    conn: SqliteConnection,
}

const BASE_SELECT: &str = "SELECT session_uid, text, attachments, updated_at
    FROM agent_composer_drafts";

impl ComposerDraftRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    /// 写入 / 覆盖一条草稿；空草稿则删行。
    pub fn save(&self, draft: &ComposerDraft) -> Result<()> {
        if draft.is_empty() {
            return self.clear(&draft.session_uid);
        }
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO agent_composer_drafts (session_uid, text, attachments, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(session_uid) DO UPDATE SET
                     text = excluded.text,
                     attachments = excluded.attachments,
                     updated_at = excluded.updated_at",
                params![
                    draft.session_uid,
                    draft.text,
                    draft.attachments,
                    draft.updated_at
                ],
            )?;
            Ok(())
        })
    }

    /// 读某会话的草稿；没有则 `None`。
    pub fn load(&self, uid: &str) -> Result<Option<ComposerDraft>> {
        let sql = format!("{BASE_SELECT} WHERE session_uid = ?1");
        self.conn.with_connection(|conn| {
            let mut statement = conn.prepare(&sql)?;
            let mut rows = statement.query(params![uid])?;
            match rows.next()? {
                Some(row) => Ok(Some(row_to_draft(row)?)),
                None => Ok(None),
            }
        })
    }

    /// 有草稿的会话 uid（最近编辑在前）。
    ///
    /// 只取 uid：会话列表只要一个集合来判断行上画不画标记，把正文与 base64
    /// 附件一起拉回来是白花的 IO。
    pub fn drafted_uids(&self) -> Result<Vec<String>> {
        self.conn.with_connection(|conn| {
            let mut statement = conn.prepare(
                "SELECT session_uid FROM agent_composer_drafts ORDER BY updated_at DESC",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            let mut uids = Vec::new();
            for row in rows {
                uids.push(row?);
            }
            Ok(uids)
        })
    }

    /// 删掉某会话的草稿（提交成功、会话被删时调用）。
    pub fn clear(&self, uid: &str) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "DELETE FROM agent_composer_drafts WHERE session_uid = ?1",
                params![uid],
            )?;
            Ok(())
        })
    }
}

fn row_to_draft(row: &Row<'_>) -> rusqlite::Result<ComposerDraft> {
    Ok(ComposerDraft {
        session_uid: row.get(0)?,
        text: row.get(1)?,
        attachments: row.get(2)?,
        updated_at: row.get(3)?,
    })
}

#[cfg(test)]
#[path = "composer_draft_tests.rs"]
mod tests;
