//! 持久化的 SQL 执行历史，供数据库视图及其他 SQL 执行入口复用。

use anyhow::Result;
use rusqlite::{Row, params, params_from_iter};

use crate::storage::connection::SqliteConnection;
use crate::storage::manager::now;

const MAX_RECORDS_PER_CONNECTION: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlExecutionStatus {
    Success,
    Error,
}

impl SqlExecutionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "success" => Some(Self::Success),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlExecutionHistoryEntry {
    pub id: Option<i64>,
    pub connection_id: String,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub status: SqlExecutionStatus,
    pub sql: String,
    pub summary: String,
    pub details: Vec<String>,
    pub affected_rows: u64,
    pub returned_rows: Option<usize>,
    pub elapsed_ms: u128,
    pub executed_at: i64,
}

impl SqlExecutionHistoryEntry {
    pub fn new(connection_id: String, status: SqlExecutionStatus, sql: String) -> Self {
        Self {
            id: None,
            connection_id,
            database: None,
            schema: None,
            status,
            sql,
            summary: String::new(),
            details: Vec::new(),
            affected_rows: 0,
            returned_rows: None,
            elapsed_ms: 0,
            executed_at: now(),
        }
    }
}

#[derive(Clone)]
pub struct SqlExecutionHistoryRepository {
    conn: SqliteConnection,
}

impl SqlExecutionHistoryRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn record(&self, entry: &SqlExecutionHistoryEntry) -> Result<i64> {
        let details = serde_json::to_string(&entry.details)?;
        let affected_rows = i64::try_from(entry.affected_rows).unwrap_or(i64::MAX);
        let returned_rows = entry
            .returned_rows
            .map(|rows| i64::try_from(rows).unwrap_or(i64::MAX));
        let elapsed_ms = i64::try_from(entry.elapsed_ms).unwrap_or(i64::MAX);

        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO sql_execution_history (
                    connection_id, database_name, schema_name, status, sql, summary, details,
                    affected_rows, returned_rows, elapsed_ms, executed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    entry.connection_id,
                    entry.database,
                    entry.schema,
                    entry.status.as_str(),
                    entry.sql,
                    entry.summary,
                    details,
                    affected_rows,
                    returned_rows,
                    elapsed_ms,
                    entry.executed_at,
                ],
            )?;
            let id = conn.last_insert_rowid();
            conn.execute(
                "DELETE FROM sql_execution_history
                 WHERE connection_id = ?1
                   AND id NOT IN (
                       SELECT id FROM sql_execution_history
                       WHERE connection_id = ?1
                       ORDER BY executed_at DESC, id DESC
                       LIMIT ?2
                   )",
                params![
                    entry.connection_id,
                    i64::try_from(MAX_RECORDS_PER_CONNECTION).unwrap_or(i64::MAX)
                ],
            )?;
            Ok(id)
        })
    }

    pub fn list(
        &self,
        connection_ids: &[String],
        limit: usize,
    ) -> Result<Vec<SqlExecutionHistoryEntry>> {
        if connection_ids.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let placeholders = std::iter::repeat_n("?", connection_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "{BASE_SELECT}
             WHERE connection_id IN ({placeholders})
             ORDER BY executed_at DESC, id DESC
             LIMIT ?"
        );
        let mut parameters = connection_ids.to_vec();
        parameters.push(limit.to_string());

        self.conn.with_connection(|conn| {
            let mut statement = conn.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(parameters.iter()), row_to_entry)?;
            let mut entries = Vec::new();
            for row in rows {
                entries.push(row?);
            }
            Ok(entries)
        })
    }

    pub fn clear(&self, connection_ids: &[String]) -> Result<()> {
        if connection_ids.is_empty() {
            return Ok(());
        }
        let placeholders = std::iter::repeat_n("?", connection_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql =
            format!("DELETE FROM sql_execution_history WHERE connection_id IN ({placeholders})");
        self.conn.with_connection(|conn| {
            conn.execute(&sql, params_from_iter(connection_ids.iter()))?;
            Ok(())
        })
    }
}

const BASE_SELECT: &str = "SELECT
    id, connection_id, database_name, schema_name, status, sql, summary, details,
    affected_rows, returned_rows, elapsed_ms, executed_at
    FROM sql_execution_history";

fn row_to_entry(row: &Row<'_>) -> rusqlite::Result<SqlExecutionHistoryEntry> {
    let status: String = row.get(4)?;
    let details: String = row.get(7)?;
    let affected_rows: i64 = row.get(8)?;
    let returned_rows: Option<i64> = row.get(9)?;
    let elapsed_ms: i64 = row.get(10)?;

    Ok(SqlExecutionHistoryEntry {
        id: row.get(0)?,
        connection_id: row.get(1)?,
        database: row.get(2)?,
        schema: row.get(3)?,
        status: SqlExecutionStatus::parse(&status).ok_or_else(|| {
            to_sql_error(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown SQL execution status: {status}"),
            ))
        })?,
        sql: row.get(5)?,
        summary: row.get(6)?,
        details: serde_json::from_str(&details).map_err(to_sql_error)?,
        affected_rows: u64::try_from(affected_rows).map_err(to_sql_error)?,
        returned_rows: returned_rows
            .map(usize::try_from)
            .transpose()
            .map_err(to_sql_error)?,
        elapsed_ms: u128::try_from(elapsed_ms).map_err(to_sql_error)?,
        executed_at: row.get(11)?,
    })
}

fn to_sql_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

#[cfg(test)]
#[path = "sql_execution_history_tests.rs"]
mod tests;
