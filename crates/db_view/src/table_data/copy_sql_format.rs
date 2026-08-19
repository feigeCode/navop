use db::{ColumnInfo, TableCellValue};
use gpui::SharedString;

use super::{CopyFormat, CopyFormatContext};

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
    if context.data.is_empty() {
        return String::new();
    }
    let table = table_identifier(context);
    let column_count = context.data.first().map_or(0, Vec::len);
    let columns = (0..column_count)
        .map(|index| column_identifier(context, index))
        .collect::<Vec<_>>();
    let values = context
        .data
        .iter()
        .map(|row| {
            let values = row
                .iter()
                .enumerate()
                .map(|(index, value)| sql_value(context, index, value))
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
    context
        .data
        .iter()
        .filter_map(|row| format_update_row(context, row, &primary_keys))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_update_row(
    context: CopyFormatContext<'_>,
    row: &[Option<String>],
    primary_keys: &[usize],
) -> Option<String> {
    let set_parts = row
        .iter()
        .enumerate()
        .filter(|(index, _)| !primary_keys.contains(index))
        .map(|(index, value)| assignment(context, index, value))
        .collect::<Vec<_>>();
    let where_parts = primary_keys
        .iter()
        .filter_map(|index| {
            row.get(*index)
                .map(|value| predicate(context, *index, value))
        })
        .collect::<Vec<_>>();
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
    context
        .data
        .iter()
        .filter_map(|row| {
            let parts = primary_keys
                .iter()
                .filter_map(|index| {
                    row.get(*index)
                        .map(|value| predicate(context, *index, value))
                })
                .collect::<Vec<_>>();
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
    let Some(first_row) = context.data.first() else {
        return String::new();
    };
    if first_row.len() == 1 {
        return format_single_column_in(context);
    }
    context
        .data
        .iter()
        .map(|row| {
            let parts = row
                .iter()
                .enumerate()
                .map(|(index, value)| predicate(context, index, value))
                .collect::<Vec<_>>();
            format!("({})", parts.join(" AND "))
        })
        .collect::<Vec<_>>()
        .join(" OR\n")
}

fn format_single_column_in(context: CopyFormatContext<'_>) -> String {
    let column = column_identifier(context, 0);
    let values = context
        .data
        .iter()
        .filter_map(|row| row.first())
        .filter(|value| value.is_some())
        .map(|value| sql_value(context, 0, value))
        .collect::<Vec<_>>();
    let has_null = context
        .data
        .iter()
        .filter_map(|row| row.first())
        .any(Option::is_none);
    match (values.is_empty(), has_null) {
        (false, true) => format!("({column} IN ({}) OR {column} IS NULL)", values.join(", ")),
        (false, false) => format!("{column} IN ({})", values.join(", ")),
        (true, true) => format!("{column} IS NULL"),
        (true, false) => String::new(),
    }
}

fn primary_key_indices(context: CopyFormatContext<'_>) -> Vec<usize> {
    if context.metadata.primary_key_indices.is_empty() {
        vec![0]
    } else {
        context.metadata.primary_key_indices.clone()
    }
}

fn assignment(context: CopyFormatContext<'_>, index: usize, value: &Option<String>) -> String {
    format!(
        "{} = {}",
        column_identifier(context, index),
        sql_value(context, index, value)
    )
}

fn predicate(context: CopyFormatContext<'_>, index: usize, value: &Option<String>) -> String {
    let column = column_identifier(context, index);
    match value {
        None => format!("{column} IS NULL"),
        Some(_) => format!("{column} = {}", sql_value(context, index, value)),
    }
}

fn sql_value(context: CopyFormatContext<'_>, index: usize, value: &Option<String>) -> String {
    let Some(value) = value else {
        return "NULL".to_string();
    };
    let column = column_meta(context, index);
    if let Some(plugin) = context.plugin {
        return plugin.format_table_change_value(&TableCellValue::Text(value.clone()), column);
    }
    fallback_sql_value(value)
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
