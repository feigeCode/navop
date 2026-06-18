use crate::clickhouse::ClickHousePlugin;
use crate::connection::{DbConnection, DbError};
use crate::executor::SqlSource;
use crate::import_export::{
    ExportConfig, ExportProgressSender, ExportResult, ImportConfig, ImportProgressSender,
    ImportResult,
};
use crate::ipc::client::JsonRpcClient;
use crate::ipc::connection::{ExternalDbConnection, WIRE_PREFIX};
use crate::ipc::protocol::driver_config_value;
use crate::ipc::registry::{IpcDriverManifest, IpcDriverRegistry, LimitStyle};
use crate::mssql::MsSqlPlugin;
use crate::mysql::MySqlPlugin;
use crate::oracle::OraclePlugin;
use crate::plugin::{ConnectionLifecycle, DatabasePlugin, SqlCompletionInfo};
use crate::plugin_manifest::{DatabaseCapabilities, DatabaseUiManifest};
use crate::postgresql::PostgresPlugin;
use crate::sqlite::SqlitePlugin;
use crate::streaming_parser::StreamingSqlParser;
use crate::types::*;
use anyhow::Result;
use async_trait::async_trait;
use extension_protocol::{
    conn::ConnTestResult, ddl as wire_ddl, method as wire_method, schema as wire_schema,
};
use one_core::storage::{DatabaseType, DbConnectionConfig};
use sqlparser::dialect::{Dialect, GenericDialect};
use std::collections::HashMap;
use std::sync::Arc;

type RegistryReloader = dyn Fn() -> IpcDriverRegistry + Send + Sync;

#[derive(Clone)]
pub struct ExternalDatabasePlugin {
    driver: IpcDriverManifest,
    registry: Option<IpcDriverRegistry>,
    registry_reloader: Option<Arc<RegistryReloader>>,
}

impl ExternalDatabasePlugin {
    pub fn new() -> Self {
        Self::with_registry_reloader(
            IpcDriverRegistry::load_default(),
            Arc::new(IpcDriverRegistry::load_default),
        )
    }

    pub fn with_registry(registry: IpcDriverRegistry) -> Self {
        Self::with_registry_source(registry, None)
    }

    pub fn with_registry_reloader(
        registry: IpcDriverRegistry,
        registry_reloader: Arc<RegistryReloader>,
    ) -> Self {
        Self::with_registry_source(registry, Some(registry_reloader))
    }

    fn with_registry_source(
        registry: IpcDriverRegistry,
        registry_reloader: Option<Arc<RegistryReloader>>,
    ) -> Self {
        let driver = registry
            .find("duckdb")
            .unwrap_or_else(|| placeholder_driver_manifest("duckdb"));
        Self {
            driver,
            registry: Some(registry),
            registry_reloader,
        }
    }

    pub fn for_driver(driver: IpcDriverManifest) -> Self {
        Self {
            driver,
            registry: None,
            registry_reloader: None,
        }
    }

    fn driver_for_config(&self, config: &DbConnectionConfig) -> Result<IpcDriverManifest, DbError> {
        let driver_id = driver_id_for_config(config)?;
        if let Some(registry) = &self.registry {
            if let Some(driver) = registry.find(driver_id) {
                return Ok(driver);
            }
            if let Some(reloader) = &self.registry_reloader {
                if let Some(driver) = reloader().find(driver_id) {
                    return Ok(driver);
                }
            }
            return Err(DbError::connection(format!(
                "external driver '{}' not found",
                driver_id
            )));
        }
        if driver_id != self.driver.id {
            return Err(DbError::connection(format!(
                "external driver '{}' does not match plugin driver '{}'",
                driver_id, self.driver.id
            )));
        }
        Ok(self.driver.clone())
    }

    async fn test_connection_via_open(&self, config: DbConnectionConfig) -> Result<(), DbError> {
        let mut conn = self.create_connection(config).await?;
        let ping_result = conn.ping().await;
        let _ = conn.disconnect().await;
        ping_result
    }

    async fn test_connection_via_conn_test(
        &self,
        config: &DbConnectionConfig,
        driver: &IpcDriverManifest,
    ) -> Result<(), DbError> {
        let client = JsonRpcClient::start(driver).await?;
        let params = serde_json::json!({
            "driver_id": driver.id,
            "config": driver_config_value(config),
        });
        let result = client.request_value(wire_method::CONN_TEST, params).await;
        client.shutdown().await;

        conn_test_value_to_result(&driver.id, result?)
    }

    fn build_column_change_sql(
        &self,
        table: &str,
        original: &ColumnDefinition,
        new: &ColumnDefinition,
    ) -> Vec<String> {
        let column = self.quote_identifier(&new.name);
        let mut statements = Vec::new();
        if original.data_type.to_uppercase() != new.data_type.to_uppercase()
            || original.length != new.length
            || original.precision != new.precision
            || original.scale != new.scale
        {
            statements.push(format!(
                "ALTER TABLE {table} ALTER COLUMN {column} TYPE {};",
                column_type_string(new)
            ));
        }
        if original.is_nullable != new.is_nullable {
            let action = if new.is_nullable {
                "DROP NOT NULL"
            } else {
                "SET NOT NULL"
            };
            statements.push(format!(
                "ALTER TABLE {table} ALTER COLUMN {column} {action};"
            ));
        }
        if original.default_value != new.default_value {
            match &new.default_value {
                Some(default) => statements.push(format!(
                    "ALTER TABLE {table} ALTER COLUMN {column} SET DEFAULT {default};"
                )),
                None => statements.push(format!(
                    "ALTER TABLE {table} ALTER COLUMN {column} DROP DEFAULT;"
                )),
            }
        }
        statements
    }

    fn index_changed(original: &IndexDefinition, new: &IndexDefinition) -> bool {
        original.columns != new.columns
            || original.is_unique != new.is_unique
            || original.index_type != new.index_type
    }

    fn build_index_sql(&self, table: &str, index: &IndexDefinition) -> Option<String> {
        if index.is_primary || index.columns.is_empty() {
            return None;
        }

        let columns = index
            .columns
            .iter()
            .map(|column| self.quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let unique = if index.is_unique { "UNIQUE " } else { "" };

        Some(format!(
            "CREATE {unique}INDEX {} ON {table} ({columns});",
            self.quote_identifier(&index.name)
        ))
    }

    fn build_index_change_sql(
        &self,
        table: &str,
        original: &TableDesign,
        new: &TableDesign,
    ) -> (Vec<String>, Vec<String>) {
        let original_indexes: HashMap<&str, &IndexDefinition> = original
            .indexes
            .iter()
            .map(|index| (index.name.as_str(), index))
            .collect();
        let new_indexes: HashMap<&str, &IndexDefinition> = new
            .indexes
            .iter()
            .map(|index| (index.name.as_str(), index))
            .collect();

        let mut drops = Vec::new();
        let mut creates = Vec::new();

        for index in original.indexes.iter().filter(|index| !index.is_primary) {
            let should_drop = new_indexes
                .get(index.name.as_str())
                .is_none_or(|new_index| Self::index_changed(index, new_index));
            if should_drop {
                drops.push(format!(
                    "DROP INDEX IF EXISTS {};",
                    self.quote_identifier(&index.name)
                ));
            }
        }

        for index in new.indexes.iter().filter(|index| !index.is_primary) {
            let should_create = original_indexes
                .get(index.name.as_str())
                .is_none_or(|original_index| Self::index_changed(original_index, index));
            if should_create {
                if let Some(sql) = self.build_index_sql(table, index) {
                    creates.push(sql);
                }
            }
        }

        (drops, creates)
    }

    fn build_external_explain_statement(&self, statement: &str) -> Option<String> {
        let statement = statement.trim();
        if statement.is_empty() {
            return None;
        }
        if statement.starts_with(WIRE_PREFIX) || self.is_explain_statement(statement) {
            return Some(statement.to_string());
        }
        if !self.is_query_statement(statement) {
            return None;
        }
        Some(wire_explain_sql(
            statement,
            self.driver.dialect.format_explain_sql(statement),
        ))
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
        let value = connection.driver_request_value(method, params).await?;
        serde_json::from_value(value).map_err(Into::into)
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

    fn compatible_plugin(&self) -> Option<Box<dyn DatabasePlugin>> {
        compatible_plugin_for(self.driver.dialect.compatible_database_type.clone()?)
    }
}

fn driver_id_for_config(config: &DbConnectionConfig) -> Result<&str, DbError> {
    if let Some(driver_id) = config.database_type.external_driver_id() {
        return Ok(driver_id);
    }
    match config.database_type {
        DatabaseType::DuckDB => Ok("duckdb"),
        _ => Err(DbError::connection(format!(
            "external driver id is required for {:?}",
            config.database_type
        ))),
    }
}

fn placeholder_driver_manifest(driver_id: &str) -> IpcDriverManifest {
    IpcDriverManifest {
        id: driver_id.to_string(),
        name: driver_id.to_string(),
        category: None,
        description: String::new(),
        version: String::new(),
        entry: crate::ipc::registry::IpcDriverEntry {
            command: String::new(),
            args: Vec::new(),
            working_dir: None,
        },
        transport: crate::ipc::registry::IpcDriverTransport::local_socket(format!(
            "{driver_id}.sock"
        )),
        dialect: Default::default(),
        capabilities: None,
        connection: Default::default(),
        methods: Vec::new(),
        ui: Default::default(),
        manifest_dir: Default::default(),
    }
}

fn compatible_plugin_for(database_type: DatabaseType) -> Option<Box<dyn DatabasePlugin>> {
    match database_type {
        DatabaseType::MySQL => Some(Box::new(MySqlPlugin::new())),
        DatabaseType::PostgreSQL => Some(Box::new(PostgresPlugin::new())),
        DatabaseType::SQLite => Some(Box::new(SqlitePlugin::new())),
        DatabaseType::DuckDB => None,
        DatabaseType::MSSQL => Some(Box::new(MsSqlPlugin::new())),
        DatabaseType::Oracle => Some(Box::new(OraclePlugin::new())),
        DatabaseType::ClickHouse => Some(Box::new(ClickHousePlugin::new())),
        DatabaseType::External { .. } => None,
    }
}

fn conn_test_value_to_result(driver_id: &str, value: serde_json::Value) -> Result<(), DbError> {
    let result: ConnTestResult = serde_json::from_value(value).map_err(|error| {
        DbError::query_with_source("invalid external driver conn/test response", error)
    })?;
    if result.ok {
        return Ok(());
    }

    let mut message = format!("external driver `{driver_id}` reported conn/test ok=false");
    if !result.warnings.is_empty() {
        message.push_str(": ");
        message.push_str(&result.warnings.join("; "));
    }
    Err(DbError::connection(message))
}

impl Default for ExternalDatabasePlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DatabasePlugin for ExternalDatabasePlugin {
    fn name(&self) -> DatabaseType {
        DatabaseType::external(self.driver.id.clone())
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        let (left, right) = self.driver.dialect.identifier_quote_pair();
        quote_identifier_with(left, right, identifier)
    }

    fn get_completion_info(&self) -> SqlCompletionInfo {
        SqlCompletionInfo::default().with_standard_sql()
    }

    async fn create_connection(
        &self,
        config: DbConnectionConfig,
    ) -> Result<Box<dyn DbConnection + Send + Sync>, DbError> {
        let driver = self.driver_for_config(&config)?;
        let mut conn = ExternalDbConnection::new(config, driver);
        conn.connect().await?;
        Ok(Box::new(conn))
    }

    fn connection_lifecycle(&self, config: &DbConnectionConfig) -> ConnectionLifecycle {
        let Ok(driver) = self.driver_for_config(config) else {
            return ConnectionLifecycle::default();
        };

        let close_on_release = driver.connection.close_on_release;
        let physical_open_lock_key =
            if driver.connection.single_file && driver.connection.single_connection {
                ConnectionLifecycle::single_file(&driver.id, config, &driver.connection.path_fields)
                    .physical_open_lock_key
            } else {
                None
            };

        ConnectionLifecycle {
            close_on_release,
            physical_open_lock_key,
        }
    }

    async fn test_connection(&self, config: DbConnectionConfig) -> Result<(), DbError> {
        let driver = self.driver_for_config(&config)?;
        let declares_conn_test = driver.methods.iter().any(|m| m == wire_method::CONN_TEST);
        if !driver.methods.is_empty() && !declares_conn_test {
            return self.test_connection_via_open(config).await;
        }

        match self.test_connection_via_conn_test(&config, &driver).await {
            Ok(()) => Ok(()),
            Err(DbError::NotSupported(_)) => self.test_connection_via_open(config).await,
            Err(error) => Err(error),
        }
    }

    async fn list_databases(&self, connection: &dyn DbConnection) -> Result<Vec<String>> {
        let infos: Vec<wire_schema::DatabaseInfo> = self
            .metadata(
                connection,
                wire_method::SCHEMA_DATABASES,
                serde_json::json!({}),
            )
            .await?;
        Ok(infos.into_iter().map(|database| database.name).collect())
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
                wire_method::SCHEMA_DATABASES,
                serde_json::json!({}),
            )
            .await
        {
            Ok(databases) => {
                let databases: Vec<wire_schema::DatabaseInfo> = databases;
                Ok(databases.into_iter().map(database_info_from_wire).collect())
            }
            Err(error) if is_not_supported(&error) => {
                Ok(names_to_databases(self.list_databases(connection).await?))
            }
            Err(error) => Err(error),
        }
    }

    fn capabilities(&self) -> DatabaseCapabilities {
        self.driver.effective_capabilities()
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
            wire_method::SCHEMA_SCHEMAS,
            serde_json::json!({ "database": database }),
        )
        .await
        .map(|schemas: Vec<wire_schema::SchemaInfo>| {
            schemas.into_iter().map(|schema| schema.name).collect()
        })
    }

    async fn list_tables(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<TableInfo>> {
        self.metadata(
            connection,
            wire_method::SCHEMA_OBJECTS,
            serde_json::json!({
                "database": database,
                "schema": schema,
                "kinds": ["table"],
            }),
        )
        .await
        .map(|tables: Vec<wire_schema::ObjectInfo>| {
            tables.into_iter().map(table_info_from_wire).collect()
        })
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
            wire_method::SCHEMA_COLUMNS,
            serde_json::json!({
                "database": database,
                "schema": schema,
                "table": table,
            }),
        )
        .await
        .map(|columns: Vec<wire_schema::ColumnInfo>| {
            columns.into_iter().map(column_info_from_wire).collect()
        })
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
            wire_method::SCHEMA_INDEXES,
            serde_json::json!({
                "database": database,
                "schema": schema,
                "table": table,
            }),
        )
        .await
        .map(|indexes: Vec<wire_schema::IndexInfo>| {
            indexes.into_iter().map(index_info_from_wire).collect()
        })
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
            wire_method::SCHEMA_VIEWS,
            serde_json::json!({
                "database": database,
                "schema": schema,
            }),
        )
        .await
        .map(|views: Vec<wire_schema::ViewInfo>| {
            views.into_iter().map(view_info_from_wire).collect()
        })
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
        let keys: Vec<wire_schema::ForeignKeyInfo> = self
            .optional_metadata(
                connection,
                wire_method::SCHEMA_FOREIGN_KEYS,
                serde_json::json!({
                    "database": database,
                    "schema": schema,
                    "table": table,
                }),
            )
            .await?
            .unwrap_or_default();
        Ok(keys.into_iter().map(foreign_key_from_wire).collect())
    }

    async fn list_table_triggers(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<TriggerInfo>> {
        let triggers: Vec<wire_schema::TriggerInfo> = self
            .optional_metadata(
                connection,
                wire_method::SCHEMA_TRIGGERS,
                serde_json::json!({
                    "database": database,
                    "schema": schema,
                    "table": table,
                }),
            )
            .await?
            .unwrap_or_default();
        Ok(triggers.into_iter().map(trigger_info_from_wire).collect())
    }

    async fn list_table_checks(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<CheckInfo>> {
        let checks: Vec<wire_schema::CheckInfo> = self
            .optional_metadata(
                connection,
                wire_method::SCHEMA_CHECKS,
                serde_json::json!({
                    "database": database,
                    "schema": schema,
                    "table": table,
                }),
            )
            .await?
            .unwrap_or_default();
        Ok(checks.into_iter().map(check_info_from_wire).collect())
    }

    async fn list_functions(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>> {
        let functions: Vec<wire_schema::FunctionInfo> = self
            .optional_metadata(
                connection,
                wire_method::SCHEMA_FUNCTIONS,
                serde_json::json!({ "database": database }),
            )
            .await?
            .unwrap_or_default();
        Ok(functions.into_iter().map(function_info_from_wire).collect())
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
        self.driver.ui.form.clone().unwrap_or_default()
    }

    async fn list_procedures(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>> {
        let procedures: Vec<wire_schema::ProcedureInfo> = self
            .optional_metadata(
                connection,
                wire_method::SCHEMA_PROCEDURES,
                serde_json::json!({ "database": database }),
            )
            .await?
            .unwrap_or_default();
        Ok(procedures
            .into_iter()
            .map(function_info_from_wire)
            .collect())
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
        let triggers: Vec<wire_schema::TriggerInfo> = self
            .optional_metadata(
                connection,
                wire_method::SCHEMA_TRIGGERS,
                serde_json::json!({ "database": database }),
            )
            .await?
            .unwrap_or_default();
        Ok(triggers.into_iter().map(trigger_info_from_wire).collect())
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
        let sequences: Vec<wire_schema::SequenceInfo> = self
            .optional_metadata(
                connection,
                wire_method::SCHEMA_SEQUENCES,
                serde_json::json!({
                    "database": database,
                    "schema": schema,
                }),
            )
            .await?
            .unwrap_or_default();
        Ok(sequences.into_iter().map(sequence_info_from_wire).collect())
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
        match self.driver.dialect.limit_style {
            LimitStyle::LimitOffset => "LIMIT".to_string(),
            LimitStyle::OffsetFetch => String::new(),
        }
    }

    fn format_pagination(&self, limit: usize, offset: usize, order_clause: &str) -> String {
        match self.driver.dialect.limit_style {
            LimitStyle::LimitOffset => format!(" LIMIT {limit} OFFSET {offset}"),
            LimitStyle::OffsetFetch => {
                if order_clause.is_empty() {
                    format!(
                        " ORDER BY (SELECT NULL) OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY"
                    )
                } else {
                    format!(" OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY")
                }
            }
        }
    }

    fn format_boolean_value(&self, v: &str) -> String {
        if v == "1" || v.eq_ignore_ascii_case("true") {
            self.driver.dialect.bool_true.clone()
        } else {
            self.driver.dialect.bool_false.clone()
        }
    }

    fn build_explain_statement(&self, sql: &str) -> String {
        self.driver
            .dialect
            .format_explain_sql(sql)
            .unwrap_or_default()
    }

    fn build_explain_sql(&self, sql: &str) -> Option<String> {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            return None;
        }

        let statements = self
            .split_sql_statements(trimmed)
            .into_iter()
            .filter_map(|statement| self.build_external_explain_statement(&statement))
            .collect::<Vec<_>>();
        if statements.is_empty() {
            None
        } else {
            Some(statements.join("\n"))
        }
    }

    fn split_sql_statements(&self, sql: &str) -> Vec<String> {
        let trimmed = sql.trim();
        if trimmed.starts_with(WIRE_PREFIX) {
            return split_wire_script(trimmed);
        }
        split_sql_with_parser(trimmed, self.name())
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

    fn build_alter_table_sql(&self, original: &TableDesign, new: &TableDesign) -> String {
        let table = self.quote_identifier(&new.table_name);
        let original_cols: HashMap<&str, &ColumnDefinition> = original
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column))
            .collect();
        let new_cols: HashMap<&str, &ColumnDefinition> = new
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column))
            .collect();
        let mut statements = Vec::new();
        let (index_drops, index_creates) = self.build_index_change_sql(&table, original, new);

        statements.extend(index_drops);
        for column in &new.columns {
            if !original_cols.contains_key(column.name.as_str()) {
                statements.push(format!(
                    "ALTER TABLE {table} ADD COLUMN {};",
                    self.build_column_def(column)
                ));
            }
        }
        for column in &original.columns {
            if !new_cols.contains_key(column.name.as_str()) {
                statements.push(format!(
                    "ALTER TABLE {table} DROP COLUMN {};",
                    self.quote_identifier(&column.name)
                ));
            }
        }
        for column in &new.columns {
            if let Some(original_col) = original_cols.get(column.name.as_str()) {
                statements.extend(self.build_column_change_sql(&table, original_col, column));
            }
        }
        statements.extend(index_creates);

        if statements.is_empty() {
            "-- No changes detected".to_string()
        } else {
            statements.join("\n")
        }
    }

    async fn build_create_table_sql_async(
        &self,
        connection: &dyn DbConnection,
        design: &TableDesign,
    ) -> Result<String> {
        let params = wire_ddl::BuildCreateTableParams {
            conn_id: None,
            spec: table_spec_from_design(design),
            options: wire_ddl::CreateTableOptions::default(),
        };
        let value = serde_json::to_value(params)?;
        match self
            .metadata::<wire_ddl::BuildCreateTableResult>(
                connection,
                wire_method::DDL_BUILD_CREATE_TABLE,
                value,
            )
            .await
        {
            Ok(result) => Ok(join_ddl_statements(result.statements, Some(result.sql))),
            Err(error) if is_not_supported(&error) => Ok(self
                .compatible_plugin()
                .map(|plugin| plugin.build_create_table_sql(design))
                .unwrap_or_else(|| self.build_create_table_sql(design))),
            Err(error) => Err(error),
        }
    }

    async fn build_alter_table_sql_with_renames_async(
        &self,
        connection: &dyn DbConnection,
        original: &TableDesign,
        new: &TableDesign,
        column_renames: &[(String, String)],
    ) -> Result<String> {
        let params = wire_ddl::BuildAlterTableParams {
            conn_id: None,
            from_spec: table_spec_from_design(original),
            to_spec: table_spec_from_design(new),
            column_renames: column_renames
                .iter()
                .map(|(old_name, new_name)| wire_ddl::ColumnRenameSpec {
                    old_name: old_name.clone(),
                    new_name: new_name.clone(),
                })
                .collect(),
            options: wire_ddl::AlterTableOptions {
                allow_destructive: true,
                with_rollback: false,
            },
        };
        let value = serde_json::to_value(params)?;
        match self
            .metadata::<wire_ddl::BuildAlterTableResult>(
                connection,
                wire_method::DDL_BUILD_ALTER_TABLE,
                value,
            )
            .await
        {
            Ok(result) => Ok(join_ddl_statements(result.statements, None)),
            Err(error) if is_not_supported(&error) => Ok(self
                .compatible_plugin()
                .map(|plugin| {
                    plugin.build_alter_table_sql_with_renames(original, new, column_renames)
                })
                .unwrap_or_else(|| {
                    self.build_alter_table_sql_with_renames(original, new, column_renames)
                })),
            Err(error) => Err(error),
        }
    }

    async fn import_data_with_progress(
        &self,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
        file_name: &str,
        progress_tx: Option<ImportProgressSender>,
    ) -> Result<ImportResult> {
        crate::ipc::import::import_data_with_progress(
            self,
            connection,
            config,
            data,
            file_name,
            progress_tx,
        )
        .await
    }

    async fn export_data_with_progress(
        &self,
        connection: &dyn DbConnection,
        config: &ExportConfig,
        progress_tx: Option<ExportProgressSender>,
    ) -> Result<ExportResult> {
        crate::ipc::export::export_data_with_progress(self, connection, config, progress_tx).await
    }
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

fn table_spec_from_design(design: &TableDesign) -> wire_ddl::TableSpec {
    wire_ddl::TableSpec {
        name: design.table_name.clone(),
        schema: None,
        database: Some(design.database_name.clone()).filter(|database| !database.is_empty()),
        columns: design
            .columns
            .iter()
            .map(column_spec_from_definition)
            .collect(),
        primary_key: design
            .primary_key_columns()
            .into_iter()
            .map(str::to_string)
            .collect(),
        indexes: design
            .indexes
            .iter()
            .filter(|index| !index.is_primary)
            .map(index_spec_from_definition)
            .collect(),
        foreign_keys: design
            .foreign_keys
            .iter()
            .map(foreign_key_spec_from_definition)
            .collect(),
        comment: design.options.comment.clone(),
        options: table_options_value(&design.options),
    }
}

fn column_spec_from_definition(column: &ColumnDefinition) -> wire_ddl::ColumnSpec {
    wire_ddl::ColumnSpec {
        name: column.name.clone(),
        type_str: column_type_string(column),
        nullable: column.is_nullable,
        default: column.default_value.clone(),
        is_primary: column.is_primary_key,
        is_unique: false,
        auto_increment: column.is_auto_increment,
        comment: column.comment.clone(),
        extra: serde_json::json!({
            "unsigned": column.is_unsigned,
            "charset": column.charset,
            "collation": column.collation,
        }),
    }
}

fn index_spec_from_definition(index: &IndexDefinition) -> wire_ddl::IndexSpec {
    wire_ddl::IndexSpec {
        name: index.name.clone(),
        columns: index.columns.clone(),
        kind: index.index_type.clone(),
        is_unique: index.is_unique,
        where_clause: None,
    }
}

fn foreign_key_spec_from_definition(
    foreign_key: &ForeignKeyDefinition,
) -> wire_ddl::ForeignKeySpec {
    wire_ddl::ForeignKeySpec {
        name: foreign_key.name.clone(),
        from_columns: foreign_key.columns.clone(),
        to_table: foreign_key.ref_table.clone(),
        to_columns: foreign_key.ref_columns.clone(),
        on_delete: empty_to_none(foreign_key.on_delete.clone()),
        on_update: empty_to_none(foreign_key.on_update.clone()),
    }
}

fn table_options_value(options: &TableOptions) -> serde_json::Value {
    serde_json::json!({
        "engine": options.engine,
        "charset": options.charset,
        "collation": options.collation,
        "auto_increment": options.auto_increment,
    })
}

fn column_type_string(column: &ColumnDefinition) -> String {
    let mut type_str = column.data_type.clone();
    if let Some(precision) = column.precision {
        if let Some(scale) = column.scale {
            type_str = format!("{}({},{})", type_str, precision, scale);
        } else {
            type_str = format!("{}({})", type_str, precision);
        }
    } else if let Some(length) = column.length {
        type_str = format!("{}({})", type_str, length);
    }
    type_str
}

fn join_ddl_statements(statements: Vec<String>, fallback_sql: Option<String>) -> String {
    let mut statements: Vec<String> = statements
        .into_iter()
        .filter(|statement| !statement.trim().is_empty())
        .collect();
    if statements.is_empty() {
        if let Some(sql) = fallback_sql.filter(|sql| !sql.trim().is_empty()) {
            statements.push(sql);
        }
    }
    if statements.is_empty() {
        return "-- No changes detected".to_string();
    }
    statements
        .into_iter()
        .map(ensure_statement_terminated)
        .collect::<Vec<_>>()
        .join("\n")
}

fn ensure_statement_terminated(statement: String) -> String {
    let trimmed = statement.trim();
    if trimmed.ends_with(';') {
        trimmed.to_string()
    } else {
        format!("{trimmed};")
    }
}

fn database_info_from_wire(database: wire_schema::DatabaseInfo) -> DatabaseInfo {
    DatabaseInfo {
        name: database.name,
        charset: database.charset,
        collation: database.collation,
        size: database.size_bytes.map(|size| size.to_string()),
        table_count: None,
        comment: empty_to_none(database.comment),
    }
}

fn table_info_from_wire(object: wire_schema::ObjectInfo) -> TableInfo {
    TableInfo {
        name: object.name,
        schema: None,
        comment: empty_to_none(object.comment),
        engine: None,
        row_count: object.row_count_estimate.map(|count| count as i64),
        create_time: object.created_at,
        charset: None,
        collation: None,
    }
}

fn column_info_from_wire(column: wire_schema::ColumnInfo) -> ColumnInfo {
    ColumnInfo {
        name: column.name,
        data_type: column.raw_type.unwrap_or(column.type_str),
        is_nullable: column.nullable,
        is_primary_key: column.is_primary,
        default_value: column.default,
        comment: empty_to_none(column.comment),
        charset: None,
        collation: None,
    }
}

fn index_info_from_wire(index: wire_schema::IndexInfo) -> IndexInfo {
    IndexInfo {
        name: index.name,
        columns: index.columns,
        is_unique: index.is_unique,
        index_type: index.kind,
    }
}

fn view_info_from_wire(view: wire_schema::ViewInfo) -> ViewInfo {
    ViewInfo {
        name: view.name,
        schema: None,
        definition: Some(view.definition_sql).filter(|definition| !definition.is_empty()),
        comment: empty_to_none(view.comment),
    }
}

fn foreign_key_from_wire(foreign_key: wire_schema::ForeignKeyInfo) -> ForeignKeyDefinition {
    ForeignKeyDefinition {
        name: foreign_key.name,
        columns: foreign_key.from_columns,
        ref_table: foreign_key.to_table,
        ref_columns: foreign_key.to_columns,
        on_delete: foreign_key.on_delete.unwrap_or_default(),
        on_update: foreign_key.on_update.unwrap_or_default(),
    }
}

fn check_info_from_wire(check: wire_schema::CheckInfo) -> CheckInfo {
    CheckInfo {
        name: check.name,
        table_name: check.table,
        definition: check.definition,
    }
}

fn function_info_from_wire(function: wire_schema::FunctionInfo) -> FunctionInfo {
    FunctionInfo {
        name: function.name,
        return_type: function.return_type,
        parameters: function
            .args
            .into_iter()
            .map(|arg| {
                let name = arg.name;
                let type_str = arg.type_str;
                if name.is_empty() {
                    type_str
                } else if type_str.is_empty() {
                    name
                } else {
                    format!("{name} {type_str}")
                }
            })
            .collect(),
        definition: function.definition,
        comment: empty_to_none(function.comment),
    }
}

fn trigger_info_from_wire(trigger: wire_schema::TriggerInfo) -> TriggerInfo {
    TriggerInfo {
        name: trigger.name,
        table_name: trigger.table,
        event: trigger.event,
        timing: trigger.timing,
        definition: trigger.definition,
    }
}

fn sequence_info_from_wire(sequence: wire_schema::SequenceInfo) -> SequenceInfo {
    SequenceInfo {
        name: sequence.name,
        start_value: sequence.start_value,
        increment: sequence.increment,
        min_value: sequence.min_value,
        max_value: sequence.max_value,
    }
}

fn empty_to_none(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

fn is_not_supported(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<DbError>()
        .is_some_and(|error| matches!(error, DbError::NotSupported(_)))
}

fn quote_identifier_with(left: &str, right: &str, identifier: &str) -> String {
    if left.is_empty() && right.is_empty() {
        return identifier.to_string();
    }
    let escaped = if right.is_empty() {
        identifier.to_string()
    } else {
        identifier.replace(right, &format!("{right}{right}"))
    };
    format!("{left}{escaped}{right}")
}

fn wire_explain_sql(sql: &str, fallback_sql: Option<String>) -> String {
    let mut params = serde_json::json!({ "sql": sql });
    if let Some(fallback_sql) = fallback_sql {
        params["fallback_sql"] = serde_json::json!(fallback_sql);
    }
    wire_request_sql(wire_method::SQL_EXPLAIN, params)
}

fn wire_request_sql(method: &str, params: serde_json::Value) -> String {
    format!(
        "{WIRE_PREFIX}{}",
        serde_json::json!({ "method": method, "params": params })
    )
}

fn split_wire_script(sql: &str) -> Vec<String> {
    sql.lines()
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .map(|statement| statement.trim_end_matches(';').trim().to_string())
        .collect()
}

fn split_sql_with_parser(sql: &str, database_type: DatabaseType) -> Vec<String> {
    if sql.is_empty() {
        return Vec::new();
    }
    let Ok(parser) =
        StreamingSqlParser::from_source(SqlSource::Script(sql.to_string()), database_type)
    else {
        return vec![sql.to_string()];
    };
    parser
        .filter_map(Result::ok)
        .map(|sql| sql.trim().to_string())
        .filter(|sql| !sql.is_empty())
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::StreamingProgress;
    use crate::executor::{ExecOptions, SqlResult, SqlSource};
    use crate::ipc::connection::WIRE_PREFIX;
    use std::path::PathBuf;
    use tokio::sync::mpsc;

    struct DriverRequestOnlyConnection {
        config: DbConnectionConfig,
        supports_alter_table_builder: bool,
    }

    impl DriverRequestOnlyConnection {
        fn new() -> Self {
            Self {
                config: DbConnectionConfig {
                    id: "driver-request-only".into(),
                    name: "Driver Request Only".into(),
                    database_type: DatabaseType::external("driver-request-only"),
                    host: String::new(),
                    port: 0,
                    username: String::new(),
                    password: String::new(),
                    database: None,
                    service_name: None,
                    sid: None,
                    workspace_id: None,
                    extra_params: Default::default(),
                },
                supports_alter_table_builder: true,
            }
        }

        fn without_alter_table_builder() -> Self {
            Self {
                supports_alter_table_builder: false,
                ..Self::new()
            }
        }
    }

    #[async_trait]
    impl DbConnection for DriverRequestOnlyConnection {
        fn config(&self) -> &DbConnectionConfig {
            &self.config
        }

        fn set_config_database(&mut self, database: Option<String>) {
            self.config.database = database;
        }

        async fn connect(&mut self) -> Result<(), DbError> {
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<(), DbError> {
            Ok(())
        }

        async fn execute(
            &self,
            _plugin: &dyn DatabasePlugin,
            _script: &str,
            _options: ExecOptions,
        ) -> Result<Vec<SqlResult>, DbError> {
            Err(DbError::query("execute should not be used by metadata"))
        }

        async fn query(&self, _query: &str) -> Result<SqlResult, DbError> {
            Err(DbError::query("query should not be used by metadata"))
        }

        async fn driver_request_value(
            &self,
            method: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, DbError> {
            match method {
                wire_method::SCHEMA_DATABASES => Ok(serde_json::json!([{
                    "name": "mockdb",
                    "comment": "",
                    "extra": null
                }])),
                wire_method::SCHEMA_FUNCTIONS => Ok(serde_json::json!([{
                    "name": "lower",
                    "return_type": "VARCHAR",
                    "args": [{"name": "value", "type": "VARCHAR"}],
                    "definition": "lower(value)",
                    "comment": "",
                    "extra": null
                }])),
                wire_method::SCHEMA_CHECKS => Ok(serde_json::json!([{
                    "name": "events_payload_check",
                    "table": "events",
                    "definition": "payload IS NOT NULL",
                    "comment": "",
                    "extra": null
                }])),
                wire_method::DDL_BUILD_ALTER_TABLE if self.supports_alter_table_builder => {
                    Ok(serde_json::json!({
                        "statements": ["DRIVER RENAME SQL"],
                        "rollback_statements": [],
                        "warnings": []
                    }))
                }
                other => Err(DbError::NotSupported(other.to_string())),
            }
        }

        async fn current_database(&self) -> Result<Option<String>, DbError> {
            Ok(None)
        }

        async fn switch_database(&self, _database: &str) -> Result<(), DbError> {
            Ok(())
        }

        async fn execute_streaming(
            &self,
            _plugin: &dyn DatabasePlugin,
            _source: SqlSource,
            _options: ExecOptions,
            _sender: mpsc::Sender<StreamingProgress>,
        ) -> Result<(), DbError> {
            Ok(())
        }
    }

    fn driver_manifest(id: &str, supports_schema: bool, form_title: &str) -> IpcDriverManifest {
        let mut driver: IpcDriverManifest = serde_json::from_str(&format!(
            r#"{{
                "id":"{id}",
                "name":"{id}",
                "entry":{{"command":"driver"}},
                "transport":{{"name":"{id}.sock"}},
                "capabilities":{{"supports_schema":{supports_schema}}},
                "ui":{{
                    "form":{{
                        "schema_version":1,
                        "forms":[{{
                            "kind":"Connection",
                            "title_i18n_key":"{form_title}",
                            "submit_i18n_key":"submit",
                            "tabs":[]
                        }}],
                        "actions":{{"actions":[]}}
                    }}
                }}
            }}"#
        ))
        .unwrap();
        driver.manifest_dir = PathBuf::from(format!("/drivers/{id}"));
        driver
    }

    #[test]
    fn fixed_driver_plugin_uses_that_driver_capabilities_ui_and_quote() {
        let alpha = driver_manifest("alpha", true, "alpha.connection");
        let beta = driver_manifest("beta", false, "beta.connection");

        let plugin = ExternalDatabasePlugin::for_driver(beta.clone());

        assert_eq!("\"has\"\"quote\"", plugin.quote_identifier("has\"quote"));
        assert!(!plugin.capabilities().supports_schema);
        assert_eq!(
            "beta.connection",
            plugin.ui_manifest().forms[0].title_i18n_key
        );
        assert_ne!(
            alpha.ui.form.unwrap().forms[0].title_i18n_key,
            plugin.ui_manifest().forms[0].title_i18n_key
        );
    }

    #[test]
    fn fixed_driver_plugin_uses_manifest_connection_lifecycle() {
        let mut driver = driver_manifest("singlefile", false, "singlefile.connection");
        driver.connection.single_file = true;
        driver.connection.single_connection = true;
        driver.connection.close_on_release = true;
        driver.connection.path_fields = vec!["host".to_string(), "extra_params.path".to_string()];
        let plugin = ExternalDatabasePlugin::for_driver(driver);
        let mut config = DbConnectionConfig {
            id: "singlefile-conn".into(),
            name: "SingleFile".into(),
            database_type: DatabaseType::external("singlefile"),
            host: "file:/tmp/singlefile.db".into(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            extra_params: Default::default(),
        };

        let lifecycle = plugin.connection_lifecycle(&config);
        assert!(lifecycle.close_on_release);
        assert_eq!(
            Some("singlefile:/tmp/singlefile.db".to_string()),
            lifecycle.physical_open_lock_key
        );

        config.host.clear();
        config
            .extra_params
            .insert("path".to_string(), "/tmp/from-extra.db".to_string());
        let lifecycle = plugin.connection_lifecycle(&config);
        assert_eq!(
            Some("singlefile:/tmp/from-extra.db".to_string()),
            lifecycle.physical_open_lock_key
        );
    }

    #[test]
    fn reloading_registry_finds_driver_added_after_plugin_creation() {
        let driver = driver_manifest("duckdb", true, "duckdb.connection");
        let plugin = ExternalDatabasePlugin::with_registry_reloader(
            IpcDriverRegistry::empty(),
            std::sync::Arc::new(move || IpcDriverRegistry::from_drivers(vec![driver.clone()])),
        );
        let mut config = DbConnectionConfig {
            id: "duckdb-conn".into(),
            name: "DuckDB".into(),
            database_type: DatabaseType::DuckDB,
            host: "/tmp/on-demand.duckdb".into(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            extra_params: Default::default(),
        };

        let resolved = plugin
            .driver_for_config(&config)
            .expect("installed DuckDB driver should be discovered after registry reload");

        assert_eq!("duckdb", resolved.id);
        assert!(resolved.effective_capabilities().supports_schema);

        config.database_type = DatabaseType::external("missing");
        let error = plugin.driver_for_config(&config).unwrap_err();
        assert!(format!("{error}").contains("external driver 'missing' not found"));
    }

    #[test]
    fn driver_id_for_config_ignores_extra_params_driver_id() {
        let mut config = DbConnectionConfig {
            id: "conn-1".into(),
            name: "demo".into(),
            database_type: DatabaseType::MySQL,
            host: String::new(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            extra_params: Default::default(),
        };
        config
            .extra_params
            .insert("external_driver_id".to_string(), "iotdb".to_string());

        let error = driver_id_for_config(&config).unwrap_err();

        assert!(
            error.to_string().contains("external driver id is required"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn dialect_contract_drives_host_sql_fragments() {
        let mut driver = driver_manifest("mssql-ish", false, "mssql.connection");
        driver.dialect.identifier_quote_left = "[".to_string();
        driver.dialect.identifier_quote_right = Some("]".to_string());
        driver.dialect.limit_style = LimitStyle::OffsetFetch;
        driver.dialect.bool_true = "1".to_string();
        driver.dialect.bool_false = "0".to_string();

        let plugin = ExternalDatabasePlugin::for_driver(driver);

        assert_eq!("[has]]quote]", plugin.quote_identifier("has]quote"));
        assert_eq!(
            " OFFSET 20 ROWS FETCH NEXT 10 ROWS ONLY",
            plugin.format_pagination(10, 20, " ORDER BY id")
        );
        assert_eq!(
            " ORDER BY (SELECT NULL) OFFSET 0 ROWS FETCH NEXT 500 ROWS ONLY",
            plugin.format_pagination(500, 0, "")
        );
        assert_eq!("", plugin.build_limit_clause());
        assert_eq!("1", plugin.format_boolean_value("true"));
        assert_eq!("1", plugin.format_boolean_value("1"));
        assert_eq!("0", plugin.format_boolean_value("false"));
        assert_eq!("0", plugin.format_boolean_value("0"));
    }

    #[test]
    fn external_explain_sql_uses_wire_method_with_dialect_fallback() {
        let mut driver = driver_manifest("explainable", false, "explain.connection");
        driver.methods = vec![wire_method::SQL_EXPLAIN.to_string()];
        driver.dialect.explain_template = Some("EXPLAIN QUERY PLAN {sql}".to_string());
        let plugin = ExternalDatabasePlugin::for_driver(driver);

        let sql = plugin
            .build_explain_sql("select * from metrics")
            .expect("query should produce explain SQL");
        let envelope: serde_json::Value =
            serde_json::from_str(sql.strip_prefix(WIRE_PREFIX).unwrap()).unwrap();

        assert_eq!(Some(wire_method::SQL_EXPLAIN), envelope["method"].as_str());
        assert_eq!(
            Some("select * from metrics"),
            envelope["params"]["sql"].as_str()
        );
        assert_eq!(
            Some("EXPLAIN QUERY PLAN select * from metrics"),
            envelope["params"]["fallback_sql"].as_str()
        );
    }

    #[test]
    fn external_splitter_preserves_wire_explain_requests() {
        let mut driver = driver_manifest("explainable", false, "explain.connection");
        driver.methods = vec![wire_method::SQL_EXPLAIN.to_string()];
        let plugin = ExternalDatabasePlugin::for_driver(driver);
        let sql = plugin
            .build_explain_sql("select 1; select 2;")
            .expect("queries should produce explain SQL");

        let statements = plugin.split_sql_statements(&sql);

        assert_eq!(2, statements.len());
        assert!(
            statements
                .iter()
                .all(|statement| statement.starts_with(WIRE_PREFIX))
        );
    }

    #[test]
    fn external_splitter_keeps_original_sql_when_parser_errors() {
        let sql = "SELECT * FROM";

        assert_eq!(
            vec![sql.to_string()],
            split_sql_with_parser(sql, DatabaseType::external("demo"))
        );
    }

    #[tokio::test]
    async fn metadata_uses_driver_request_instead_of_query_tunnel() {
        let plugin = ExternalDatabasePlugin::new();
        let connection = DriverRequestOnlyConnection::new();

        let databases = plugin.list_databases(&connection).await.unwrap();

        assert_eq!(vec!["mockdb"], databases);
    }

    #[tokio::test]
    async fn optional_metadata_uses_schema_functions_method() {
        let plugin = ExternalDatabasePlugin::new();
        let connection = DriverRequestOnlyConnection::new();

        let functions = plugin.list_functions(&connection, "main").await.unwrap();

        assert_eq!(1, functions.len());
        assert_eq!("lower", functions[0].name);
        assert_eq!(Some("VARCHAR".to_string()), functions[0].return_type);
        assert_eq!(vec!["value VARCHAR"], functions[0].parameters);
    }

    #[tokio::test]
    async fn table_checks_use_schema_checks_method() {
        let plugin = ExternalDatabasePlugin::new();
        let connection = DriverRequestOnlyConnection::new();

        let checks = plugin
            .list_table_checks(&connection, "main", None, "events")
            .await
            .unwrap();

        assert_eq!(1, checks.len());
        assert_eq!("events_payload_check", checks[0].name);
        assert_eq!("events", checks[0].table_name);
        assert_eq!(
            Some("payload IS NOT NULL".to_string()),
            checks[0].definition
        );
    }

    #[test]
    fn conn_test_response_requires_ok_bool() {
        let error = conn_test_value_to_result("duckdb", serde_json::json!({})).unwrap_err();

        match error {
            DbError::Query { message, .. } => {
                assert!(message.contains("invalid external driver conn/test response"));
            }
            other => panic!("expected Query error, got {other:?}"),
        }
    }

    #[test]
    fn conn_test_response_false_is_connection_error() {
        let error = conn_test_value_to_result(
            "duckdb",
            serde_json::json!({
                "ok": false,
                "warnings": ["bad path"],
            }),
        )
        .unwrap_err();

        match error {
            DbError::Connection { message, .. } => {
                assert!(message.contains("duckdb"));
                assert!(message.contains("ok=false"));
            }
            other => panic!("expected Connection error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn async_alter_with_column_renames_calls_driver_ddl_method() {
        let plugin = ExternalDatabasePlugin::new();
        let connection = DriverRequestOnlyConnection::new();
        let mut original = TableDesign::new("main", "events");
        original.add_column(ColumnDefinition::new("payload").data_type("VARCHAR"));
        let mut current = TableDesign::new("main", "events");
        current.add_column(ColumnDefinition::new("body").data_type("VARCHAR"));

        let sql = plugin
            .build_alter_table_sql_with_renames_async(
                &connection,
                &original,
                &current,
                &[("payload".to_string(), "body".to_string())],
            )
            .await
            .unwrap();

        assert_eq!("DRIVER RENAME SQL;", sql);
    }

    #[tokio::test]
    async fn async_create_table_uses_compatible_database_fallback() {
        let mut driver = driver_manifest("postgres-compatible", false, "postgres.connection");
        driver.dialect.compatible_database_type = Some(DatabaseType::PostgreSQL);
        let plugin = ExternalDatabasePlugin::for_driver(driver);
        let mut connection = DriverRequestOnlyConnection::new();
        connection.config.database_type = DatabaseType::external("postgres-compatible");
        let mut design = TableDesign::new("main", "events");
        design.add_column(
            ColumnDefinition::new("id")
                .data_type("INTEGER")
                .nullable(false),
        );

        let sql = plugin
            .build_create_table_sql_async(&connection, &design)
            .await
            .unwrap();

        assert_eq!(
            "CREATE TABLE \"events\" (\n  \"id\" INTEGER NOT NULL\n);",
            sql
        );
    }

    #[tokio::test]
    async fn async_alter_table_uses_compatible_database_fallback() {
        let mut driver = driver_manifest("postgres-compatible", false, "postgres.connection");
        driver.dialect.compatible_database_type = Some(DatabaseType::PostgreSQL);
        let plugin = ExternalDatabasePlugin::for_driver(driver);
        let mut connection = DriverRequestOnlyConnection::without_alter_table_builder();
        connection.config.database_type = DatabaseType::external("postgres-compatible");
        let mut original = TableDesign::new("main", "events");
        original.add_column(ColumnDefinition::new("id").data_type("INTEGER"));
        let mut current = TableDesign::new("main", "events");
        current.add_column(ColumnDefinition::new("id").data_type("INTEGER"));
        current.add_column(ColumnDefinition::new("payload").data_type("TEXT"));

        let sql = plugin
            .build_alter_table_sql_with_renames_async(&connection, &original, &current, &[])
            .await
            .unwrap();

        assert_eq!("ALTER TABLE \"events\" ADD COLUMN \"payload\" TEXT;", sql);
    }

    #[test]
    fn sync_alter_table_builder_returns_local_fallback_without_ipc() {
        let plugin = ExternalDatabasePlugin::new();
        let mut original = TableDesign::new("main", "events");
        original.add_column(
            ColumnDefinition::new("id")
                .data_type("INTEGER")
                .nullable(false),
        );
        let mut current = original.clone();
        current.add_column(ColumnDefinition::new("payload").data_type("VARCHAR"));

        let sql = plugin.build_alter_table_sql(&original, &current);

        assert!(sql.contains("ALTER TABLE \"events\" ADD COLUMN \"payload\" VARCHAR"));
    }

    #[test]
    fn sync_alter_table_builder_includes_index_changes() {
        let plugin = ExternalDatabasePlugin::new();
        let mut original = TableDesign::new("main", "events");
        original.add_column(ColumnDefinition::new("id").data_type("INTEGER"));
        original.add_column(ColumnDefinition::new("payload").data_type("VARCHAR"));
        original.add_index(IndexDefinition::new("idx_payload").columns(vec!["payload".into()]));

        let mut current = TableDesign::new("main", "events");
        current.add_column(ColumnDefinition::new("id").data_type("INTEGER"));
        current.add_column(ColumnDefinition::new("payload").data_type("VARCHAR"));
        current.add_index(
            IndexDefinition::new("idx_id")
                .columns(vec!["id".into()])
                .unique(true),
        );

        let sql = plugin.build_alter_table_sql(&original, &current);

        assert!(sql.contains("DROP INDEX IF EXISTS \"idx_payload\";"));
        assert!(sql.contains("CREATE UNIQUE INDEX \"idx_id\" ON \"events\" (\"id\");"));
    }
}
