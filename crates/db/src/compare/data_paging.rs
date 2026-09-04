use std::collections::HashMap;

use crate::{
    BinaryCell, ColumnInfo, FieldType, QueryCellRef, QueryColumnMeta, QueryResult,
    TableDataRequest, TableDataResponse, plugin::DatabasePlugin,
    query_result_normalization::normalize_query_result_binary_semantics,
};
use one_core::storage::DatabaseType;

use super::{DataCompareLimits, RowData, binary_cell_value, sync_plan::format_value_for_database};

pub const DEFAULT_DATA_COMPARE_PAGE_SIZE: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataComparePagingDecision {
    Complete,
    Truncated,
    Continue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataCompareColumnMapping {
    pub source: String,
    pub target: String,
}

pub fn build_table_data_request(
    database: String,
    schema: Option<String>,
    table: String,
    order_by_clause: Option<&str>,
    page: usize,
    offset: usize,
    page_size: usize,
) -> TableDataRequest {
    let mut request = TableDataRequest::new(database, table)
        .with_page(page, page_size)
        .with_offset(offset);
    if let Some(schema) = schema {
        request = request.with_schema(schema);
    }
    if let Some(order_by_clause) = order_by_clause {
        request = request.with_order_by_clause(order_by_clause);
    }
    request
}

/// Applies a keyset predicate to a table-data request.
///
/// Keyset requests always start at offset zero because the predicate itself
/// identifies the next page. Keeping this separate from the ordinary request
/// builder preserves the existing OFFSET fallback for unsupported databases
/// and nullable or otherwise unsafe key columns.
pub(crate) fn apply_keyset_where_clause(
    request: TableDataRequest,
    where_clause: Option<&str>,
) -> TableDataRequest {
    match where_clause {
        Some(where_clause) => request.with_offset(0).with_where_clause(where_clause),
        None => request,
    }
}

/// Builds a portable lexicographic keyset predicate from the last row.
///
/// The first safe implementation is deliberately conservative:
/// - only the five built-in compare profiles use keyset pagination;
/// - every key column must be a non-nullable primary-key column with a scalar,
///   orderable type;
/// - external/unknown drivers and unsafe keys retain the existing OFFSET path.
///
/// Once keyset mode is eligible, malformed query results are errors rather
/// than silent OFFSET fallbacks. Falling back after some keyset pages could
/// otherwise duplicate or skip rows.
pub(crate) fn build_keyset_where_clause(
    plugin: &dyn DatabasePlugin,
    key_columns: &[String],
    business_columns: &[ColumnInfo],
    result: &QueryResult,
    case_sensitive_identifiers: bool,
) -> anyhow::Result<Option<String>> {
    if key_columns.is_empty() || !database_supports_compare_keyset(&plugin.name()) {
        return Ok(None);
    }

    let mut resolved_columns = Vec::with_capacity(key_columns.len());
    for key_column in key_columns {
        let key = identifier_key(key_column, case_sensitive_identifiers);
        let matching = business_columns
            .iter()
            .filter(|column| identifier_key(&column.name, case_sensitive_identifiers) == key)
            .collect::<Vec<_>>();
        let column = match matching.as_slice() {
            [column] => *column,
            [] => anyhow::bail!(
                "Keyset pagination column `{key_column}` is missing from table metadata"
            ),
            _ => anyhow::bail!(
                "Keyset pagination column `{key_column}` is ambiguous in table metadata"
            ),
        };
        if !column.is_primary_key
            || column.is_nullable
            || !field_type_supports_compare_keyset(&column.data_type)
        {
            return Ok(None);
        }
        resolved_columns.push(column);
    }

    let Some(last_row) = result.rows.last() else {
        return Ok(None);
    };
    let mut cursor_values = Vec::with_capacity(key_columns.len());
    for (key_column, column) in key_columns.iter().zip(&resolved_columns) {
        let key = identifier_key(key_column, case_sensitive_identifiers);
        let matching_indices = result
            .columns
            .iter()
            .enumerate()
            .filter_map(|(index, result_column)| {
                (identifier_key(result_column, case_sensitive_identifiers) == key).then_some(index)
            })
            .collect::<Vec<_>>();
        let index = match matching_indices.as_slice() {
            [index] => *index,
            [] => anyhow::bail!(
                "Keyset pagination column `{key_column}` is missing from query results"
            ),
            _ => anyhow::bail!(
                "Keyset pagination column `{key_column}` is ambiguous in query results"
            ),
        };
        let raw_value = last_row
            .get(index)
            .ok_or_else(|| {
                anyhow::anyhow!("Keyset pagination row is missing value for column `{key_column}`")
            })?
            .as_deref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Keyset pagination encountered NULL in non-nullable key column `{key_column}`"
                )
            })?;
        cursor_values.push(
            compare_keyset_cell_value(raw_value, &column.data_type).map_err(|error| {
                anyhow::anyhow!("Invalid keyset value for column `{key_column}`: {error}")
            })?,
        );
    }

    let database_type = plugin.name();
    let mut branches = Vec::with_capacity(resolved_columns.len());
    for branch_index in 0..resolved_columns.len() {
        let mut conditions = Vec::with_capacity(branch_index + 1);
        for equality_index in 0..branch_index {
            let column = resolved_columns[equality_index];
            conditions.push(format!(
                "{} = {}",
                plugin.quote_identifier(&column.name),
                format_value_for_database(
                    &cursor_values[equality_index],
                    Some(&column.data_type),
                    Some(database_type.clone()),
                )
            ));
        }
        let column = resolved_columns[branch_index];
        conditions.push(format!(
            "{} > {}",
            plugin.quote_identifier(&column.name),
            format_value_for_database(
                &cursor_values[branch_index],
                Some(&column.data_type),
                Some(database_type.clone()),
            )
        ));
        branches.push(format!("({})", conditions.join(" AND ")));
    }
    Ok(Some(branches.join(" OR ")))
}

fn database_supports_compare_keyset(database_type: &DatabaseType) -> bool {
    matches!(
        database_type,
        DatabaseType::MySQL
            | DatabaseType::PostgreSQL
            | DatabaseType::SQLite
            | DatabaseType::MSSQL
            | DatabaseType::ClickHouse
            // TDengine 支持 LIMIT n OFFSET m,可参与游标分页比较。
            | DatabaseType::TDengine
    )
}

fn field_type_supports_compare_keyset(data_type: &str) -> bool {
    matches!(
        FieldType::from_db_type(data_type),
        FieldType::Integer
            | FieldType::Decimal
            | FieldType::Text
            | FieldType::Boolean
            | FieldType::Date
            | FieldType::Time
            | FieldType::DateTime
    )
}

fn compare_keyset_cell_value(value: &str, data_type: &str) -> anyhow::Result<serde_json::Value> {
    match FieldType::from_db_type(data_type) {
        FieldType::Integer => {
            let number = canonical_integer_literal(value)
                .ok_or_else(|| anyhow::anyhow!("`{value}` is not a plain integer literal"))?;
            Ok(serde_json::Value::Number(
                serde_json::Number::from_string_unchecked(number),
            ))
        }
        FieldType::Decimal => {
            let number = canonical_decimal_literal(value)
                .ok_or_else(|| anyhow::anyhow!("`{value}` is not a plain numeric literal"))?;
            Ok(serde_json::Value::Number(
                serde_json::Number::from_string_unchecked(number),
            ))
        }
        FieldType::Boolean => match parse_boolean_cell(value) {
            serde_json::Value::Bool(value) => Ok(serde_json::Value::Bool(value)),
            _ => anyhow::bail!("`{value}` is not a boolean literal"),
        },
        FieldType::Text => Ok(serde_json::Value::String(value.to_string())),
        field_type @ (FieldType::Date | FieldType::Time | FieldType::DateTime) => Ok(
            serde_json::Value::String(normalize_temporal_cell_text(value, field_type)),
        ),
        _ => anyhow::bail!("type `{data_type}` is not supported for keyset pagination"),
    }
}

pub fn identifier_key(value: &str, case_sensitive_identifiers: bool) -> String {
    if case_sensitive_identifiers {
        value.trim().to_string()
    } else {
        value.trim().to_lowercase()
    }
}

pub fn common_column_mappings(
    source_columns: &[String],
    target_columns: &[String],
    case_sensitive_identifiers: bool,
) -> Vec<DataCompareColumnMapping> {
    let target = target_columns
        .iter()
        .map(|column| {
            (
                identifier_key(column, case_sensitive_identifiers),
                column.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    source_columns
        .iter()
        .filter_map(|source| {
            target
                .get(&identifier_key(source, case_sensitive_identifiers))
                .map(|target| DataCompareColumnMapping {
                    source: source.clone(),
                    target: target.clone(),
                })
        })
        .collect()
}

pub fn rows_from_query_result(result: &QueryResult) -> anyhow::Result<Vec<RowData>> {
    let mappings = result
        .columns
        .iter()
        .map(|column| DataCompareColumnMapping {
            source: column.clone(),
            target: column.clone(),
        })
        .collect::<Vec<_>>();
    rows_from_query_result_with_mappings(result, &mappings, false)
}

pub fn rows_from_query_result_with_mappings(
    result: &QueryResult,
    mappings: &[DataCompareColumnMapping],
    case_sensitive_identifiers: bool,
) -> anyhow::Result<Vec<RowData>> {
    let view = result
        .typed_view()
        .map_err(|error| anyhow::anyhow!("Invalid query result for data comparison: {error}"))?;
    let index_by_column = result
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| (identifier_key(column, case_sensitive_identifiers), index))
        .collect::<HashMap<_, _>>();
    (0..result.rows.len())
        .map(|row_index| {
            mappings
                .iter()
                .map(|mapping| {
                    let index = *index_by_column
                        .get(&identifier_key(&mapping.target, case_sensitive_identifiers))
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "compare column mapping target {:?} is missing from query result",
                                mapping.target
                            )
                        })?;
                    let cell = match view.cell(row_index, index) {
                        Some(QueryCellRef::Null) => {
                            value_to_cell(None, result.column_meta.get(index), None)
                        }
                        Some(QueryCellRef::Text(value)) => {
                            value_to_cell(Some(value), result.column_meta.get(index), None)
                        }
                        Some(QueryCellRef::Binary(bytes)) => {
                            value_to_cell(None, result.column_meta.get(index), Some(bytes))
                        }
                        None => unreachable!("typed view validated row and column bounds"),
                    };
                    Ok((mapping.source.clone(), cell))
                })
                .collect()
        })
        .collect()
}

/// Removes the synthetic row-id column injected by table-data query plugins.
///
/// The injected column is always the leading synthetic rowid projection. It is
/// important not to remove every column with that name: a real table is
/// allowed to have a business column with the same name, and `t.*` will then
/// return two columns with the same display name.
pub fn strip_internal_compare_columns_if(
    mut response: TableDataResponse,
    has_internal_rowid: bool,
    internal_rowid_alias: &str,
    business_columns: &[ColumnInfo],
) -> TableDataResponse {
    if !has_internal_rowid
        || response
            .query_result
            .columns
            .first()
            .is_none_or(|column| !is_internal_compare_column(column, internal_rowid_alias))
    {
        return response;
    }

    if response.query_result.typed_view().is_err() {
        // Preserve malformed payloads untouched so the typed conversion stage can report them.
        return response;
    }

    let columns_len = response.query_result.columns.len();
    let keep_indices = (1..columns_len).collect::<Vec<_>>();
    if !business_columns.is_empty()
        && (keep_indices.len() != business_columns.len()
            || keep_indices
                .iter()
                .zip(business_columns)
                .any(|(index, column)| {
                    !response.query_result.columns[*index]
                        .trim()
                        .eq_ignore_ascii_case(column.name.trim())
                }))
    {
        return response;
    }

    let mut new_index_by_old_index = HashMap::new();
    let columns = keep_indices
        .iter()
        .enumerate()
        .map(|(new_index, old_index)| {
            new_index_by_old_index.insert(*old_index, new_index);
            response.query_result.columns[*old_index].clone()
        })
        .collect::<Vec<_>>();
    let column_meta = keep_indices
        .iter()
        .filter_map(|index| response.query_result.column_meta.get(*index).cloned())
        .collect::<Vec<_>>();
    let rows = response
        .query_result
        .rows
        .into_iter()
        .map(|row| {
            keep_indices
                .iter()
                .map(|index| row[*index].clone())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let binary_cells = response
        .query_result
        .binary_cells
        .into_iter()
        .filter_map(|cell| {
            new_index_by_old_index
                .get(&cell.column_index)
                .copied()
                .map(|column_index| BinaryCell {
                    row_index: cell.row_index,
                    column_index,
                    bytes: cell.bytes,
                })
        })
        .collect::<Vec<_>>();

    response.query_result.columns = columns;
    response.query_result.column_meta = column_meta;
    response.query_result.rows = rows;
    response.query_result.binary_cells = binary_cells;
    response
}

pub(crate) fn normalize_compare_table_data_response(
    mut response: TableDataResponse,
    internal_rowid_alias: Option<&str>,
    database_type: &DatabaseType,
    business_columns: &[ColumnInfo],
) -> anyhow::Result<TableDataResponse> {
    response = if let Some(internal_rowid_alias) = internal_rowid_alias {
        strip_internal_compare_columns_if(response, true, internal_rowid_alias, business_columns)
    } else {
        response
    };
    normalize_query_result_binary_semantics(
        &mut response.query_result,
        database_type,
        business_columns,
    )
    .map_err(|error| anyhow::anyhow!("Invalid query result for data comparison: {error}"))?;
    Ok(response)
}

/// Backwards-compatible wrapper for callers that already know the query
/// contains the synthetic row-id column.
pub fn strip_internal_compare_columns(response: TableDataResponse) -> TableDataResponse {
    strip_internal_compare_columns_if(response, true, "__rowid__", &[])
}

pub fn append_table_data_page(
    accumulated: &mut Option<TableDataResponse>,
    mut page: TableDataResponse,
) -> anyhow::Result<()> {
    page.query_result
        .typed_view()
        .map_err(|error| anyhow::anyhow!("Invalid table data page: {error}"))?;

    if let Some(existing) = accumulated.as_mut() {
        existing
            .query_result
            .typed_view()
            .map_err(|error| anyhow::anyhow!("Invalid accumulated table data: {error}"))?;
        if page.total_count != existing.total_count {
            anyhow::bail!(
                "table row count changed while paging: expected {}, got {}",
                existing.total_count,
                page.total_count
            );
        }
        if page.query_result.columns != existing.query_result.columns
            || !query_column_meta_eq(
                &page.query_result.column_meta,
                &existing.query_result.column_meta,
            )
        {
            anyhow::bail!("table columns changed while paging");
        }

        let row_offset = existing.query_result.rows.len();
        let combined_row_count = row_offset
            .checked_add(page.query_result.rows.len())
            .ok_or_else(|| anyhow::anyhow!("table row count overflow while paging"))?;
        if combined_row_count > existing.total_count {
            anyhow::bail!(
                "table returned more rows while paging than the initial COUNT: expected {}, got at least {}",
                existing.total_count,
                combined_row_count
            );
        }
        let adjusted_binary_cells = page
            .query_result
            .binary_cells
            .into_iter()
            .map(|mut cell| {
                cell.row_index = cell.row_index.checked_add(row_offset).ok_or_else(|| {
                    anyhow::anyhow!(
                        "binary cell row index overflow while appending table data page"
                    )
                })?;
                Ok(cell)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        existing
            .query_result
            .rows
            .append(&mut page.query_result.rows);
        existing
            .query_result
            .binary_cells
            .extend(adjusted_binary_cells);
        existing.duration = existing.duration.saturating_add(page.duration);
        existing
            .query_result
            .typed_view()
            .map_err(|error| anyhow::anyhow!("Invalid combined table data: {error}"))?;
        return Ok(());
    }

    if page.query_result.rows.len() > page.total_count {
        anyhow::bail!(
            "table returned more rows than COUNT: expected {}, got {}",
            page.total_count,
            page.query_result.rows.len()
        );
    }
    *accumulated = Some(page);
    Ok(())
}

/// Returns whether paging has fetched the exact row count reported by COUNT.
///
/// A short or empty page before reaching `total_count` is not a successful
/// termination condition: it means the table changed while OFFSET paging was
/// in progress, or the driver returned an incomplete page. Failing loudly
/// prevents an incomplete snapshot from being presented as a trustworthy diff.
pub fn table_data_page_is_complete(
    accumulated_rows: usize,
    total_count: usize,
    page_row_count: usize,
    page_size: usize,
) -> anyhow::Result<bool> {
    Ok(matches!(
        data_compare_paging_decision(
            accumulated_rows,
            total_count,
            page_row_count,
            page_size,
            0,
            DataCompareLimits::unlimited(),
        )?,
        DataComparePagingDecision::Complete
    ))
}

/// Returns whether an exact COUNT completion landed on a full page.
///
/// Without one final empty-page probe, an initial COUNT of exactly one page
/// can hide rows inserted before or during the fetch: the first page still
/// looks complete even though another row is now reachable at `OFFSET count`.
pub fn table_data_terminal_probe_required(
    accumulated_rows: usize,
    total_count: usize,
    page_row_count: usize,
    requested_page_size: usize,
) -> bool {
    accumulated_rows > 0 && accumulated_rows == total_count && page_row_count == requested_page_size
}

/// Chooses the next request size without allowing a configurable row limit to
/// be overshot by a full default-sized page.
pub fn data_compare_next_page_size(
    default_page_size: usize,
    accumulated_rows: usize,
    limits: DataCompareLimits,
) -> anyhow::Result<usize> {
    if default_page_size == 0 {
        anyhow::bail!("data compare page size must be greater than zero");
    }
    validate_data_compare_limits(limits)?;

    let remaining_rows = limits
        .max_rows_per_table
        .map(|max_rows| max_rows.saturating_sub(accumulated_rows))
        .unwrap_or(default_page_size);
    if remaining_rows == 0 {
        anyhow::bail!("data compare row limit was reached before requesting the next page");
    }
    Ok(default_page_size.min(remaining_rows))
}

/// Determines whether paging is complete, safely truncated, or should
/// continue.
///
/// Exact COUNT completion takes precedence over limits so a table whose row
/// count is exactly equal to a configured threshold is not mislabeled as
/// truncated. Short or empty pages before either condition remain hard errors:
/// they indicate a changing table or an incomplete driver response.
pub fn data_compare_paging_decision(
    accumulated_rows: usize,
    total_count: usize,
    page_row_count: usize,
    requested_page_size: usize,
    fetched_pages: usize,
    limits: DataCompareLimits,
) -> anyhow::Result<DataComparePagingDecision> {
    if requested_page_size == 0 {
        anyhow::bail!("data compare page size must be greater than zero");
    }
    validate_data_compare_limits(limits)?;

    if accumulated_rows > total_count {
        anyhow::bail!(
            "table returned more rows while paging than COUNT: expected {}, got {}",
            total_count,
            accumulated_rows
        );
    }
    if accumulated_rows == total_count {
        return Ok(DataComparePagingDecision::Complete);
    }
    if limits
        .max_rows_per_table
        .is_some_and(|max_rows| accumulated_rows >= max_rows)
        || limits
            .max_pages_per_table
            .is_some_and(|max_pages| fetched_pages >= max_pages)
    {
        return Ok(DataComparePagingDecision::Truncated);
    }
    if page_row_count == 0 || page_row_count < requested_page_size {
        anyhow::bail!(
            "table returned fewer rows while paging than COUNT: expected {}, got {}",
            total_count,
            accumulated_rows
        );
    }
    Ok(DataComparePagingDecision::Continue)
}

fn validate_data_compare_limits(limits: DataCompareLimits) -> anyhow::Result<()> {
    if limits.max_rows_per_table == Some(0) {
        anyhow::bail!("data compare maximum rows per table must be greater than zero");
    }
    if limits.max_pages_per_table == Some(0) {
        anyhow::bail!("data compare maximum pages per table must be greater than zero");
    }
    Ok(())
}

fn is_internal_compare_column(column: &str, internal_rowid_alias: &str) -> bool {
    column
        .trim()
        .eq_ignore_ascii_case(internal_rowid_alias.trim())
}

fn query_column_meta_eq(left: &[QueryColumnMeta], right: &[QueryColumnMeta]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.name == right.name
                && left.db_type == right.db_type
                && left.field_type == right.field_type
                && left.nullable == right.nullable
        })
}

fn value_to_cell(
    value: Option<&str>,
    meta: Option<&QueryColumnMeta>,
    binary: Option<&[u8]>,
) -> serde_json::Value {
    if let Some(bytes) = binary {
        return binary_cell_value(bytes);
    }
    let Some(value) = value else {
        return serde_json::Value::Null;
    };
    match meta.map(|meta| meta.field_type) {
        Some(FieldType::Integer) => parse_integer_cell(value),
        Some(FieldType::Decimal) => parse_decimal_cell(value),
        Some(FieldType::Boolean) => parse_boolean_cell(value),
        Some(FieldType::Json) => parse_json_cell(value),
        Some(field_type @ (FieldType::Date | FieldType::Time | FieldType::DateTime)) => {
            serde_json::Value::String(normalize_temporal_cell_text(value, field_type))
        }
        _ => serde_json::Value::String(value.to_string()),
    }
}

fn parse_integer_cell(value: &str) -> serde_json::Value {
    let Some(number) = canonical_integer_literal(value) else {
        return serde_json::Value::String(value.to_string());
    };
    serde_json::Value::Number(serde_json::Number::from_string_unchecked(number))
}

fn canonical_integer_literal(value: &str) -> Option<String> {
    let value = value.trim();
    let (negative, digits) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let digits = digits.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    if negative && digits != "0" {
        Some(format!("-{digits}"))
    } else {
        Some(digits.to_string())
    }
}

fn parse_decimal_cell(value: &str) -> serde_json::Value {
    let Some(number) = canonical_decimal_literal(value) else {
        return serde_json::Value::String(value.to_string());
    };
    serde_json::Value::Number(serde_json::Number::from_string_unchecked(number))
}

fn canonical_decimal_literal(value: &str) -> Option<String> {
    let value = value.trim();
    let (negative, digits) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let (integer, fraction) = match digits.split_once('.') {
        Some((integer, fraction)) => (integer, fraction),
        None => (digits, ""),
    };
    if integer.is_empty() && fraction.is_empty() {
        return None;
    }
    if !integer.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    if !fraction.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let fraction = fraction.trim_end_matches('0');
    let mut number = if fraction.is_empty() {
        integer.to_string()
    } else {
        format!("{integer}.{fraction}")
    };
    if negative && number != "0" {
        number.insert(0, '-');
    }
    Some(number)
}

fn parse_boolean_cell(value: &str) -> serde_json::Value {
    match value.to_ascii_lowercase().as_str() {
        "true" | "t" | "1" | "yes" => serde_json::Value::Bool(true),
        "false" | "f" | "0" | "no" => serde_json::Value::Bool(false),
        _ => serde_json::Value::String(value.to_string()),
    }
}

fn parse_json_cell(value: &str) -> serde_json::Value {
    serde_json::from_str(value).unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
}

fn normalize_temporal_cell_text(value: &str, field_type: FieldType) -> String {
    let value = value.trim();
    if !matches!(field_type, FieldType::DateTime) {
        return value.to_string();
    }
    let Some(separator) = value.as_bytes().get(10).copied() else {
        return value.to_string();
    };
    if !matches!(separator, b'T' | b't') {
        return value.to_string();
    }
    let mut normalized = value.to_string();
    normalized.replace_range(10..11, " ");
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::QueryColumnMeta;

    fn column(name: &str, data_type: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            is_nullable: true,
            is_primary_key: false,
            default_value: None,
            comment: None,
            charset: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_0900_ai_ci".to_string()),
        }
    }

    fn keyset_column(name: &str, data_type: &str) -> ColumnInfo {
        ColumnInfo {
            is_nullable: false,
            is_primary_key: true,
            ..column(name, data_type)
        }
    }

    fn response(
        columns: Vec<&str>,
        column_meta: Vec<QueryColumnMeta>,
        rows: Vec<Vec<Option<&str>>>,
    ) -> TableDataResponse {
        TableDataResponse {
            total_count: rows.len(),
            page: 1,
            page_size: 10,
            duration: 0,
            query_result: QueryResult {
                sql: String::new(),
                columns: columns.into_iter().map(str::to_string).collect(),
                column_meta,
                rows: rows
                    .into_iter()
                    .map(|row| {
                        row.into_iter()
                            .map(|value| value.map(str::to_string))
                            .collect()
                    })
                    .collect(),
                binary_cells: Vec::new(),
                elapsed_ms: 0,
            },
        }
    }

    #[test]
    fn rows_use_metadata_without_losing_decimal_precision() {
        let result = response(
            vec!["id", "amount", "enabled"],
            vec![
                QueryColumnMeta::new("id", "bigint"),
                QueryColumnMeta::new("amount", "decimal"),
                QueryColumnMeta::new("enabled", "boolean"),
            ],
            vec![vec![
                Some("42"),
                Some("12345678901234567890.1234500"),
                Some("true"),
            ]],
        );

        let rows = rows_from_query_result(&result.query_result).unwrap();
        assert_eq!(rows[0].get("id"), Some(&serde_json::json!(42)));
        assert_eq!(
            rows[0].get("amount").map(ToString::to_string).as_deref(),
            Some("12345678901234567890.12345")
        );
        assert_eq!(rows[0].get("enabled"), Some(&serde_json::json!(true)));
    }

    #[test]
    fn rows_preserve_binary_sidecar_cells_even_when_text_value_is_null() {
        let mut result = response(
            vec!["payload"],
            vec![QueryColumnMeta::new("payload", "blob")],
            vec![vec![None]],
        );
        result.query_result.binary_cells = vec![BinaryCell {
            row_index: 0,
            column_index: 0,
            bytes: vec![0, 1, 2, 255],
        }];

        let rows = rows_from_query_result(&result.query_result).unwrap();
        assert_eq!(
            rows[0].get("payload"),
            Some(&binary_cell_value(&[0, 1, 2, 255]))
        );
        assert_eq!(
            super::super::binary_cell_bytes(rows[0].get("payload").unwrap()),
            Some(vec![0, 1, 2, 255])
        );
    }

    #[test]
    fn rows_reject_malformed_query_result_shapes() {
        let result = response(
            vec!["id", "name"],
            vec![
                QueryColumnMeta::new("id", "bigint"),
                QueryColumnMeta::new("name", "text"),
            ],
            vec![vec![Some("1")]],
        );

        let error = rows_from_query_result(&result.query_result)
            .expect_err("short rows must fail instead of becoming NULL");

        assert!(
            error
                .to_string()
                .contains("Invalid query result for data comparison")
        );
        assert!(error.to_string().contains("row 0 has width 1, expected 2"));
    }

    #[test]
    fn rows_reject_missing_mapping_targets() {
        let result = response(
            vec!["id"],
            vec![QueryColumnMeta::new("id", "bigint")],
            vec![vec![Some("1")]],
        );
        let mappings = vec![DataCompareColumnMapping {
            source: "missing".to_string(),
            target: "missing".to_string(),
        }];

        let error = rows_from_query_result_with_mappings(&result.query_result, &mappings, false)
            .expect_err("missing mapping targets must not be silently skipped");

        assert!(
            error
                .to_string()
                .contains("mapping target \"missing\" is missing")
        );
    }

    #[test]
    fn longtext_binary_sidecar_is_ignored_for_data_compare_values() {
        let mut result = response(
            vec!["payload"],
            vec![QueryColumnMeta::new("payload", "MYSQL_TYPE_LONG_BLOB")],
            vec![vec![Some("true")]],
        );
        result.query_result.binary_cells = vec![BinaryCell {
            row_index: 0,
            column_index: 0,
            bytes: b"true".to_vec(),
        }];

        let result = normalize_compare_table_data_response(
            result,
            None,
            &DatabaseType::MySQL,
            &[column("payload", "LONGTEXT")],
        )
        .unwrap();
        assert!(result.query_result.binary_cells.is_empty());

        let rows = rows_from_query_result(&result.query_result).unwrap();
        assert_eq!(rows[0]["payload"], serde_json::json!("true"));
        assert_eq!(
            format_value_for_database(
                &rows[0]["payload"],
                Some("LONGTEXT"),
                Some(DatabaseType::MySQL),
            ),
            "'true'"
        );
    }

    #[test]
    fn binary_sidecar_is_preserved_for_blob_columns() {
        let mut result = response(
            vec!["payload"],
            vec![QueryColumnMeta::new("payload", "MYSQL_TYPE_BLOB")],
            vec![vec![Some("0x0102")]],
        );
        result.query_result.binary_cells = vec![BinaryCell {
            row_index: 0,
            column_index: 0,
            bytes: vec![1, 2],
        }];

        let result = normalize_compare_table_data_response(
            result,
            None,
            &DatabaseType::MySQL,
            &[column("payload", "LONGBLOB")],
        )
        .unwrap();
        assert_eq!(result.query_result.binary_cells.len(), 1);
        let rows = rows_from_query_result(&result.query_result).unwrap();
        assert_eq!(
            format_value_for_database(
                &rows[0]["payload"],
                Some("LONGBLOB"),
                Some(DatabaseType::MySQL),
            ),
            "X'0102'"
        );
    }

    #[test]
    fn normalization_remaps_rowid_cells_before_dropping_text_sidecars() {
        let mut result = response(
            vec!["__rowid__", "id", "payload"],
            vec![
                QueryColumnMeta::new("__rowid__", "text"),
                QueryColumnMeta::new("id", "bigint"),
                QueryColumnMeta::new("payload", "longblob"),
            ],
            vec![vec![Some("row-1"), Some("1"), Some("true")]],
        );
        result.query_result.binary_cells = vec![
            BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: b"row-1".to_vec(),
            },
            BinaryCell {
                row_index: 0,
                column_index: 2,
                bytes: b"true".to_vec(),
            },
        ];

        let result = normalize_compare_table_data_response(
            result,
            Some("__rowid__"),
            &DatabaseType::MySQL,
            &[column("id", "BIGINT"), column("payload", "LONGTEXT")],
        )
        .unwrap();

        assert_eq!(result.query_result.columns, vec!["id", "payload"]);
        assert!(result.query_result.binary_cells.is_empty());
    }

    #[test]
    fn non_mysql_text_declared_runtime_blob_keeps_binary_sidecar() {
        let mut result = response(
            vec!["payload"],
            vec![QueryColumnMeta::new("payload", "blob")],
            vec![vec![Some("true")]],
        );
        result.query_result.binary_cells = vec![BinaryCell {
            row_index: 0,
            column_index: 0,
            bytes: b"true".to_vec(),
        }];

        let result = normalize_compare_table_data_response(
            result,
            None,
            &DatabaseType::SQLite,
            &[column("payload", "TEXT")],
        )
        .unwrap();
        assert_eq!(result.query_result.binary_cells.len(), 1);
    }

    #[test]
    fn mysql_binary_charset_text_keeps_binary_sidecar() {
        let mut result = response(
            vec!["payload"],
            vec![QueryColumnMeta::new("payload", "MYSQL_TYPE_LONG_BLOB")],
            vec![vec![Some("true")]],
        );
        result.query_result.binary_cells = vec![BinaryCell {
            row_index: 0,
            column_index: 0,
            bytes: b"true".to_vec(),
        }];
        let mut payload = column("payload", "LONGTEXT");
        payload.charset = Some("binary".to_string());

        let result =
            normalize_compare_table_data_response(result, None, &DatabaseType::MySQL, &[payload])
                .unwrap();
        assert_eq!(result.query_result.binary_cells.len(), 1);
    }

    #[test]
    fn mysql_character_types_are_reclassified_from_authoritative_schema() {
        for data_type in ["CHAR(8)", "VARCHAR(255)", "ENUM('a','b')", "SET('a','b')"] {
            let mut result = response(
                vec!["payload"],
                vec![QueryColumnMeta::new("payload", "MYSQL_TYPE_VAR_STRING")],
                vec![vec![Some("true")]],
            );
            result.query_result.binary_cells = vec![BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: b"true".to_vec(),
            }];

            let result = normalize_compare_table_data_response(
                result,
                None,
                &DatabaseType::MySQL,
                &[column("payload", data_type)],
            )
            .unwrap();
            assert!(
                result.query_result.binary_cells.is_empty(),
                "authoritative schema must classify {data_type} as text"
            );
            assert_eq!(result.query_result.rows[0][0].as_deref(), Some("true"));
        }
    }

    #[test]
    fn integer_cells_keep_large_values_as_json_numbers_and_canonicalize_signs() {
        let result = response(
            vec!["large", "positive", "negative_zero"],
            vec![
                QueryColumnMeta::new("large", "bigint"),
                QueryColumnMeta::new("positive", "bigint"),
                QueryColumnMeta::new("negative_zero", "integer"),
            ],
            vec![vec![
                Some("18446744073709551615"),
                Some("+00042"),
                Some("-0"),
            ]],
        );

        let rows = rows_from_query_result(&result.query_result).unwrap();
        assert!(rows[0]["large"].is_number());
        assert_eq!(rows[0]["large"].to_string(), "18446744073709551615");
        assert_eq!(rows[0]["positive"].to_string(), "42");
        assert_eq!(rows[0]["negative_zero"].to_string(), "0");
    }

    #[test]
    fn temporal_cells_trim_and_normalize_datetime_separator_without_changing_offsets() {
        let result = response(
            vec!["date", "time", "with_t", "with_space"],
            vec![
                QueryColumnMeta::new("date", "date"),
                QueryColumnMeta::new("time", "time"),
                QueryColumnMeta::new("with_t", "timestamp with time zone"),
                QueryColumnMeta::new("with_space", "timestamp with time zone"),
            ],
            vec![vec![
                Some(" 2024-01-01 "),
                Some(" 12:34:56 "),
                Some("2024-01-01T00:00:00+05:30"),
                Some(" 2024-01-01 00:00:00+05:30 "),
            ]],
        );

        let rows = rows_from_query_result(&result.query_result).unwrap();
        assert_eq!(rows[0]["date"], serde_json::json!("2024-01-01"));
        assert_eq!(rows[0]["time"], serde_json::json!("12:34:56"));
        assert_eq!(rows[0]["with_t"], rows[0]["with_space"]);
        assert_eq!(
            rows[0]["with_t"],
            serde_json::json!("2024-01-01 00:00:00+05:30")
        );
    }

    #[test]
    fn keyset_integer_cursor_rejects_decimal_literals() {
        let plugin = crate::mysql::MySqlPlugin::new();
        let result = response(
            vec!["id"],
            vec![QueryColumnMeta::new("id", "bigint")],
            vec![vec![Some("1.5")]],
        );

        let error = build_keyset_where_clause(
            &plugin,
            &["id".to_string()],
            &[keyset_column("id", "bigint")],
            &result.query_result,
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("plain integer literal"));
    }

    #[test]
    fn keyset_mysql_single_key_uses_last_row() {
        let plugin = crate::mysql::MySqlPlugin::new();
        let result = response(
            vec!["id"],
            vec![QueryColumnMeta::new("id", "bigint")],
            vec![vec![Some("41")], vec![Some("42")]],
        );

        let clause = build_keyset_where_clause(
            &plugin,
            &["id".to_string()],
            &[keyset_column("id", "bigint")],
            &result.query_result,
            false,
        )
        .unwrap();

        assert_eq!(clause.as_deref(), Some("(`id` > 42)"));
    }

    #[test]
    fn keyset_mysql_composite_key_expands_lexicographic_predicate() {
        let plugin = crate::mysql::MySqlPlugin::new();
        let result = response(
            vec!["tenant_id", "slug"],
            vec![
                QueryColumnMeta::new("tenant_id", "bigint"),
                QueryColumnMeta::new("slug", "varchar"),
            ],
            vec![vec![Some("7"), Some("a'b")]],
        );

        let clause = build_keyset_where_clause(
            &plugin,
            &["tenant_id".to_string(), "slug".to_string()],
            &[
                keyset_column("tenant_id", "bigint"),
                keyset_column("slug", "varchar"),
            ],
            &result.query_result,
            false,
        )
        .unwrap();

        assert_eq!(
            clause.as_deref(),
            Some("(`tenant_id` > 7) OR (`tenant_id` = 7 AND `slug` > 'a''b')")
        );
    }

    #[test]
    fn keyset_mssql_text_uses_unicode_literal() {
        let plugin = crate::mssql::MsSqlPlugin::new();
        let result = response(
            vec!["name"],
            vec![QueryColumnMeta::new("name", "nvarchar")],
            vec![vec![Some("中文")]],
        );

        let clause = build_keyset_where_clause(
            &plugin,
            &["name".to_string()],
            &[keyset_column("name", "nvarchar")],
            &result.query_result,
            false,
        )
        .unwrap();

        assert_eq!(clause.as_deref(), Some("([name] > N'中文')"));
    }

    #[test]
    fn keyset_falls_back_for_non_primary_nullable_or_unsupported_keys() {
        let plugin = crate::mysql::MySqlPlugin::new();
        let integer_result = response(
            vec!["id"],
            vec![QueryColumnMeta::new("id", "bigint")],
            vec![vec![Some("42")]],
        );
        let binary_result = response(
            vec!["payload"],
            vec![QueryColumnMeta::new("payload", "blob")],
            vec![vec![Some("<binary>")]],
        );

        assert!(
            build_keyset_where_clause(
                &plugin,
                &["id".to_string()],
                &[column("id", "bigint")],
                &integer_result.query_result,
                false,
            )
            .unwrap()
            .is_none()
        );
        assert!(
            build_keyset_where_clause(
                &plugin,
                &["id".to_string()],
                &[ColumnInfo {
                    is_nullable: false,
                    ..column("id", "bigint")
                }],
                &integer_result.query_result,
                false,
            )
            .unwrap()
            .is_none()
        );
        assert!(
            build_keyset_where_clause(
                &plugin,
                &["payload".to_string()],
                &[keyset_column("payload", "blob")],
                &binary_result.query_result,
                false,
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn keyset_support_is_conservative_for_external_and_unimplemented_databases() {
        for database_type in [
            DatabaseType::MySQL,
            DatabaseType::PostgreSQL,
            DatabaseType::SQLite,
            DatabaseType::MSSQL,
            DatabaseType::ClickHouse,
        ] {
            assert!(database_supports_compare_keyset(&database_type));
        }
        for database_type in [
            DatabaseType::DuckDB,
            DatabaseType::Oracle,
            DatabaseType::external("custom"),
        ] {
            assert!(!database_supports_compare_keyset(&database_type));
        }
    }

    #[test]
    fn keyset_rejects_missing_or_null_cursor_values() {
        let plugin = crate::mysql::MySqlPlugin::new();
        let missing_result = response(
            vec!["other"],
            vec![QueryColumnMeta::new("other", "bigint")],
            vec![vec![Some("42")]],
        );
        let null_result = response(
            vec!["id"],
            vec![QueryColumnMeta::new("id", "bigint")],
            vec![vec![None]],
        );
        let business_columns = [keyset_column("id", "bigint")];
        let key_columns = ["id".to_string()];

        let missing_error = build_keyset_where_clause(
            &plugin,
            &key_columns,
            &business_columns,
            &missing_result.query_result,
            false,
        )
        .unwrap_err();
        assert!(
            missing_error
                .to_string()
                .contains("missing from query results")
        );

        let null_error = build_keyset_where_clause(
            &plugin,
            &key_columns,
            &business_columns,
            &null_result.query_result,
            false,
        )
        .unwrap_err();
        assert!(null_error.to_string().contains("encountered NULL"));
    }

    #[test]
    fn applying_keyset_resets_offset_but_fallback_preserves_it() {
        let request = build_table_data_request(
            "app".to_string(),
            None,
            "users".to_string(),
            Some("`id`"),
            2,
            10_000,
            10_000,
        );

        let keyset_request = apply_keyset_where_clause(request.clone(), Some("(`id` > 42)"));
        assert_eq!(keyset_request.effective_offset(), 0);
        assert_eq!(keyset_request.where_clause.as_deref(), Some("(`id` > 42)"));

        let fallback_request = apply_keyset_where_clause(request, None);
        assert_eq!(fallback_request.effective_offset(), 10_000);
        assert!(fallback_request.where_clause.is_none());
    }

    #[test]
    fn stripping_internal_columns_remaps_binary_coordinates() {
        let mut response = response(
            vec!["__rowid__", "id", "payload"],
            vec![
                QueryColumnMeta::new("__rowid__", "bigint"),
                QueryColumnMeta::new("id", "bigint"),
                QueryColumnMeta::new("payload", "blob"),
            ],
            vec![vec![Some("99"), Some("1"), Some("<binary>")]],
        );
        response.query_result.binary_cells = vec![BinaryCell {
            row_index: 0,
            column_index: 2,
            bytes: vec![1, 2, 3],
        }];

        let response = strip_internal_compare_columns(response);
        assert_eq!(response.query_result.columns, vec!["id", "payload"]);
        assert_eq!(response.query_result.binary_cells[0].column_index, 1);
    }

    #[test]
    fn stripping_is_noop_without_plugin_rowid_support() {
        let mut response = response(
            vec!["__rowid__", "payload"],
            vec![
                QueryColumnMeta::new("__rowid__", "text"),
                QueryColumnMeta::new("payload", "blob"),
            ],
            vec![vec![Some("business"), Some("<binary>")]],
        );
        response.query_result.binary_cells = vec![BinaryCell {
            row_index: 0,
            column_index: 1,
            bytes: vec![1, 2, 3],
        }];

        let response = strip_internal_compare_columns_if(
            response,
            false,
            "__rowid__",
            &[column("__rowid__", "text"), column("payload", "blob")],
        );
        assert_eq!(response.query_result.columns, vec!["__rowid__", "payload"]);
        assert_eq!(response.query_result.binary_cells[0].column_index, 1);
    }

    #[test]
    fn stripping_supports_custom_internal_rowid_alias() {
        let response = response(
            vec!["dbx_rowid", "id"],
            vec![
                QueryColumnMeta::new("dbx_rowid", "text"),
                QueryColumnMeta::new("id", "bigint"),
            ],
            vec![vec![Some("AA"), Some("1")]],
        );

        let response = strip_internal_compare_columns_if(
            response,
            true,
            "dbx_rowid",
            &[column("id", "bigint")],
        );

        assert_eq!(response.query_result.columns, vec!["id"]);
        assert_eq!(
            response.query_result.rows,
            vec![vec![Some("1".to_string())]]
        );
    }

    #[test]
    fn stripping_only_removes_leading_internal_rowid_on_name_collision() {
        let mut response = response(
            vec!["__rowid__", "__rowid__", "payload"],
            vec![
                QueryColumnMeta::new("__rowid__", "bigint"),
                QueryColumnMeta::new("__rowid__", "text"),
                QueryColumnMeta::new("payload", "blob"),
            ],
            vec![vec![Some("99"), Some("business"), Some("<binary>")]],
        );
        response.query_result.binary_cells = vec![
            BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: vec![9],
            },
            BinaryCell {
                row_index: 0,
                column_index: 2,
                bytes: vec![1, 2, 3],
            },
        ];

        let response = strip_internal_compare_columns_if(
            response,
            true,
            "__rowid__",
            &[column("__rowid__", "text"), column("payload", "blob")],
        );
        assert_eq!(response.query_result.columns, vec!["__rowid__", "payload"]);
        assert_eq!(
            response.query_result.rows,
            vec![vec![
                Some("business".to_string()),
                Some("<binary>".to_string())
            ]]
        );
        assert_eq!(response.query_result.column_meta.len(), 2);
        assert_eq!(response.query_result.binary_cells.len(), 1);
        assert_eq!(response.query_result.binary_cells[0].column_index, 1);
        assert_eq!(response.query_result.binary_cells[0].bytes, vec![1, 2, 3]);
    }

    #[test]
    fn stripping_does_not_remove_non_leading_business_rowid() {
        let response = response(
            vec!["id", "__rowid__", "payload"],
            vec![
                QueryColumnMeta::new("id", "bigint"),
                QueryColumnMeta::new("__rowid__", "text"),
                QueryColumnMeta::new("payload", "blob"),
            ],
            vec![vec![Some("1"), Some("business"), Some("<binary>")]],
        );

        let response = strip_internal_compare_columns_if(
            response,
            true,
            "__rowid__",
            &[
                column("id", "bigint"),
                column("__rowid__", "text"),
                column("payload", "blob"),
            ],
        );
        assert_eq!(
            response.query_result.columns,
            vec!["id", "__rowid__", "payload"]
        );
    }

    #[test]
    fn appending_pages_checks_count_and_offsets_binary_rows() {
        let mut first = response(
            vec!["id"],
            vec![QueryColumnMeta::new("id", "int")],
            vec![vec![Some("1")]],
        );
        first.total_count = 2;
        let mut second = response(
            vec!["id"],
            vec![QueryColumnMeta::new("id", "int")],
            vec![vec![Some("2")]],
        );
        second.total_count = 2;
        second.query_result.binary_cells = vec![BinaryCell {
            row_index: 0,
            column_index: 0,
            bytes: vec![2],
        }];
        let mut accumulated = None;

        append_table_data_page(&mut accumulated, first).unwrap();
        append_table_data_page(&mut accumulated, second).unwrap();

        let accumulated = accumulated.unwrap();
        assert_eq!(accumulated.query_result.rows.len(), 2);
        assert_eq!(accumulated.query_result.binary_cells[0].row_index, 1);
    }

    #[test]
    fn appending_rejects_malformed_first_page() {
        let malformed = response(
            vec!["id", "name"],
            vec![
                QueryColumnMeta::new("id", "int"),
                QueryColumnMeta::new("name", "text"),
            ],
            vec![vec![Some("1")]],
        );
        let mut accumulated = None;

        let error = append_table_data_page(&mut accumulated, malformed)
            .expect_err("malformed pages must fail before accumulation");

        assert!(error.to_string().contains("Invalid table data page"));
        assert!(accumulated.is_none());
    }

    #[test]
    fn appending_rejects_malformed_later_page_without_mutating_accumulator() {
        let mut first = response(
            vec!["id", "name"],
            vec![
                QueryColumnMeta::new("id", "int"),
                QueryColumnMeta::new("name", "text"),
            ],
            vec![vec![Some("1"), Some("first")]],
        );
        first.total_count = 2;
        let mut malformed = response(
            vec!["id", "name"],
            vec![
                QueryColumnMeta::new("id", "int"),
                QueryColumnMeta::new("name", "text"),
            ],
            vec![vec![Some("2"), Some("second")]],
        );
        malformed.total_count = 2;
        malformed.query_result.binary_cells = vec![
            BinaryCell {
                row_index: 0,
                column_index: 1,
                bytes: vec![1],
            },
            BinaryCell {
                row_index: 0,
                column_index: 1,
                bytes: vec![2],
            },
        ];
        let mut accumulated = None;
        append_table_data_page(&mut accumulated, first).unwrap();

        let error = append_table_data_page(&mut accumulated, malformed)
            .expect_err("duplicate binary coordinates must fail");

        assert!(error.to_string().contains("Invalid table data page"));
        let accumulated = accumulated.expect("first page must remain intact");
        assert_eq!(accumulated.query_result.rows.len(), 1);
        assert!(accumulated.query_result.binary_cells.is_empty());
    }

    #[test]
    fn paging_rejects_short_page_before_count() {
        let error = table_data_page_is_complete(5_000, 10_001, 5_000, 10_000).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("fewer rows while paging than COUNT")
        );
    }

    #[test]
    fn paging_rejects_empty_page_before_count() {
        let error = table_data_page_is_complete(10_000, 10_001, 0, 10_000).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("fewer rows while paging than COUNT")
        );
    }

    #[test]
    fn paging_completes_only_at_exact_count() {
        assert!(table_data_page_is_complete(10_001, 10_001, 1, 10_000).unwrap());
        assert!(!table_data_page_is_complete(10_000, 10_001, 10_000, 10_000).unwrap());
        assert!(table_data_page_is_complete(0, 0, 0, 10_000).unwrap());
    }

    #[test]
    fn full_final_page_requires_an_empty_terminal_probe() {
        assert!(table_data_terminal_probe_required(
            10_000, 10_000, 10_000, 10_000
        ));
        assert!(!table_data_terminal_probe_required(
            10_001, 10_001, 1, 10_000
        ));
        assert!(!table_data_terminal_probe_required(0, 0, 0, 10_000));
    }

    #[test]
    fn paging_request_uses_accumulated_offset_when_limit_shrinks_page_size() {
        let request = build_table_data_request(
            "app".to_string(),
            None,
            "users".to_string(),
            Some("\"id\""),
            2,
            10_000,
            5_000,
        );

        assert_eq!(request.page, 2);
        assert_eq!(request.page_size, 5_000);
        assert_eq!(request.effective_offset(), 10_000);
    }

    #[test]
    fn paging_rejects_rows_beyond_count() {
        let error = table_data_page_is_complete(10_002, 10_001, 2, 10_000).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("more rows while paging than COUNT")
        );
    }

    #[test]
    fn paging_truncates_at_row_limit_without_overshooting_it() {
        let limits = DataCompareLimits {
            max_rows_per_table: Some(15_000),
            max_pages_per_table: None,
        };

        assert_eq!(
            data_compare_next_page_size(10_000, 0, limits).unwrap(),
            10_000
        );
        assert_eq!(
            data_compare_next_page_size(10_000, 10_000, limits).unwrap(),
            5_000
        );
        assert_eq!(
            data_compare_paging_decision(15_000, 20_000, 5_000, 5_000, 2, limits).unwrap(),
            DataComparePagingDecision::Truncated
        );
    }

    #[test]
    fn paging_truncates_at_page_limit() {
        let limits = DataCompareLimits {
            max_rows_per_table: None,
            max_pages_per_table: Some(2),
        };

        assert_eq!(
            data_compare_paging_decision(20_000, 20_001, 10_000, 10_000, 2, limits).unwrap(),
            DataComparePagingDecision::Truncated
        );
    }

    #[test]
    fn exact_count_wins_over_limits() {
        let limits = DataCompareLimits {
            max_rows_per_table: Some(20_000),
            max_pages_per_table: Some(2),
        };

        assert_eq!(
            data_compare_paging_decision(20_000, 20_000, 10_000, 10_000, 2, limits).unwrap(),
            DataComparePagingDecision::Complete
        );
    }

    #[test]
    fn paging_rejects_zero_limits() {
        let error = data_compare_next_page_size(
            10_000,
            0,
            DataCompareLimits {
                max_rows_per_table: Some(0),
                max_pages_per_table: None,
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("must be greater than zero"));
    }
}
