//! Support code for DuckDB `data/import_*` handlers.

// `ProtocolError` 是 wire 契约类型,保持与 handler 签名一致。
#![allow(clippy::result_large_err)]

use duckdb::params_from_iter;
use extension_protocol::conn::ConnId;
use extension_protocol::data::{DataFormat, FailedRow, ImportBeginParams, ImportChunkResult};
use extension_protocol::error::{ProtocolError, error_codes};
use extension_protocol::row::Row;

use crate::server::{invalid_params, missing_param, protocol_error_from_anyhow};
use crate::state::{ConnectionState, ImportState};
use crate::value::cell_value_to_duckdb_value;

pub(crate) fn validate_begin_params(
    state: &ConnectionState,
    params: &ImportBeginParams,
) -> Result<(), ProtocolError> {
    if params.table.trim().is_empty() {
        return Err(missing_param("table"));
    }
    validate_database(params.database.as_deref())?;
    validate_import_format(params.format)?;
    validate_import_options(params)?;
    if state.has_active_tx(params.conn_id) {
        return Err(ProtocolError::new(
            error_codes::TX_NESTED_NOT_SUPPORTED,
            "DuckDB data/import cannot run inside an active wire transaction",
        ));
    }
    if params.columns.iter().any(|column| column.trim().is_empty()) {
        return Err(invalid_params("data/import columns must not be empty"));
    }
    Ok(())
}

pub(crate) fn start_import_tx(
    state: &ConnectionState,
    params: &ImportBeginParams,
) -> Result<(String, Vec<String>), ProtocolError> {
    let session = state
        .get_conn(params.conn_id)
        .ok_or_else(|| unknown_conn(params.conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
    let table_columns = load_table_columns(conn, params.schema.as_deref(), &params.table)?;
    let columns = selected_import_columns(&table_columns, &params.columns)?;
    exec_tx_control(conn, "BEGIN TRANSACTION")?;
    Ok((
        qualified_table_ref(params.schema.as_deref(), &params.table),
        columns,
    ))
}

pub(crate) fn run_import_chunk(
    state: &ConnectionState,
    import_state: &mut ImportState,
    rows: &[Row],
) -> Result<ImportChunkResult, ProtocolError> {
    if rows.is_empty() {
        return Ok(ImportChunkResult::default());
    }
    let session = state
        .get_conn(import_state.conn_id())
        .ok_or_else(|| unknown_conn(import_state.conn_id()))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
    let sql = insert_sql(import_state);
    let mut stmt = conn
        .prepare(&sql)
        .map_err(crate::handlers::duckdb_sql_error)?;
    let mut result = ImportChunkResult::default();
    for row in rows {
        match insert_row(&mut stmt, import_state, row) {
            Ok(()) => result.inserted = result.inserted.saturating_add(1),
            Err(Some(failed)) => result.failed.push(failed),
            Err(None) => {}
        }
        enforce_failure_threshold(import_state)?;
    }
    Ok(result)
}

pub(crate) fn exec_import_tx(
    state: &ConnectionState,
    conn_id: ConnId,
    command: &str,
) -> Result<(), ProtocolError> {
    let session = state
        .get_conn(conn_id)
        .ok_or_else(|| unknown_conn(conn_id))?;
    let conn = session
        .connection()
        .map_err(|e| protocol_error_from_anyhow(error_codes::NOT_INITIALIZED, e))?;
    exec_tx_control(conn, command)
}

pub(crate) fn unknown_import(id: &str) -> ProtocolError {
    ProtocolError::new(
        error_codes::UNKNOWN_IMPORT_ID,
        format!("unknown import_id `{id}`"),
    )
}

fn validate_database(database: Option<&str>) -> Result<(), ProtocolError> {
    match database {
        None | Some("") | Some("main") => Ok(()),
        Some(database) => Err(invalid_params(format!(
            "DuckDB data/import only supports database `main`, got `{database}`"
        ))),
    }
}

fn validate_import_format(format: DataFormat) -> Result<(), ProtocolError> {
    match format {
        DataFormat::Csv | DataFormat::Json | DataFormat::Ndjson => Ok(()),
        other => Err(invalid_params(format!(
            "DuckDB data/import does not support format `{other:?}` yet"
        ))),
    }
}

fn validate_import_options(params: &ImportBeginParams) -> Result<(), ProtocolError> {
    if params.options.upsert || !params.options.on_conflict_columns.is_empty() {
        return Err(invalid_params(
            "DuckDB data/import upsert is not implemented yet",
        ));
    }
    if params.options.disable_triggers {
        return Err(invalid_params(
            "DuckDB data/import cannot disable triggers because triggers are unsupported",
        ));
    }
    Ok(())
}

fn load_table_columns(
    conn: &duckdb::Connection,
    schema: Option<&str>,
    table: &str,
) -> Result<Vec<String>, ProtocolError> {
    let mut sql = "SELECT column_name FROM duckdb_columns() \
        WHERE table_name = ? AND internal = FALSE"
        .to_string();
    let mut values = vec![table.to_string()];
    if let Some(schema) = schema.filter(|schema| !schema.trim().is_empty()) {
        sql.push_str(" AND schema_name = ?");
        values.push(schema.to_string());
    }
    sql.push_str(" ORDER BY column_index");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(crate::handlers::duckdb_sql_error)?;
    let rows = stmt
        .query_map(params_from_iter(values.iter()), |row| {
            row.get::<_, String>(0)
        })
        .map_err(crate::handlers::duckdb_sql_error)?;
    let mut columns = Vec::new();
    for row in rows {
        columns.push(row.map_err(crate::handlers::duckdb_sql_error)?);
    }
    if columns.is_empty() {
        return Err(ProtocolError::new(
            error_codes::SQL_UNKNOWN_TABLE,
            format!("unknown import table `{table}`"),
        ));
    }
    Ok(columns)
}

fn selected_import_columns(
    table_columns: &[String],
    requested: &[String],
) -> Result<Vec<String>, ProtocolError> {
    let columns = if requested.is_empty() {
        table_columns.to_vec()
    } else {
        requested.to_vec()
    };
    for column in &columns {
        if !table_columns
            .iter()
            .any(|table_column| table_column == column)
        {
            return Err(ProtocolError::new(
                error_codes::SQL_UNKNOWN_COLUMN,
                format!("unknown import column `{column}`"),
            ));
        }
    }
    Ok(columns)
}

fn insert_sql(import_state: &ImportState) -> String {
    let columns = import_state
        .columns()
        .iter()
        .map(|column| quote_sql_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (0..import_state.columns().len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO {} ({columns}) VALUES ({placeholders})",
        import_state.table_ref()
    )
}

fn insert_row(
    stmt: &mut duckdb::Statement<'_>,
    import_state: &mut ImportState,
    row: &Row,
) -> Result<(), Option<FailedRow>> {
    if row.len() != import_state.columns().len() {
        return Err(import_state.record_failed(
            format!(
                "expected {} values, got {}",
                import_state.columns().len(),
                row.len()
            ),
            error_codes::DATA_TYPE_MISMATCH,
        ));
    }
    let values = row
        .iter()
        .map(cell_value_to_duckdb_value)
        .collect::<anyhow::Result<Vec<_>>>();
    let values = match values {
        Ok(values) => values,
        Err(error) => {
            let message = format!("invalid import row value: {error}");
            return Err(import_state.record_failed(message, error_codes::DATA_TYPE_MISMATCH));
        }
    };
    match stmt.execute(params_from_iter(values.iter())) {
        Ok(_) => {
            import_state.record_inserted();
            Ok(())
        }
        Err(error) => {
            let error = crate::handlers::duckdb_sql_error(error);
            Err(import_state.record_failed(error.message, error.code))
        }
    }
}

fn enforce_failure_threshold(import_state: &ImportState) -> Result<(), ProtocolError> {
    if let Some(limit) = import_state.options().abort_on_failures
        && import_state.failed_count() > limit as u64
    {
        return Err(ProtocolError::new(
            error_codes::DATA_TYPE_MISMATCH,
            "data/import failure threshold exceeded; call data/import_abort to roll back",
        ));
    }
    Ok(())
}

fn exec_tx_control(conn: &duckdb::Connection, command: &str) -> Result<(), ProtocolError> {
    conn.execute(command, []).map(|_| ()).map_err(|error| {
        protocol_error_from_anyhow(
            error_codes::TX_ROLLBACK_REQUIRED,
            anyhow::Error::from(error),
        )
    })
}

fn qualified_table_ref(schema: Option<&str>, table: &str) -> String {
    match schema.filter(|schema| !schema.trim().is_empty()) {
        Some(schema) => format!(
            "{}.{}",
            quote_sql_identifier(schema),
            quote_sql_identifier(table)
        ),
        None => quote_sql_identifier(table),
    }
}

fn quote_sql_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn unknown_conn(id: ConnId) -> ProtocolError {
    ProtocolError::new(
        error_codes::UNKNOWN_CONN_ID,
        format!("unknown conn_id {id}"),
    )
}
