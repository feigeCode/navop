use crate::connection::{DbConnection, DbError};
use crate::executor::SqlResult;
use crate::import_export::{
    ExportConfig, ExportProgressSender, ExportResult, ImportConfig, ImportProgressSender,
    ImportResult,
};
use crate::ipc::connection::ExternalDbConnection;
use crate::ipc::protocol::{database_metadata_params, table_metadata_params};
use crate::ipc::registry::{EXTERNAL_DRIVER_ID_PARAM, IpcDriverManifest, IpcDriverRegistry};
use crate::plugin::{DatabasePlugin, SqlCompletionInfo};
use crate::plugin_manifest::{DatabaseCapabilities, DatabaseUiCapabilities, DatabaseUiManifest};
use crate::types::*;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use one_core::storage::{DatabaseType, DbConnectionConfig};
use sqlparser::dialect::{Dialect, GenericDialect};

#[derive(Clone)]
pub struct ExternalDatabasePlugin {
    registry: IpcDriverRegistry,
}

impl ExternalDatabasePlugin {
    pub fn new() -> Self {
        Self {
            registry: IpcDriverRegistry::load_default(),
        }
    }

    fn driver_for_config(&self, config: &DbConnectionConfig) -> Result<IpcDriverManifest, DbError> {
        let driver_id = config
            .get_param(EXTERNAL_DRIVER_ID_PARAM)
            .ok_or_else(|| DbError::connection("external driver id is required"))?;
        self.registry.find(driver_id).ok_or_else(|| {
            DbError::connection(format!("external driver '{}' not found", driver_id))
        })
    }

    async fn metadata<T>(
        &self,
        connection: &dyn DbConnection,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let sql = serde_json::json!({ "method": method, "params": params }).to_string();
        match connection
            .query(&format!("/*onetcli-ipc-metadata*/ {sql}"))
            .await?
        {
            SqlResult::Query(query) => decode_single_cell(query),
            SqlResult::Error(error) => Err(anyhow!(error.message)),
            SqlResult::Exec(_) => Err(anyhow!(
                "external driver returned non-query metadata result"
            )),
        }
    }

    async fn optional_metadata<T>(
        &self,
        connection: &dyn DbConnection,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        match self.metadata(connection, method, params).await {
            Ok(value) => Ok(Some(value)),
            Err(error) if is_not_supported(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl Default for ExternalDatabasePlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DatabasePlugin for ExternalDatabasePlugin {
    fn name(&self) -> DatabaseType {
        DatabaseType::External
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{}\"", identifier.replace('"', "\"\""))
    }

    fn get_completion_info(&self) -> SqlCompletionInfo {
        SqlCompletionInfo::default().with_standard_sql()
    }

    async fn create_connection(
        &self,
        config: DbConnectionConfig,
    ) -> Result<Box<dyn DbConnection + Send + Sync>, DbError> {
        let driver = self.driver_for_config(&config)?;
        Ok(Box::new(ExternalDbConnection::new(config, driver)))
    }

    async fn list_databases(&self, connection: &dyn DbConnection) -> Result<Vec<String>> {
        self.metadata(connection, "metadata.list_databases", serde_json::json!({}))
            .await
    }

    async fn list_databases_view(&self, connection: &dyn DbConnection) -> Result<ObjectView> {
        let rows = self
            .list_databases_detailed(connection)
            .await?
            .into_iter()
            .map(|db| vec![db.name, db.comment.unwrap_or_default()])
            .collect();
        Ok(object_view(
            DbNodeType::Database,
            "Databases",
            vec!["Name", "Comment"],
            rows,
        ))
    }

    async fn list_databases_detailed(
        &self,
        connection: &dyn DbConnection,
    ) -> Result<Vec<DatabaseInfo>> {
        match self
            .metadata(
                connection,
                "metadata.list_databases_detailed",
                serde_json::json!({}),
            )
            .await
        {
            Ok(databases) => Ok(databases),
            Err(error) if is_not_supported(&error) => {
                Ok(names_to_databases(self.list_databases(connection).await?))
            }
            Err(error) => Err(error),
        }
    }

    fn capabilities(&self) -> DatabaseCapabilities {
        merge_capabilities(
            self.registry
                .drivers()
                .iter()
                .map(IpcDriverManifest::effective_capabilities),
        )
    }

    fn sql_dialect(&self) -> Box<dyn Dialect> {
        Box::new(GenericDialect {})
    }

    async fn list_schemas(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<String>> {
        self.metadata(
            connection,
            "metadata.list_schemas",
            database_metadata_params(database, None),
        )
        .await
    }

    async fn list_tables(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<TableInfo>> {
        self.metadata(
            connection,
            "metadata.list_tables",
            database_metadata_params(database, schema),
        )
        .await
    }

    async fn list_tables_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<ObjectView> {
        let rows = self
            .list_tables(connection, database, schema)
            .await?
            .into_iter()
            .map(|table| vec![table.name, table.comment.unwrap_or_default()])
            .collect();
        Ok(object_view(
            DbNodeType::Table,
            "Tables",
            vec!["Name", "Comment"],
            rows,
        ))
    }

    async fn list_columns(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<ColumnInfo>> {
        self.metadata(
            connection,
            "metadata.list_columns",
            table_metadata_params(database, schema, table),
        )
        .await
    }

    async fn list_columns_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<ObjectView> {
        let rows = self
            .list_columns(connection, database, schema, table)
            .await?
            .into_iter()
            .map(|col| vec![col.name, col.data_type, col.is_nullable.to_string()])
            .collect();
        Ok(object_view(
            DbNodeType::Column,
            "Columns",
            vec!["Name", "Type", "Nullable"],
            rows,
        ))
    }

    async fn list_indexes(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<IndexInfo>> {
        self.metadata(
            connection,
            "metadata.list_indexes",
            table_metadata_params(database, schema, table),
        )
        .await
    }

    async fn list_indexes_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<ObjectView> {
        let rows = self
            .list_indexes(connection, database, schema.map(str::to_string), table)
            .await?
            .into_iter()
            .map(|idx| vec![idx.name, idx.columns.join(", "), idx.is_unique.to_string()])
            .collect();
        Ok(object_view(
            DbNodeType::Index,
            "Indexes",
            vec!["Name", "Columns", "Unique"],
            rows,
        ))
    }

    async fn list_views(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<ViewInfo>> {
        self.metadata(
            connection,
            "metadata.list_views",
            database_metadata_params(database, schema),
        )
        .await
    }

    async fn list_views_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        let rows = self
            .list_views(connection, database, None)
            .await?
            .into_iter()
            .map(|view| vec![view.name, view.comment.unwrap_or_default()])
            .collect();
        Ok(object_view(
            DbNodeType::View,
            "Views",
            vec!["Name", "Comment"],
            rows,
        ))
    }

    async fn list_foreign_keys(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<ForeignKeyDefinition>> {
        Ok(self
            .optional_metadata(
                connection,
                "metadata.list_foreign_keys",
                table_metadata_params(database, schema, table),
            )
            .await?
            .unwrap_or_default())
    }

    async fn list_table_triggers(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<TriggerInfo>> {
        Ok(self
            .optional_metadata(
                connection,
                "metadata.list_table_triggers",
                table_metadata_params(database, schema, table),
            )
            .await?
            .unwrap_or_default())
    }

    async fn list_table_checks(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<CheckInfo>> {
        Ok(self
            .optional_metadata(
                connection,
                "metadata.list_table_checks",
                table_metadata_params(database, schema, table),
            )
            .await?
            .unwrap_or_default())
    }

    async fn list_functions(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>> {
        Ok(self
            .optional_metadata(
                connection,
                "metadata.list_functions",
                database_metadata_params(database, None),
            )
            .await?
            .unwrap_or_default())
    }

    async fn list_functions_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        let rows = self
            .list_functions(connection, database)
            .await?
            .into_iter()
            .map(|function| vec![function.name, function.return_type.unwrap_or_default()])
            .collect();
        Ok(object_view(
            DbNodeType::Function,
            "Functions",
            vec!["Name", "Return Type"],
            rows,
        ))
    }

    fn ui_manifest(&self) -> DatabaseUiManifest {
        self.registry
            .drivers()
            .first()
            .and_then(|driver| driver.ui.form.clone())
            .unwrap_or_default()
    }

    async fn list_procedures(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>> {
        Ok(self
            .optional_metadata(
                connection,
                "metadata.list_procedures",
                database_metadata_params(database, None),
            )
            .await?
            .unwrap_or_default())
    }

    async fn list_procedures_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        let rows = self
            .list_procedures(connection, database)
            .await?
            .into_iter()
            .map(|procedure| vec![procedure.name, procedure.parameters.join(", ")])
            .collect();
        Ok(object_view(
            DbNodeType::Procedure,
            "Procedures",
            vec!["Name", "Parameters"],
            rows,
        ))
    }

    async fn list_triggers(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<TriggerInfo>> {
        Ok(self
            .optional_metadata(
                connection,
                "metadata.list_triggers",
                database_metadata_params(database, None),
            )
            .await?
            .unwrap_or_default())
    }

    async fn list_triggers_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        let rows = self
            .list_triggers(connection, database)
            .await?
            .into_iter()
            .map(|trigger| vec![trigger.name, trigger.table_name, trigger.event])
            .collect();
        Ok(object_view(
            DbNodeType::Trigger,
            "Triggers",
            vec!["Name", "Table", "Event"],
            rows,
        ))
    }

    async fn list_sequences(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<SequenceInfo>> {
        Ok(self
            .optional_metadata(
                connection,
                "metadata.list_sequences",
                database_metadata_params(database, schema),
            )
            .await?
            .unwrap_or_default())
    }

    async fn list_sequences_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        let rows = self
            .list_sequences(connection, database, None)
            .await?
            .into_iter()
            .map(|sequence| {
                vec![
                    sequence.name,
                    sequence.increment.unwrap_or_default().to_string(),
                ]
            })
            .collect();
        Ok(object_view(
            DbNodeType::Sequence,
            "Sequences",
            vec!["Name", "Increment"],
            rows,
        ))
    }

    fn build_column_definition(&self, column: &ColumnInfo, include_name: bool) -> String {
        let nullable = if column.is_nullable { "" } else { " NOT NULL" };
        let default = column
            .default_value
            .as_ref()
            .map(|value| format!(" DEFAULT {value}"))
            .unwrap_or_default();
        let name = if include_name {
            format!("{} ", self.quote_identifier(&column.name))
        } else {
            String::new()
        };
        format!("{name}{}{nullable}{default}", column.data_type)
    }

    fn build_create_database_sql(
        &self,
        request: &crate::plugin::DatabaseOperationRequest,
    ) -> String {
        format!(
            "CREATE DATABASE {}",
            self.quote_identifier(&request.database_name)
        )
    }

    fn build_modify_database_sql(
        &self,
        request: &crate::plugin::DatabaseOperationRequest,
    ) -> String {
        format!(
            "ALTER DATABASE {}",
            self.quote_identifier(&request.database_name)
        )
    }

    fn build_drop_database_sql(&self, database_name: &str) -> String {
        format!("DROP DATABASE {}", self.quote_identifier(database_name))
    }

    fn build_limit_clause(&self) -> String {
        "LIMIT".to_string()
    }

    fn build_where_and_limit_clause(
        &self,
        request: &TableSaveRequest,
        original_data: &[String],
    ) -> (String, String) {
        (
            self.build_table_change_where_clause(request, original_data),
            String::new(),
        )
    }

    fn rename_table(&self, _database: &str, old_name: &str, new_name: &str) -> String {
        format!(
            "ALTER TABLE {} RENAME TO {}",
            self.quote_identifier(old_name),
            self.quote_identifier(new_name)
        )
    }

    fn build_column_def(&self, col: &ColumnDefinition) -> String {
        let nullable = if col.is_nullable { "" } else { " NOT NULL" };
        let default = col
            .default_value
            .as_ref()
            .map(|value| format!(" DEFAULT {value}"))
            .unwrap_or_default();
        format!(
            "{} {}{nullable}{default}",
            self.quote_identifier(&col.name),
            col.data_type
        )
    }

    fn build_create_table_sql(&self, design: &TableDesign) -> String {
        let columns = design
            .columns
            .iter()
            .map(|column| self.build_column_def(column))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "CREATE TABLE {} ({})",
            self.quote_identifier(&design.table_name),
            columns
        )
    }

    fn build_alter_table_sql(&self, _original: &TableDesign, _new: &TableDesign) -> String {
        String::new()
    }

    async fn import_data_with_progress(
        &self,
        _connection: &dyn DbConnection,
        _config: &ImportConfig,
        _data: &str,
        _file_name: &str,
        _progress_tx: Option<ImportProgressSender>,
    ) -> Result<ImportResult> {
        Err(anyhow!("external database import is not supported yet"))
    }

    async fn export_data_with_progress(
        &self,
        _connection: &dyn DbConnection,
        _config: &ExportConfig,
        _progress_tx: Option<ExportProgressSender>,
    ) -> Result<ExportResult> {
        Err(anyhow!("external database export is not supported yet"))
    }
}

fn decode_single_cell<T>(query: crate::executor::QueryResult) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let value = query
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(|cell| cell.as_deref())
        .ok_or_else(|| anyhow!("external metadata response is empty"))?;
    serde_json::from_str(value).map_err(Into::into)
}

fn names_to_databases(names: Vec<String>) -> Vec<DatabaseInfo> {
    names
        .into_iter()
        .map(|name| DatabaseInfo {
            name,
            charset: None,
            collation: None,
            size: None,
            table_count: None,
            comment: None,
        })
        .collect()
}

fn is_not_supported(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<DbError>()
        .is_some_and(|error| matches!(error, DbError::NotSupported(_)))
}

fn merge_capabilities(
    capabilities: impl IntoIterator<Item = DatabaseCapabilities>,
) -> DatabaseCapabilities {
    capabilities
        .into_iter()
        .fold(DatabaseUiCapabilities::default(), |mut merged, current| {
            merged.supports_schema |= current.supports_schema;
            merged.uses_schema_as_database |= current.uses_schema_as_database;
            merged.supports_sequences |= current.supports_sequences;
            merged.supports_functions |= current.supports_functions;
            merged.supports_procedures |= current.supports_procedures;
            merged.supports_triggers |= current.supports_triggers;
            merged.supports_table_engine |= current.supports_table_engine;
            merged.supports_table_charset |= current.supports_table_charset;
            merged.supports_table_collation |= current.supports_table_collation;
            merged.supports_auto_increment |= current.supports_auto_increment;
            merged.supports_tablespace |= current.supports_tablespace;
            merged.supports_unsigned |= current.supports_unsigned;
            merged.supports_enum_values |= current.supports_enum_values;
            merged.show_charset_in_column_detail |= current.show_charset_in_column_detail;
            merged.show_collation_in_column_detail |= current.show_collation_in_column_detail;
            for engine in current.table_engines {
                if !merged.table_engines.contains(&engine) {
                    merged.table_engines.push(engine);
                }
            }
            merged
        })
}

fn object_view(
    db_node_type: DbNodeType,
    title: impl Into<String>,
    columns: Vec<&'static str>,
    rows: Vec<Vec<String>>,
) -> ObjectView {
    ObjectView {
        db_node_type,
        title: title.into(),
        columns: columns
            .into_iter()
            .map(|name| gpui_component::table::Column::new(name, name))
            .collect(),
        rows,
    }
}
