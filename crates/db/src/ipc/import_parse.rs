use anyhow::{Result, anyhow};
use extension_protocol::data as wire_data;
use extension_protocol::row::{CellValue, Row};
use serde_json::Value;

use crate::import_export::formats::CsvFormatHandler;
use crate::import_export::{DataFormat, ImportConfig};

pub(crate) struct ParsedRows {
    pub format: wire_data::DataFormat,
    pub columns: Vec<String>,
    pub rows: Vec<Row>,
}

pub(crate) fn parse_rows(config: &ImportConfig, data: &str) -> Result<ParsedRows> {
    match config.format {
        DataFormat::Json => parse_json_rows(data),
        DataFormat::Csv => parse_delimited_rows(config, data, wire_data::DataFormat::Csv),
        DataFormat::Txt => parse_txt_rows(data),
        DataFormat::Sql | DataFormat::Xml => unreachable!("handled by fallback"),
    }
}

fn parse_json_rows(data: &str) -> Result<ParsedRows> {
    let value: Value = serde_json::from_str(data)?;
    let rows = match value {
        Value::Array(rows) => rows,
        Value::Object(_) => vec![value],
        _ => return Err(anyhow!("JSON must be array or object")),
    };
    if rows.is_empty() {
        return Ok(parsed(wire_data::DataFormat::Json, Vec::new(), Vec::new()));
    }
    let first = rows[0]
        .as_object()
        .ok_or_else(|| anyhow!("JSON array must contain objects"))?;
    let columns: Vec<String> = first.keys().cloned().collect();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let object = row
            .as_object()
            .ok_or_else(|| anyhow!("JSON array must contain objects"))?;
        out.push(
            columns
                .iter()
                .map(|column| json_to_cell(object.get(column).unwrap_or(&Value::Null)))
                .collect(),
        );
    }
    Ok(parsed(wire_data::DataFormat::Json, columns, out))
}

fn parse_delimited_rows(
    config: &ImportConfig,
    data: &str,
    format: wire_data::DataFormat,
) -> Result<ParsedRows> {
    let csv = config.csv_config.clone().unwrap_or_default();
    let records =
        CsvFormatHandler::parse_csv_data_with_config(data, csv.field_delimiter, csv.text_qualifier);
    if records.is_empty() {
        return Ok(parsed(format, Vec::new(), Vec::new()));
    }
    let (columns, values) = if csv.has_header {
        (header_columns(&records[0])?, &records[1..])
    } else {
        (generated_columns(records[0].len()), records.as_slice())
    };
    Ok(parsed(
        format,
        columns,
        values.iter().map(optional_strings_to_row).collect(),
    ))
}

fn parse_txt_rows(data: &str) -> Result<ParsedRows> {
    let lines: Vec<&str> = data.lines().collect();
    if lines.is_empty() {
        return Ok(parsed(wire_data::DataFormat::Csv, Vec::new(), Vec::new()));
    }
    let columns: Vec<String> = lines[0].split('\t').map(str::to_string).collect();
    let rows = lines
        .iter()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split('\t').map(text_cell).collect())
        .collect();
    Ok(parsed(wire_data::DataFormat::Csv, columns, rows))
}

fn parsed(format: wire_data::DataFormat, columns: Vec<String>, rows: Vec<Row>) -> ParsedRows {
    ParsedRows {
        format,
        columns,
        rows,
    }
}

fn header_columns(values: &[Option<String>]) -> Result<Vec<String>> {
    let columns: Vec<String> = values
        .iter()
        .map(|value| value.clone().unwrap_or_default())
        .collect();
    if columns.iter().any(|column| column.trim().is_empty()) {
        return Err(anyhow!("CSV header contains empty column names"));
    }
    Ok(columns)
}

fn generated_columns(len: usize) -> Vec<String> {
    (0..len).map(|index| format!("col{}", index + 1)).collect()
}

fn optional_strings_to_row(values: &Vec<Option<String>>) -> Row {
    values
        .iter()
        .map(|value| match value {
            Some(value) => text_cell(value),
            None => CellValue::Null,
        })
        .collect()
}

fn json_to_cell(value: &Value) -> CellValue {
    match value {
        Value::Null => CellValue::Null,
        Value::Bool(value) => CellValue::Bool { value: *value },
        Value::Number(value) => number_to_cell(value),
        Value::String(value) => text_cell(value),
        Value::Array(_) | Value::Object(_) => CellValue::Json {
            value: value.clone(),
        },
    }
}

fn number_to_cell(value: &serde_json::Number) -> CellValue {
    if let Some(value) = value.as_i64() {
        CellValue::I64 { value }
    } else if let Some(value) = value.as_u64() {
        CellValue::U64 { value }
    } else {
        CellValue::F64 {
            value: value.as_f64().unwrap_or_default(),
        }
    }
}

fn text_cell(value: impl ToString) -> CellValue {
    CellValue::Text {
        value: value.to_string(),
    }
}
