use crate::DatabasePlugin;
use crate::connection::{DbConnection, DbError};
use crate::import_export::ImportConfig;
use crate::types::{ColumnInfo, TableCellValue};

pub mod csv;
mod import_execution;
pub mod json;
mod json_import;
pub mod sql;
pub(crate) mod sql_export;
pub mod txt;
pub mod xml;
mod xml_codec;
mod xml_import;

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

pub(super) async fn load_import_columns(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ImportConfig,
    table: &str,
) -> anyhow::Result<Vec<ColumnInfo>> {
    match plugin
        .list_columns(connection, &config.database, config.schema.clone(), table)
        .await
    {
        Ok(columns) => Ok(columns),
        Err(error) if is_not_supported(&error) => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

pub(super) fn format_import_text_value(
    plugin: &dyn DatabasePlugin,
    value: &Option<String>,
    column_name: &str,
    table_columns: &[ColumnInfo],
) -> String {
    let column = table_columns
        .iter()
        .find(|column| column.name == column_name)
        .or_else(|| {
            table_columns
                .iter()
                .find(|column| column.name.eq_ignore_ascii_case(column_name))
        });
    let value = value.as_ref().map_or(TableCellValue::Null, |value| {
        TableCellValue::Text(value.clone())
    });
    plugin.format_table_change_value(&value, column)
}

fn is_not_supported(error: &anyhow::Error) -> bool {
    error.to_string().contains("operation not supported:")
        || error.chain().any(|cause| {
            cause
                .downcast_ref::<DbError>()
                .is_some_and(|error| matches!(error, DbError::NotSupported(_)))
        })
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
