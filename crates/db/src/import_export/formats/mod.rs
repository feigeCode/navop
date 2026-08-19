use crate::DatabasePlugin;
use crate::connection::DbConnection;
use crate::import_export::ImportConfig;
use crate::types::ColumnInfo;
use one_core::storage::DatabaseType;

pub mod csv;
mod import_execution;
pub mod json;
pub mod sql;
mod sql_export;
pub mod txt;
pub mod xml;
mod xml_codec;

pub use csv::CsvFormatHandler;
pub use json::JsonFormatHandler;
pub use sql::SqlFormatHandler;
pub use txt::TxtFormatHandler;
pub use xml::XmlFormatHandler;

pub(super) fn format_import_table_reference(
    plugin: &dyn DatabasePlugin,
    config: &ImportConfig,
    table: &str,
) -> String {
    plugin.format_table_reference(&config.database, config.schema.as_deref(), table)
}

pub(super) async fn load_mysql_import_columns(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ImportConfig,
    table: &str,
) -> anyhow::Result<Vec<ColumnInfo>> {
    if plugin.name() != DatabaseType::MySQL {
        return Ok(Vec::new());
    }

    plugin
        .list_columns(connection, &config.database, config.schema.clone(), table)
        .await
}

pub(super) fn format_import_text_value(
    plugin: &dyn DatabasePlugin,
    value: &Option<String>,
    column_name: &str,
    table_columns: &[ColumnInfo],
) -> String {
    let Some(value) = value else {
        return "NULL".to_string();
    };
    let column_info = table_columns
        .iter()
        .find(|column| column.name == column_name)
        .or_else(|| {
            table_columns
                .iter()
                .find(|column| column.name.eq_ignore_ascii_case(column_name))
        });
    if column_info.is_some_and(|column| is_mysql_bit_type(&column.data_type)) {
        if let Some(literal) = format_mysql_bit_literal(value) {
            return literal;
        }
    }

    plugin.escape_sql_value(value)
}

fn is_mysql_bit_type(data_type: &str) -> bool {
    let data_type = data_type.trim().to_ascii_uppercase();
    data_type == "BIT" || data_type.starts_with("BIT(") || data_type.starts_with("BIT ")
}

fn format_mysql_bit_literal(value: &str) -> Option<String> {
    let value = value.trim();

    if value.eq_ignore_ascii_case("true") {
        return Some("1".to_string());
    }
    if value.eq_ignore_ascii_case("false") {
        return Some("0".to_string());
    }
    if value.parse::<u64>().is_ok() {
        return Some(value.to_string());
    }

    let hex = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))?;
    if hex.is_empty() || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }

    Some(format!("0x{hex}"))
}

#[cfg(test)]
mod sql_import_tests;

#[cfg(test)]
mod table_import_tests;

#[cfg(test)]
mod xml_tests;

#[cfg(test)]
mod tests {
    use super::format_import_table_reference;
    use crate::import_export::ImportConfig;
    use crate::mssql::MsSqlPlugin;
    use crate::mysql::MySqlPlugin;

    #[test]
    fn test_format_import_table_reference_uses_database_for_mysql() {
        let plugin = MySqlPlugin::new();
        let config = ImportConfig {
            database: "analytics".to_string(),
            table: Some("orders".to_string()),
            ..ImportConfig::default()
        };

        let table_ref = format_import_table_reference(&plugin, &config, "orders");

        assert_eq!(table_ref, "`analytics`.`orders`");
    }

    #[test]
    fn test_format_import_table_reference_uses_schema_for_mssql() {
        let plugin = MsSqlPlugin::new();
        let config = ImportConfig {
            database: "warehouse".to_string(),
            schema: Some("sales".to_string()),
            table: Some("orders".to_string()),
            ..ImportConfig::default()
        };

        let table_ref = format_import_table_reference(&plugin, &config, "orders");

        assert_eq!(table_ref, "[warehouse].[sales].[orders]");
    }
}
