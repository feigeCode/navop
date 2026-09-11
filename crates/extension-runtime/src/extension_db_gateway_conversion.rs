//! Query/exec 结果到扩展协议 [`RowBatch`] 的投影。
//!
//! 优先消费 `QueryResult::typed_batch()` 的类型权威 [`ResultBatch`],把
//! `db_value::DbValue`/`CellState` 尽量保真地映射到扩展协议的 `DbValue`。
//! 无 typed batch 时保留 legacy 字符串投影作为回退。

use db::{ExecResult, QueryResult, SqlResult};
use db_value::{CellState, Nullability, ResultBatch};
use extension_component::protocol::{Column, DbError, DbValue, RowBatch};

pub(super) fn sql_results_to_row_batch(results: Vec<SqlResult>) -> Result<RowBatch, DbError> {
    if let Some(query) = results.iter().find_map(|result| match result {
        SqlResult::Query(query) => Some(query),
        _ => None,
    }) {
        return query_to_row_batch(query);
    }
    Ok(exec_results_to_row_batch(results))
}

fn query_to_row_batch(query: &QueryResult) -> Result<RowBatch, DbError> {
    if let Some(batch) = query.typed_batch() {
        return typed_batch_to_row_batch(batch);
    }
    Ok(legacy_query_to_row_batch(query))
}

/// 类型化 batch 的保真映射:`Binary`→Bytes、`Decimal`→精确文本、
/// `Integer`/`Float`→对应数值、`Json`→其文本形式。
/// `Undecoded`/`DecodeError`/`Unsupported` 无法保真表达,显式报错而非伪装成 `Text`。
fn typed_batch_to_row_batch(batch: &ResultBatch) -> Result<RowBatch, DbError> {
    let columns = batch
        .columns
        .iter()
        .map(|descriptor| Column {
            name: descriptor.label.clone(),
            type_name: descriptor.native_type.clone(),
            nullable: descriptor.nullable != Nullability::No,
        })
        .collect::<Vec<_>>();

    let mut rows = Vec::with_capacity(batch.rows.len());
    for row in &batch.rows {
        let mut cells = Vec::with_capacity(row.cells.len());
        for cell in &row.cells {
            cells.push(match cell {
                CellState::Decoded(value) => map_typed_value(value)?,
                CellState::Undecoded { native_type, .. } => {
                    return Err(unsupported_cell(native_type, "undecoded"));
                }
                CellState::DecodeError { native_type, .. } => {
                    return Err(unsupported_cell(native_type, "decode error"));
                }
            });
        }
        rows.push(cells);
    }

    Ok(RowBatch {
        columns,
        rows,
        next_cursor: None,
    })
}

fn unsupported_cell(native_type: &str, reason: &str) -> DbError {
    DbError::query_failed(format!(
        "cell of type `{native_type}` cannot be represented in the extension protocol ({reason})"
    ))
}

fn map_typed_value(value: &db_value::DbValue) -> Result<DbValue, DbError> {
    use db_value::DbValue as Typed;
    Ok(match value {
        Typed::Null => DbValue::Null,
        Typed::Bool(value) => DbValue::Bool(*value),
        Typed::Integer(text) => parse_i64(text),
        Typed::Unsigned(text) => parse_u64_as_i64(text),
        Typed::Float { value, .. } => match value.parse::<f64>() {
            Ok(parsed) => DbValue::Float(parsed),
            Err(_) => DbValue::Text(value.clone()),
        },
        // 精确小数点/时间/标识以文本保真,避免精度或格式损失。
        Typed::Decimal(text)
        | Typed::BitString(text)
        | Typed::Date(text)
        | Typed::Time(text)
        | Typed::DateTime(text)
        | Typed::Uuid(text)
        | Typed::Duration(text)
        | Typed::Text(text)
        | Typed::LegacyText(text) => DbValue::Text(text.clone()),
        Typed::Binary(bytes) => DbValue::Bytes(bytes.clone()),
        Typed::Json(value) => DbValue::Text(value.to_string()),
        // 已知类型但驱动未能归类,无法在扩展协议中保真表达。
        Typed::Unsupported { native_type, .. } => {
            return Err(unsupported_cell(native_type, "unsupported"));
        }
    })
}

fn parse_i64(text: &str) -> DbValue {
    match text.parse::<i64>() {
        Ok(value) => DbValue::Integer(value),
        // 超出 i64 范围时以原始精确数字文本回退,不丢失信息。
        Err(_) => DbValue::Text(text.to_string()),
    }
}

fn parse_u64_as_i64(text: &str) -> DbValue {
    match text.parse::<u64>() {
        Ok(value) if value <= i64::MAX as u64 => DbValue::Integer(value as i64),
        _ => DbValue::Text(text.to_string()),
    }
}

fn legacy_query_to_row_batch(query: &QueryResult) -> RowBatch {
    let columns = query
        .columns
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let meta = query.column_meta.get(index);
            Column {
                name: name.clone(),
                type_name: meta.map(|meta| meta.db_type.clone()).unwrap_or_default(),
                nullable: meta.map(|meta| meta.nullable).unwrap_or(true),
            }
        })
        .collect();
    let rows = query
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| match cell {
                    Some(value) => DbValue::Text(value.clone()),
                    None => DbValue::Null,
                })
                .collect()
        })
        .collect();
    RowBatch {
        columns,
        rows,
        next_cursor: None,
    }
}

fn exec_results_to_row_batch(results: Vec<SqlResult>) -> RowBatch {
    let rows = results
        .into_iter()
        .filter_map(|result| match result {
            SqlResult::Exec(exec) => Some(exec_to_row(exec)),
            SqlResult::Error(error) => Some(vec![
                DbValue::Text(error.sql),
                DbValue::Text("error".to_string()),
                DbValue::Text(error.message),
            ]),
            SqlResult::Query(_) => None,
        })
        .collect();
    RowBatch {
        columns: vec![
            Column {
                name: "sql".to_string(),
                type_name: "text".to_string(),
                nullable: false,
            },
            Column {
                name: "status".to_string(),
                type_name: "text".to_string(),
                nullable: false,
            },
            Column {
                name: "message".to_string(),
                type_name: "text".to_string(),
                nullable: true,
            },
        ],
        rows,
        next_cursor: None,
    }
}

fn exec_to_row(exec: ExecResult) -> Vec<DbValue> {
    vec![
        DbValue::Text(exec.sql),
        DbValue::Text("ok".to_string()),
        DbValue::Text(
            exec.message
                .unwrap_or_else(|| format!("{} row(s) affected", exec.rows_affected)),
        ),
    ]
}
