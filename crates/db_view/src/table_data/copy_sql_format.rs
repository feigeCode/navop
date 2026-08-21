use db::{ColumnInfo, TableCellValue};
use gpui::SharedString;

use super::{CopyCell, CopyFormat, CopyFormatContext};

pub(super) fn format(format: CopyFormat, context: CopyFormatContext<'_>) -> String {
    match format {
        CopyFormat::SqlInsert => format_insert(context),
        CopyFormat::SqlUpdate => format_update(context),
        CopyFormat::SqlDelete => format_delete(context),
        CopyFormat::SqlIn => format_in(context),
        _ => String::new(),
    }
}

fn format_insert(context: CopyFormatContext<'_>) -> String {
    let Some(column_count) = uniform_row_width(context.data) else {
        return String::new();
    };
    if column_count == 0 {
        return String::new();
    }
    let table = table_identifier(context);
    let columns = (0..column_count)
        .map(|index| column_identifier(context, index))
        .collect::<Vec<_>>();
    let values = context
        .data
        .iter()
        .enumerate()
        .map(|(row_index, row)| {
            let values = row
                .iter()
                .enumerate()
                .map(|(column_index, _)| sql_value(context, row_index, column_index))
                .collect::<Vec<_>>();
            format!("({})", values.join(", "))
        })
        .collect::<Vec<_>>();
    format!(
        "INSERT INTO {table} ({}) VALUES\n{};",
        columns.join(", "),
        values.join(",\n")
    )
}

fn format_update(context: CopyFormatContext<'_>) -> String {
    if context.data.is_empty() {
        return String::new();
    }
    let primary_keys = primary_key_indices(context);
    if primary_keys.is_empty() {
        return String::new();
    }
    context
        .data
        .iter()
        .enumerate()
        .filter_map(|(row_index, row)| format_update_row(context, row_index, row, &primary_keys))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_update_row(
    context: CopyFormatContext<'_>,
    row_index: usize,
    row: &[Option<String>],
    primary_keys: &[usize],
) -> Option<String> {
    let set_parts = row
        .iter()
        .enumerate()
        .filter(|(index, _)| !primary_keys.contains(index))
        .map(|(column_index, _)| assignment(context, row_index, column_index))
        .collect::<Vec<_>>();
    let where_parts = primary_key_predicates(context, row_index, row, primary_keys)?;
    (!set_parts.is_empty() && !where_parts.is_empty()).then(|| {
        format!(
            "UPDATE {} SET {} WHERE {};",
            table_identifier(context),
            set_parts.join(", "),
            where_parts.join(" AND ")
        )
    })
}

fn format_delete(context: CopyFormatContext<'_>) -> String {
    if context.data.is_empty() {
        return String::new();
    }
    let primary_keys = primary_key_indices(context);
    if primary_keys.is_empty() {
        return String::new();
    }
    context
        .data
        .iter()
        .enumerate()
        .filter_map(|(row_index, row)| {
            let parts = primary_key_predicates(context, row_index, row, &primary_keys)?;
            (!parts.is_empty()).then(|| {
                format!(
                    "DELETE FROM {} WHERE {};",
                    table_identifier(context),
                    parts.join(" AND ")
                )
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_in(context: CopyFormatContext<'_>) -> String {
    let Some(column_count) = uniform_row_width(context.data) else {
        return String::new();
    };
    if column_count == 0 {
        return String::new();
    }
    if column_count == 1 {
        return format_single_column_in(context);
    }
    context
        .data
        .iter()
        .enumerate()
        .map(|(row_index, row)| {
            let parts = row
                .iter()
                .enumerate()
                .map(|(column_index, _)| predicate(context, row_index, column_index))
                .collect::<Vec<_>>();
            format!("({})", parts.join(" AND "))
        })
        .collect::<Vec<_>>()
        .join(" OR\n")
}

fn format_single_column_in(context: CopyFormatContext<'_>) -> String {
    let column = column_identifier(context, 0);
    let mut values = Vec::new();
    let mut has_null = false;
    for (row_index, row) in context.data.iter().enumerate() {
        if row.is_empty() {
            continue;
        }
        match context.cell(row_index, 0) {
            CopyCell::Null => has_null = true,
            CopyCell::Text(_) | CopyCell::Binary(_) => {
                values.push(sql_value(context, row_index, 0));
            }
        }
    }
    match (values.is_empty(), has_null) {
        (false, true) => format!("({column} IN ({}) OR {column} IS NULL)", values.join(", ")),
        (false, false) => format!("{column} IN ({})", values.join(", ")),
        (true, true) => format!("{column} IS NULL"),
        (true, false) => String::new(),
    }
}

fn primary_key_indices(context: CopyFormatContext<'_>) -> Vec<usize> {
    context.metadata.primary_key_indices.clone()
}

fn primary_key_predicates(
    context: CopyFormatContext<'_>,
    row_index: usize,
    row: &[Option<String>],
    primary_keys: &[usize],
) -> Option<Vec<String>> {
    primary_keys
        .iter()
        .map(|column_index| {
            row.get(*column_index)?;
            match context.cell(row_index, *column_index) {
                CopyCell::Null => None,
                CopyCell::Text(_) | CopyCell::Binary(_) => {
                    Some(predicate(context, row_index, *column_index))
                }
            }
        })
        .collect()
}

fn uniform_row_width(rows: &[Vec<Option<String>>]) -> Option<usize> {
    let width = rows.first()?.len();
    rows.iter().all(|row| row.len() == width).then_some(width)
}

fn assignment(context: CopyFormatContext<'_>, row_index: usize, column_index: usize) -> String {
    format!(
        "{} = {}",
        column_identifier(context, column_index),
        sql_value(context, row_index, column_index)
    )
}

fn predicate(context: CopyFormatContext<'_>, row_index: usize, column_index: usize) -> String {
    let column = column_identifier(context, column_index);
    match context.cell(row_index, column_index) {
        CopyCell::Null => format!("{column} IS NULL"),
        CopyCell::Text(_) | CopyCell::Binary(_) => {
            format!("{column} = {}", sql_value(context, row_index, column_index))
        }
    }
}

fn sql_value(context: CopyFormatContext<'_>, row_index: usize, column_index: usize) -> String {
    let value = match context.cell(row_index, column_index) {
        CopyCell::Null => TableCellValue::Null,
        CopyCell::Text(value) => TableCellValue::Text(value.to_string()),
        CopyCell::Binary(bytes) => TableCellValue::Binary(bytes.to_vec()),
    };
    let column = column_meta(context, column_index);
    if let Some(plugin) = context.plugin {
        return plugin.format_table_change_value(&value, column);
    }
    match value {
        TableCellValue::Null => "NULL".to_string(),
        TableCellValue::Text(value) => fallback_sql_value(&value),
        TableCellValue::Binary(bytes) => {
            let hex = bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            format!("X'{hex}'")
        }
    }
}

fn column_meta(context: CopyFormatContext<'_>, index: usize) -> Option<&ColumnInfo> {
    context
        .metadata
        .column_meta
        .get(index)
        .and_then(Option::as_ref)
}

fn fallback_sql_value(value: &str) -> String {
    if is_finite_numeric_literal(value) {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "''"))
}

fn is_finite_numeric_literal(value: &str) -> bool {
    value.parse::<i64>().is_ok() || value.parse::<f64>().is_ok_and(f64::is_finite)
}

fn table_identifier(context: CopyFormatContext<'_>) -> String {
    let table = if context.metadata.table_name.is_empty() {
        "table_name"
    } else {
        context.metadata.table_name.as_ref()
    };
    quote_identifier(context, table)
}

fn column_identifier(context: CopyFormatContext<'_>, index: usize) -> String {
    let column = context
        .columns
        .get(index)
        .map_or("col", SharedString::as_ref);
    quote_identifier(context, column)
}

fn quote_identifier(context: CopyFormatContext<'_>, identifier: &str) -> String {
    context
        .plugin
        .map(|plugin| plugin.quote_identifier(identifier))
        .unwrap_or_else(|| format!("\"{}\"", identifier.replace('"', "\"\"")))
}
