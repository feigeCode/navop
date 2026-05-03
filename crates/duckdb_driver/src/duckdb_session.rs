use crate::protocol::DbConnectionConfig;
use crate::result::{
    ExecResult, QueryColumnMeta, QueryResult, SqlErrorInfo, SqlResult, format_message,
};
use anyhow::{Context, Result, anyhow};
use duckdb::{Connection, types::ValueRef};
use std::time::Instant;

pub struct DuckDbSession {
    config: Option<DbConnectionConfig>,
    connection: Option<Connection>,
}

impl Default for DuckDbSession {
    fn default() -> Self {
        Self::new()
    }
}

impl DuckDbSession {
    pub fn new() -> Self {
        Self {
            config: None,
            connection: None,
        }
    }

    pub fn connect(&mut self, config: DbConnectionConfig) -> Result<()> {
        let path = database_path(&config)?;
        self.connection = Some(Connection::open(path).context("failed to open DuckDB database")?);
        self.config = Some(config);
        Ok(())
    }

    pub fn disconnect(&mut self) {
        self.connection = None;
        self.config = None;
    }

    pub fn ping(&self) -> Result<()> {
        self.connection()?;
        Ok(())
    }

    pub fn current_database(&self) -> Option<String> {
        Some("main".to_string())
    }

    pub fn query(&self, sql: &str) -> SqlResult {
        let start = Instant::now();
        let connection = match self.connection() {
            Ok(connection) => connection,
            Err(error) => return error_result(sql, error),
        };
        if !is_query_sql(sql) {
            return match connection.execute(sql, []) {
                Ok(rows_affected) => SqlResult::Exec(ExecResult {
                    sql: sql.to_string(),
                    rows_affected: rows_affected as u64,
                    elapsed_ms: start.elapsed().as_millis(),
                    message: Some(format_message(sql, rows_affected as u64)),
                }),
                Err(error) => error_result(sql, error),
            };
        }

        query_rows(connection, sql, start).unwrap_or_else(|error| error_result(sql, error))
    }

    pub fn connection(&self) -> Result<&Connection> {
        self.connection
            .as_ref()
            .ok_or_else(|| anyhow!("DuckDB connection is not initialized"))
    }
}

fn database_path(config: &DbConnectionConfig) -> Result<String> {
    if !config.host.trim().is_empty() {
        return Ok(config.host.clone());
    }
    if let Some(path) = config
        .database
        .as_ref()
        .filter(|database| !database.trim().is_empty())
    {
        return Ok(path.clone());
    }
    config
        .extra_params
        .get("path")
        .filter(|path| !path.trim().is_empty())
        .cloned()
        .ok_or_else(|| anyhow!("database path is required for DuckDB"))
}
fn query_rows(connection: &Connection, sql: &str, start: Instant) -> Result<SqlResult> {
    let mut statement = connection.prepare(sql)?;
    let rows_result = statement.query([]).and_then(|mut rows| {
        let statement = rows
            .as_ref()
            .expect("DuckDB rows should retain statement metadata");
        let column_count = statement.column_count();
        let columns = statement.column_names();
        let column_types = (0..column_count)
            .map(|idx| format!("{:?}", statement.column_type(idx)))
            .collect::<Vec<_>>();
        let mut data_rows = Vec::new();

        while let Some(row) = rows.next()? {
            let data_row = (0..column_count)
                .map(|idx| {
                    row.get_ref(idx)
                        .ok()
                        .and_then(|value| extract_value(value, Some(&column_types[idx])))
                })
                .collect();
            data_rows.push(data_row);
        }

        Ok((columns, column_types, data_rows))
    })?;

    let (columns, column_types, rows) = rows_result;
    let column_meta = columns
        .iter()
        .zip(column_types.iter())
        .map(|(name, db_type)| QueryColumnMeta::new(name.clone(), db_type.clone()))
        .collect();

    Ok(SqlResult::Query(QueryResult {
        sql: sql.to_string(),
        columns,
        column_meta,
        rows,
        elapsed_ms: start.elapsed().as_millis(),
    }))
}

fn extract_value(value: ValueRef<'_>, decl_type: Option<&str>) -> Option<String> {
    match value {
        ValueRef::Null => None,
        ValueRef::Boolean(value) => Some(value.to_string()),
        ValueRef::TinyInt(value) => Some(value.to_string()),
        ValueRef::SmallInt(value) => Some(value.to_string()),
        ValueRef::Int(value) => extract_int(value, decl_type),
        ValueRef::BigInt(value) => Some(value.to_string()),
        ValueRef::HugeInt(value) => Some(value.to_string()),
        ValueRef::UTinyInt(value) => Some(value.to_string()),
        ValueRef::USmallInt(value) => Some(value.to_string()),
        ValueRef::UInt(value) => Some(value.to_string()),
        ValueRef::UBigInt(value) => Some(value.to_string()),
        ValueRef::Float(value) => Some(value.to_string()),
        ValueRef::Double(value) => Some(value.to_string()),
        ValueRef::Decimal(value) => Some(value.to_string()),
        ValueRef::Text(value) => String::from_utf8(value.to_vec()).ok(),
        ValueRef::Blob(value) => String::from_utf8(value.to_vec())
            .ok()
            .or_else(|| Some(format!("0x{}", hex::encode(value)))),
        other => Some(format!("{other:?}")),
    }
}

fn extract_int(value: i32, decl_type: Option<&str>) -> Option<String> {
    let is_datetime = decl_type.is_some_and(|decl_type| {
        let upper = decl_type.to_uppercase();
        upper.contains("DATE") || upper.contains("TIME") || upper.contains("TIMESTAMP")
    });
    if is_datetime {
        return chrono::DateTime::from_timestamp(value as i64, 0)
            .map(|date| date.format("%Y-%m-%d %H:%M:%S").to_string());
    }
    Some(value.to_string())
}

fn is_query_sql(sql: &str) -> bool {
    let normalized = sql.trim_start().to_ascii_uppercase();
    normalized.starts_with("SELECT")
        || normalized.starts_with("WITH")
        || normalized.starts_with("PRAGMA")
        || normalized.starts_with("SHOW")
        || normalized.starts_with("DESCRIBE")
        || normalized.starts_with("EXPLAIN")
}

fn error_result(sql: &str, error: impl std::fmt::Display) -> SqlResult {
    SqlResult::Error(SqlErrorInfo {
        sql: sql.to_string(),
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_database_path() {
        let config = DbConnectionConfig {
            host: String::new(),
            database: None,
            extra_params: Default::default(),
        };

        assert!(database_path(&config).is_err());
    }
}
