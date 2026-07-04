use anyhow::Result;
use gpui::SharedString;
use rusqlite::{OptionalExtension, params};

use super::{BASE_SELECT, TerminalCommandHistory, TerminalCommandHistoryRepository, row_to_item};
use crate::storage::manager::now;
use crate::storage::traits::Repository;

impl Repository for TerminalCommandHistoryRepository {
    type Entity = TerminalCommandHistory;

    fn entity_type(&self) -> SharedString {
        SharedString::from("TerminalCommandHistory")
    }

    fn insert(&self, item: &mut Self::Entity) -> Result<i64> {
        let ts = now();
        let favorite = if item.favorite { 1 } else { 0 };
        let id = self.conn.with_connection(|conn| {
            conn.execute(
                "INSERT INTO terminal_command_history
                 (scope_key, scope_kind, connection_id, command, use_count, favorite,
                  first_used_at, last_used_at, last_exit_code, cwd, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    item.scope_key,
                    item.scope_kind,
                    item.connection_id,
                    item.command,
                    item.use_count,
                    favorite,
                    item.first_used_at.unwrap_or(ts),
                    item.last_used_at.unwrap_or(ts),
                    item.last_exit_code,
                    item.cwd,
                    item.created_at.unwrap_or(ts),
                    ts
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })?;
        item.id = Some(id);
        item.updated_at = Some(ts);
        Ok(id)
    }

    fn update(&self, item: &Self::Entity) -> Result<()> {
        let id = item
            .id
            .ok_or_else(|| anyhow::anyhow!("Cannot update without ID"))?;
        let favorite = if item.favorite { 1 } else { 0 };
        self.conn.with_connection(|conn| {
            conn.execute(
                "UPDATE terminal_command_history
                 SET scope_key = ?1, scope_kind = ?2, connection_id = ?3, command = ?4,
                     use_count = ?5, favorite = ?6, first_used_at = ?7,
                     last_used_at = ?8, last_exit_code = ?9, cwd = ?10, updated_at = ?11
                 WHERE id = ?12",
                params![
                    item.scope_key,
                    item.scope_kind,
                    item.connection_id,
                    item.command,
                    item.use_count,
                    favorite,
                    item.first_used_at,
                    item.last_used_at,
                    item.last_exit_code,
                    item.cwd,
                    now(),
                    id
                ],
            )?;
            Ok(())
        })
    }

    fn delete(&self, id: i64) -> Result<()> {
        self.conn.with_connection(|conn| {
            conn.execute(
                "DELETE FROM terminal_command_history WHERE id = ?1",
                params![id],
            )?;
            Ok(())
        })
    }

    fn get(&self, id: i64) -> Result<Option<Self::Entity>> {
        self.conn.with_connection(|conn| {
            Ok(conn
                .query_row(
                    &format!("{BASE_SELECT} WHERE id = ?1"),
                    params![id],
                    row_to_item,
                )
                .optional()?)
        })
    }

    fn list(&self) -> Result<Vec<Self::Entity>> {
        self.conn.with_connection(|conn| {
            let mut stmt =
                conn.prepare(&format!("{BASE_SELECT} ORDER BY updated_at DESC, id DESC"))?;
            let rows = stmt.query_map([], row_to_item)?;
            let mut items = Vec::new();
            for row in rows {
                items.push(row?);
            }
            Ok(items)
        })
    }

    fn count(&self) -> Result<i64> {
        self.conn.with_connection(|conn| {
            Ok(
                conn.query_row("SELECT COUNT(*) FROM terminal_command_history", [], |row| {
                    row.get(0)
                })?,
            )
        })
    }

    fn exists(&self, id: i64) -> Result<bool> {
        self.conn.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM terminal_command_history WHERE id = ?1)",
                params![id],
                |row| row.get(0),
            )?;
            Ok(exists == 1)
        })
    }
}
