//! 终端成功命令历史存储。

use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::storage::connection::SqliteConnection;
use crate::storage::manager::now;
use crate::storage::traits::Entity;

mod repository_impl;
#[cfg(test)]
mod terminal_command_history_tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalHistoryScope {
    pub scope_key: String,
    pub scope_kind: String,
    pub connection_id: Option<i64>,
}

impl TerminalHistoryScope {
    pub fn local() -> Self {
        Self {
            scope_key: "local".to_string(),
            scope_kind: "local".to_string(),
            connection_id: None,
        }
    }

    pub fn ssh(connection_id: i64) -> Self {
        Self {
            scope_key: format!("ssh:{connection_id}"),
            scope_kind: "ssh".to_string(),
            connection_id: Some(connection_id),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalCommandHistorySort {
    MostUsed,
    Latest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TerminalCommandHistory {
    pub id: Option<i64>,
    pub scope_key: String,
    pub scope_kind: String,
    pub connection_id: Option<i64>,
    pub command: String,
    pub use_count: i64,
    pub favorite: bool,
    pub first_used_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub last_exit_code: Option<i32>,
    pub cwd: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

impl Entity for TerminalCommandHistory {
    fn id(&self) -> Option<i64> {
        self.id
    }

    fn created_at(&self) -> i64 {
        self.created_at.unwrap_or(0)
    }

    fn updated_at(&self) -> i64 {
        self.updated_at.unwrap_or(0)
    }
}

impl TerminalCommandHistory {
    pub fn new(scope: &TerminalHistoryScope, command: String) -> Self {
        let ts = now();
        Self {
            id: None,
            scope_key: scope.scope_key.clone(),
            scope_kind: scope.scope_kind.clone(),
            connection_id: scope.connection_id,
            command,
            use_count: 1,
            favorite: false,
            first_used_at: Some(ts),
            last_used_at: Some(ts),
            last_exit_code: Some(0),
            cwd: None,
            created_at: Some(ts),
            updated_at: Some(ts),
        }
    }
}

#[derive(Clone)]
pub struct TerminalCommandHistoryRepository {
    conn: SqliteConnection,
}

impl TerminalCommandHistoryRepository {
    pub fn new(conn: SqliteConnection) -> Self {
        Self { conn }
    }

    pub fn record_success(
        &self,
        scope: &TerminalHistoryScope,
        command: &str,
        cwd: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<Option<TerminalCommandHistory>> {
        if exit_code != Some(0) {
            return Ok(None);
        }
        let Some(command) = normalize_command(command) else {
            return Ok(None);
        };
        let ts = now();
        self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO terminal_command_history
                 (scope_key, scope_kind, connection_id, command, use_count, favorite,
                  first_used_at, last_used_at, last_exit_code, cwd, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 1, 0, ?5, ?5, ?6, ?7, ?5, ?5)
                 ON CONFLICT(scope_key, command) DO UPDATE SET
                    scope_kind = excluded.scope_kind,
                    connection_id = excluded.connection_id,
                    use_count = terminal_command_history.use_count + 1,
                    last_used_at = excluded.last_used_at,
                    last_exit_code = excluded.last_exit_code,
                    cwd = COALESCE(excluded.cwd, terminal_command_history.cwd),
                    updated_at = excluded.updated_at",
                params![
                    scope.scope_key,
                    scope.scope_kind,
                    scope.connection_id,
                    command,
                    ts,
                    exit_code.expect("successful command must have exit code"),
                    cwd
                ],
            )?;
            select_by_scope_command(conn, scope, &command)
        })
    }

    pub fn list(
        &self,
        scope: &TerminalHistoryScope,
        sort: TerminalCommandHistorySort,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TerminalCommandHistory>> {
        let order = sort_order(sort);
        let limit = limit.max(1) as i64;
        let query = query.and_then(normalize_command);
        self.conn.with_connection(|conn| {
            let mut items = Vec::new();
            if let Some(query) = query {
                let sql = format!(
                    "{BASE_SELECT} WHERE scope_key = ?1 AND lower(command) LIKE ?2 {order} LIMIT ?3"
                );
                let pattern = format!("%{}%", query.to_lowercase());
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![scope.scope_key, pattern, limit], row_to_item)?;
                for row in rows {
                    items.push(row?);
                }
            } else {
                let sql = format!("{BASE_SELECT} WHERE scope_key = ?1 {order} LIMIT ?2");
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![scope.scope_key, limit], row_to_item)?;
                for row in rows {
                    items.push(row?);
                }
            }
            Ok(items)
        })
    }

    pub fn suggestions(
        &self,
        scope: &TerminalHistoryScope,
        query: &str,
        limit: usize,
    ) -> Result<Vec<String>> {
        let Some(query) = normalize_command(query) else {
            return Ok(Vec::new());
        };
        Ok(self
            .list(
                scope,
                TerminalCommandHistorySort::MostUsed,
                Some(&query),
                limit,
            )?
            .into_iter()
            .map(|item| item.command)
            .collect())
    }

    pub fn toggle_favorite(&self, id: i64) -> Result<bool> {
        self.conn.with_connection(|conn| {
            let current: i64 = conn.query_row(
                "SELECT favorite FROM terminal_command_history WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )?;
            let next = if current == 0 { 1 } else { 0 };
            conn.execute(
                "UPDATE terminal_command_history SET favorite = ?1, updated_at = ?2 WHERE id = ?3",
                params![next, now(), id],
            )?;
            Ok(next != 0)
        })
    }
}

pub(super) const BASE_SELECT: &str =
    "SELECT id, scope_key, scope_kind, connection_id, command, use_count,
favorite, first_used_at, last_used_at, last_exit_code, cwd, created_at, updated_at
FROM terminal_command_history";

fn normalize_command(command: &str) -> Option<String> {
    let command = command.trim();
    (!command.is_empty()).then(|| command.to_string())
}

fn sort_order(sort: TerminalCommandHistorySort) -> &'static str {
    match sort {
        TerminalCommandHistorySort::MostUsed => {
            "ORDER BY favorite DESC, use_count DESC, last_used_at DESC, id DESC"
        }
        TerminalCommandHistorySort::Latest => "ORDER BY favorite DESC, last_used_at DESC, id DESC",
    }
}

fn select_by_scope_command(
    conn: &rusqlite::Connection,
    scope: &TerminalHistoryScope,
    command: &str,
) -> Result<Option<TerminalCommandHistory>> {
    conn.query_row(
        &format!("{BASE_SELECT} WHERE scope_key = ?1 AND command = ?2"),
        params![scope.scope_key, command],
        row_to_item,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<TerminalCommandHistory> {
    Ok(TerminalCommandHistory {
        id: Some(row.get("id")?),
        scope_key: row.get("scope_key")?,
        scope_kind: row.get("scope_kind")?,
        connection_id: row.get("connection_id")?,
        command: row.get("command")?,
        use_count: row.get("use_count")?,
        favorite: row.get::<_, i64>("favorite")? != 0,
        first_used_at: row.get("first_used_at")?,
        last_used_at: row.get("last_used_at")?,
        last_exit_code: row.get("last_exit_code")?,
        cwd: row.get("cwd")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}
