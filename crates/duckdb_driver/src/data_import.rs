//! DuckDB `data/import_*` wire handlers.

// `ProtocolError` 是 wire 契约类型,保持与 handler 签名一致。
#![allow(clippy::result_large_err)]

use extension_protocol::data::{
    ImportAbortParams, ImportBeginParams, ImportBeginResult, ImportChunkParams, ImportCommitParams,
    ImportCommitResult,
};
use extension_protocol::error::ProtocolError;
use serde_json::Value;

use crate::data_import_support::{
    exec_import_tx, run_import_chunk, start_import_tx, unknown_import, validate_begin_params,
};
use crate::server::params_deserialize_error;
use crate::state::{ConnectionState, ImportState};

pub fn handle_data_import_begin(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ImportBeginParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    validate_begin_params(state, &p)?;
    let (table_ref, columns) = start_import_tx(state, &p)?;
    let import_id = state.open_import(ImportState::new(p.conn_id, table_ref, columns, p.options));
    serde_json::to_value(ImportBeginResult { import_id }).map_err(params_deserialize_error)
}

pub fn handle_data_import_chunk(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ImportChunkParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let mut import_state = state
        .remove_import(&p.import_id)
        .ok_or_else(|| unknown_import(&p.import_id))?;
    let result = run_import_chunk(state, &mut import_state, &p.rows);
    state.insert_import(p.import_id, import_state);
    serde_json::to_value(result?).map_err(params_deserialize_error)
}

pub fn handle_data_import_commit(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ImportCommitParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let import_state = state
        .remove_import(&p.import_id)
        .ok_or_else(|| unknown_import(&p.import_id))?;
    exec_import_tx(state, import_state.conn_id(), "COMMIT")?;
    serde_json::to_value(ImportCommitResult {
        inserted: import_state.inserted(),
        updated: 0,
        deleted: 0,
        failed: import_state.failed().to_vec(),
        elapsed_ms: Some(import_state.started().elapsed().as_millis() as u64),
    })
    .map_err(params_deserialize_error)
}

pub fn handle_data_import_abort(
    state: &mut ConnectionState,
    params: &Value,
) -> Result<Value, ProtocolError> {
    let p: ImportAbortParams =
        serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
    let import_state = state
        .remove_import(&p.import_id)
        .ok_or_else(|| unknown_import(&p.import_id))?;
    exec_import_tx(state, import_state.conn_id(), "ROLLBACK")?;
    Ok(Value::Null)
}
