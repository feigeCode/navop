//! Agent 会话的上下文用量流水。
//!
//! 与 `chat_sessions.snapshot_json.context_tokens` 分工明确：快照里那份是
//! 「这条会话现在用了多少」的**当前值**，每轮被覆盖，用来在会话列表 / 圆环上显示；
//! 本模块的 `agent_usage_samples` 是**只追加**的采样流水，用来画趋势与按天汇总。
//!
//! 采样点由调用侧决定（ACP 侧每次报文用量、本地侧每轮结束），这里只负责
//! 「写一条」和「按会话 / 时间窗读回来」，并在写入时做每会话条数裁剪——
//! 长会话聊上几百轮不该让流水无限长。

use anyhow::Result;
use rusqlite::{Row, params};

use crate::storage::connection::SqliteConnection;

/// 每个会话保留的采样条数上限。
///
/// 取 200：足够画一条看得出走势的曲线（按每轮一条算，覆盖 200 轮），
/// 又不会让单会话的流水在库里无界增长。
pub const MAX_SAMPLES_PER_SESSION: usize = 200;

/// 一次上下文用量采样。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentUsageSample {
    pub id: Option<i64>,
    /// 会话 uid（与 `chat_sessions.uid` 对齐）。
    pub uid: String,
    /// 采样时刻的上下文占用（token）。
    pub used: u64,
    /// 模型上下文窗口；未知为 `None`（不猜）。
    pub window: Option<u64>,
    /// 采样时的模型 id。
    pub model: Option<String>,
    /// 采样时刻（Unix 秒）。
    pub recorded_at: i64,
}

impl AgentUsageSample {
    pub fn new(uid: impl Into<String>, used: u64, recorded_at: i64) -> Self {
        Self {
            id: None,
            uid: uid.into(),
            used,
            window: None,
            model: None,
            recorded_at,
        }
    }
}

#[derive(Clone)]
pub struct AgentUsageRepository {
    conn: SqliteConnection,
}

const BASE_SELECT: &str = "SELECT
    id, uid, used, window, model, recorded_at
    FROM agent_usage_samples";

impl AgentUsageRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    /// 追加一条采样，并裁掉该会话超出的旧样本。
    pub fn record(&self, sample: &AgentUsageSample) -> Result<i64> {
        let used = i64::try_from(sample.used).unwrap_or(i64::MAX);
        let window = sample
            .window
            .map(|window| i64::try_from(window).unwrap_or(i64::MAX));
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO agent_usage_samples (uid, used, window, model, recorded_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![sample.uid, used, window, sample.model, sample.recorded_at],
            )?;
            let id = conn.last_insert_rowid();
            conn.execute(
                "DELETE FROM agent_usage_samples
                 WHERE uid = ?1
                   AND id NOT IN (
                       SELECT id FROM agent_usage_samples
                       WHERE uid = ?1
                       ORDER BY recorded_at DESC, id DESC
                       LIMIT ?2
                   )",
                params![
                    sample.uid,
                    i64::try_from(MAX_SAMPLES_PER_SESSION).unwrap_or(i64::MAX)
                ],
            )?;
            Ok(id)
        })
    }

    /// 该会话最近一条采样；没有则 `None`。
    pub fn latest_for_uid(&self, uid: &str) -> Result<Option<AgentUsageSample>> {
        let sql =
            format!("{BASE_SELECT} WHERE uid = ?1 ORDER BY recorded_at DESC, id DESC LIMIT 1");
        self.conn.with_connection(|conn| {
            let mut statement = conn.prepare(&sql)?;
            let mut rows = statement.query(params![uid])?;
            match rows.next()? {
                Some(row) => Ok(Some(row_to_sample(row)?)),
                None => Ok(None),
            }
        })
    }

    /// `since`（含）之后的全量采样，按时间正序。
    ///
    /// 汇总与绘图都需要按时间顺序扫一遍，交给调用侧聚合：SQL 层只做过滤，
    /// 免得每种图表都要改一次 SQL。
    pub fn list_since(&self, since: i64) -> Result<Vec<AgentUsageSample>> {
        let sql = format!("{BASE_SELECT} WHERE recorded_at >= ?1 ORDER BY recorded_at ASC, id ASC");
        self.conn.with_connection(|conn| {
            let mut statement = conn.prepare(&sql)?;
            let rows = statement.query_map(params![since], row_to_sample)?;
            let mut samples = Vec::new();
            for row in rows {
                samples.push(row?);
            }
            Ok(samples)
        })
    }

    /// 每个会话最近一条采样，按采样时间倒序。
    ///
    /// 会话列表要「每个会话一行」而不是「每次采样一行」，用窗口函数直接取每组第一条，
    /// 避免把整张流水拉进内存再分组。
    pub fn latest_per_session(&self) -> Result<Vec<AgentUsageSample>> {
        let sql = "SELECT id, uid, used, window, model, recorded_at FROM (
                       SELECT id, uid, used, window, model, recorded_at,
                              ROW_NUMBER() OVER (
                                  PARTITION BY uid
                                  ORDER BY recorded_at DESC, id DESC
                              ) AS rn
                       FROM agent_usage_samples
                   )
                   WHERE rn = 1
                   ORDER BY recorded_at DESC, id DESC";
        self.conn.with_connection(|conn| {
            let mut statement = conn.prepare(sql)?;
            let rows = statement.query_map([], row_to_sample)?;
            let mut samples = Vec::new();
            for row in rows {
                samples.push(row?);
            }
            Ok(samples)
        })
    }

    /// 删除一条会话的全部采样（会话被真删时用）。
    pub fn clear_uid(&self, uid: &str) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "DELETE FROM agent_usage_samples WHERE uid = ?1",
                params![uid],
            )?;
            Ok(())
        })
    }
}

fn row_to_sample(row: &Row<'_>) -> rusqlite::Result<AgentUsageSample> {
    let used: i64 = row.get(2)?;
    let window: Option<i64> = row.get(3)?;
    Ok(AgentUsageSample {
        id: row.get(0)?,
        uid: row.get(1)?,
        used: u64::try_from(used).unwrap_or(0),
        window: window.and_then(|window| u64::try_from(window).ok()),
        model: row.get(4)?,
        recorded_at: row.get(5)?,
    })
}

#[cfg(test)]
#[path = "usage_history_tests.rs"]
mod tests;
