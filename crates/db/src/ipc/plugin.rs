use crate::clickhouse::ClickHousePlugin;
use crate::connection::{DbConnection, DbError};
use crate::executor::{QueryResult, SqlResult, SqlSource};
use crate::import_export::{
    ExportConfig, ExportProgressSender, ExportResult, ImportConfig, ImportProgressSender,
    ImportResult,
};
use crate::ipc::client::JsonRpcClient;
use crate::ipc::connection::ExternalDbConnection;
use crate::ipc::protocol::{WIRE_PREFIX, driver_config_value_with_target, schema_users_wire_sql};
use crate::ipc::registry::{
    IpcDriverManifest, IpcDriverRegistry, LimitStyle, TableReferenceSchemaMode,
};
use crate::mssql::MsSqlPlugin;
use crate::mysql::MySqlPlugin;
use crate::oracle::OraclePlugin;
use crate::plugin::{
    ConnectionLifecycle, DatabasePlugin, PaginatedQuery, SqlCompletionInfo,
    format_binary_literal_for_database, parse_table_data_total_count,
};
use crate::plugin_manifest::{DatabaseCapabilities, DatabaseUiManifest};
use crate::postgresql::PostgresPlugin;
use crate::schema_preferences::{
    SchemaFilterProfile, filter_schemas, schema_filter_profile_for_database_type,
};
use crate::sqlite::SqlitePlugin;
use crate::ssh_tunnel::resolve_connection_target;
use crate::streaming_parser::StreamingSqlParser;
use crate::tdengine::TdenginePlugin;
use crate::types::*;
use anyhow::Result;
use async_trait::async_trait;
use extension_protocol::{
    conn::ConnTestResult, ddl as wire_ddl, method as wire_method, schema as wire_schema,
};
use one_core::storage::{DatabaseType, DbConnectionConfig};
use rust_i18n::t;
use sqlparser::dialect::{Dialect, GenericDialect};
use std::collections::HashMap;
use std::sync::Arc;

type RegistryReloader = dyn Fn() -> IpcDriverRegistry + Send + Sync;
const MIN_CUSTOM_COLUMN_WIDTH_PX: f32 = 1.0;

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

    fn is_oracle_compatible(&self) -> bool {
        matches!(
            self.driver.dialect.compatible_database_type,
            Some(DatabaseType::Oracle)
        )
    }

    fn schema_filter_profile(&self) -> SchemaFilterProfile {
        self.driver
            .dialect
            .compatible_database_type
            .as_ref()
            .map(schema_filter_profile_for_database_type)
            .unwrap_or(SchemaFilterProfile::None)
    }

    fn oracle_table_save_request(&self, request: &TableSaveRequest) -> TableSaveRequest {
        let mut request = request.clone();
        let uses_schema_as_database = self.driver.dialect.uses_schema_as_database
            || self.driver.effective_capabilities().uses_schema_as_database;
        if request.schema.is_none()
            && uses_schema_as_database
            && !request.database.trim().is_empty()
        {
            request.schema = Some(request.database.clone());
        }
        request
    }

    fn driver_for_config(&self, config: &DbConnectionConfig) -> Result<IpcDriverManifest, DbError> {
        let driver_id = driver_id_for_config(config)?;
        if let Some(registry) = &self.registry {
            if let Some(reloader) = &self.registry_reloader {
                if let Some(driver) = reloader().find(driver_id) {
                    return Ok(driver);
                }
            }
            if let Some(driver) = registry.find(driver_id) {
                return Ok(driver);
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
        let target = resolve_connection_target(config).await?;
        let client = JsonRpcClient::start_with_connection_config(driver, Some(config)).await?;
        let params = conn_test_params_value(config, &driver.id, (&target.host, target.port));
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

    async fn connectionless_ddl_build(
        &self,
        op: wire_ddl::DdlBuildOp,
        payload: serde_json::Value,
    ) -> Result<Option<wire_ddl::BuildDdlResult>> {
        if !self
            .driver
            .methods
            .iter()
            .any(|method| method == wire_method::DDL_BUILD)
        {
            return Ok(None);
        }
        let client = JsonRpcClient::start(&self.driver).await?;
        let params = serde_json::to_value(wire_ddl::BuildDdlParams {
            conn_id: None,
            op,
            payload,
        })?;
        let result = client
            .request::<wire_ddl::BuildDdlResult>(wire_method::DDL_BUILD, params)
            .await;
        client.shutdown().await;

        match result {
            Ok(result) => Ok(Some(result)),
            Err(DbError::NotSupported(_)) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn custom_object_view(
        &self,
        connection: &dyn DbConnection,
        view: wire_schema::ObjectViewKind,
        db_node_type: DbNodeType,
        default_title: &str,
        scope: ObjectViewScope<'_>,
    ) -> Result<Option<ObjectView>> {
        let mut params = serde_json::Map::new();
        params.insert("view".to_string(), serde_json::json!(view.as_str()));
        if let Some(database) = scope.database {
            params.insert("database".to_string(), serde_json::json!(database));
        }
        if let Some(schema) = scope.schema {
            params.insert("schema".to_string(), serde_json::json!(schema));
        }
        if let Some(table) = scope.table {
            params.insert("table".to_string(), serde_json::json!(table));
        }

        let view = self
            .optional_metadata::<wire_schema::ObjectView>(
                connection,
                wire_method::SCHEMA_OBJECT_VIEW,
                serde_json::Value::Object(params),
            )
            .await?;

        Ok(view.and_then(|view| object_view_from_wire(db_node_type, default_title, view)))
    }

    fn compatible_plugin(&self) -> Option<Box<dyn DatabasePlugin>> {
        compatible_plugin_for(self.driver.dialect.compatible_database_type.clone()?)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ObjectViewScope<'a> {
    database: Option<&'a str>,
    schema: Option<&'a str>,
    table: Option<&'a str>,
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
        api: "database".into(),
        category: None,
        description: String::new(),
        version: String::new(),
        engines: Default::default(),
        compatibility: serde_json::Value::Null,
        entry: crate::ipc::registry::IpcDriverEntry {
            command: String::new(),
            commands: Default::default(),
            args: Vec::new(),
            working_dir: None,
            env_from_config: Default::default(),
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
        DatabaseType::TDengine => Some(Box::new(TdenginePlugin::new())),
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

fn conn_test_params_value(
    config: &DbConnectionConfig,
    driver_id: &str,
    target: (&str, u16),
) -> serde_json::Value {
    serde_json::json!({
        "driver_id": driver_id,
        "config": driver_config_value_with_target(config, target.0, target.1),
    })
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
        if let Some(view) = self
            .custom_object_view(
                connection,
                wire_schema::ObjectViewKind::Databases,
                DbNodeType::Database,
                "Databases",
                ObjectViewScope::default(),
            )
            .await?
        {
            return Ok(view);
        }

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

    fn supports_rowid(&self) -> bool {
        self.driver
            .dialect
            .row_id_column
            .as_deref()
            .is_some_and(|column| !column.trim().is_empty())
    }

    fn rowid_column_alias(&self) -> &str {
        self.driver
            .dialect
            .row_id_alias
            .as_deref()
            .filter(|alias| !alias.trim().is_empty())
            .unwrap_or("__rowid__")
    }

    fn sql_dialect(&self) -> Box<dyn Dialect> {
        Box::new(GenericDialect {})
    }

    fn split_sql_statements(&self, sql: &str) -> Vec<String> {
        let trimmed = sql.trim();
        if trimmed.starts_with(WIRE_PREFIX) {
            return split_wire_script(trimmed);
        }
        split_sql_with_parser(trimmed, self.name())
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

    async fn list_schemas(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<String>> {
        let schemas = self
            .metadata(
                connection,
                wire_method::SCHEMA_SCHEMAS,
                serde_json::json!({ "database": database }),
            )
            .await
            .map(|schemas: Vec<wire_schema::SchemaInfo>| {
                schemas.into_iter().map(|schema| schema.name).collect()
            })?;

        Ok(filter_schemas(
            connection.config(),
            self.schema_filter_profile(),
            schemas,
        ))
    }

    async fn list_schemas_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        if let Some(view) = self
            .custom_object_view(
                connection,
                wire_schema::ObjectViewKind::Schemas,
                DbNodeType::Schema,
                "Schemas",
                ObjectViewScope {
                    database: Some(database),
                    ..Default::default()
                },
            )
            .await?
        {
            return Ok(view);
        }

        let rows = self
            .list_schemas(connection, database)
            .await?
            .into_iter()
            .map(|schema| vec![schema])
            .collect();
        Ok(object_view(
            DbNodeType::Schema,
            "Schemas",
            vec!["Name"],
            rows,
        ))
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
        if let Some(view) = self
            .custom_object_view(
                connection,
                wire_schema::ObjectViewKind::Tables,
                DbNodeType::Table,
                "Tables",
                ObjectViewScope {
                    database: Some(database),
                    schema: schema.as_deref(),
                    table: None,
                },
            )
            .await?
        {
            return Ok(view);
        }

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
        if let Some(view) = self
            .custom_object_view(
                connection,
                wire_schema::ObjectViewKind::Columns,
                DbNodeType::Column,
                "Columns",
                ObjectViewScope {
                    database: Some(database),
                    schema: schema.as_deref(),
                    table: Some(table),
                },
            )
            .await?
        {
            return Ok(view);
        }

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
        if !self.capabilities().supports_indexes {
            return Ok(Vec::new());
        }

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
        if let Some(view) = self
            .custom_object_view(
                connection,
                wire_schema::ObjectViewKind::Indexes,
                DbNodeType::Index,
                "Indexes",
                ObjectViewScope {
                    database: Some(database),
                    schema,
                    table: Some(table),
                },
            )
            .await?
        {
            return Ok(view);
        }

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
        Ok(triggers
            .into_iter()
            .map(|trigger| trigger_info_from_wire(trigger, Some(table)))
            .collect())
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
        Ok(checks
            .into_iter()
            .map(|check| check_info_from_wire(check, Some(table)))
            .collect())
    }

    async fn list_views(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<ViewInfo>> {
        let views: Vec<wire_schema::ViewInfo> = self
            .optional_metadata(
                connection,
                wire_method::SCHEMA_VIEWS,
                serde_json::json!({
                    "database": database,
                    "schema": schema,
                }),
            )
            .await?
            .unwrap_or_default();
        Ok(views.into_iter().map(view_info_from_wire).collect())
    }

    async fn list_views_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        if let Some(view) = self
            .custom_object_view(
                connection,
                wire_schema::ObjectViewKind::Views,
                DbNodeType::View,
                "Views",
                ObjectViewScope {
                    database: Some(database),
                    ..Default::default()
                },
            )
            .await?
        {
            return Ok(view);
        }

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

    async fn list_functions(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>> {
        self.list_functions_in_schema(connection, database, None)
            .await
    }

    async fn list_functions_in_schema(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<FunctionInfo>> {
        let functions: Vec<wire_schema::FunctionInfo> = self
            .optional_metadata(
                connection,
                wire_method::SCHEMA_FUNCTIONS,
                serde_json::json!({
                    "database": database,
                    "schema": schema,
                }),
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
        if let Some(view) = self
            .custom_object_view(
                connection,
                wire_schema::ObjectViewKind::Functions,
                DbNodeType::Function,
                "Functions",
                ObjectViewScope {
                    database: Some(database),
                    ..Default::default()
                },
            )
            .await?
        {
            return Ok(view);
        }

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

    fn capabilities(&self) -> DatabaseCapabilities {
        self.driver.effective_capabilities()
    }

    fn ui_manifest(&self) -> DatabaseUiManifest {
        self.driver.ui.form.clone().unwrap_or_default()
    }

    fn external_driver_manifest(&self) -> Option<IpcDriverManifest> {
        Some(self.driver.clone())
    }

    async fn list_procedures(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>> {
        self.list_procedures_in_schema(connection, database, None)
            .await
    }

    async fn list_procedures_in_schema(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<FunctionInfo>> {
        let procedures: Vec<wire_schema::ProcedureInfo> = self
            .optional_metadata(
                connection,
                wire_method::SCHEMA_PROCEDURES,
                serde_json::json!({
                    "database": database,
                    "schema": schema,
                }),
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
        if let Some(view) = self
            .custom_object_view(
                connection,
                wire_schema::ObjectViewKind::Procedures,
                DbNodeType::Procedure,
                "Procedures",
                ObjectViewScope {
                    database: Some(database),
                    ..Default::default()
                },
            )
            .await?
        {
            return Ok(view);
        }

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
        self.list_triggers_in_schema(connection, database, None)
            .await
    }

    async fn list_triggers_in_schema(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<TriggerInfo>> {
        let triggers: Vec<wire_schema::TriggerInfo> = self
            .optional_metadata(
                connection,
                wire_method::SCHEMA_TRIGGERS,
                serde_json::json!({
                    "database": database,
                    "schema": schema,
                }),
            )
            .await?
            .unwrap_or_default();
        Ok(triggers
            .into_iter()
            .map(|trigger| trigger_info_from_wire(trigger, None))
            .collect())
    }

    async fn list_triggers_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        if let Some(view) = self
            .custom_object_view(
                connection,
                wire_schema::ObjectViewKind::Triggers,
                DbNodeType::Trigger,
                "Triggers",
                ObjectViewScope {
                    database: Some(database),
                    ..Default::default()
                },
            )
            .await?
        {
            return Ok(view);
        }

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
        if let Some(view) = self
            .custom_object_view(
                connection,
                wire_schema::ObjectViewKind::Sequences,
                DbNodeType::Sequence,
                "Sequences",
                ObjectViewScope {
                    database: Some(database),
                    ..Default::default()
                },
            )
            .await?
        {
            return Ok(view);
        }

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

    async fn build_create_database_sql_async(
        &self,
        request: &crate::plugin::DatabaseOperationRequest,
    ) -> Result<String> {
        let Some(result) = self
            .connectionless_ddl_build(
                wire_ddl::DdlBuildOp::CreateDatabase,
                serde_json::json!({
                    "database_name": request.database_name,
                    "field_values": request.field_values,
                }),
            )
            .await?
        else {
            return Ok(self.build_create_database_sql(request));
        };
        Ok(join_ddl_statements(result.statements, None))
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

    async fn build_drop_database_sql_async(&self, database_name: &str) -> Result<String> {
        let Some(result) = self
            .connectionless_ddl_build(
                wire_ddl::DdlBuildOp::DropDatabase,
                serde_json::json!({
                    "name": database_name,
                    "database_name": database_name,
                }),
            )
            .await?
        else {
            return Ok(self.build_drop_database_sql(database_name));
        };
        Ok(join_ddl_statements(result.statements, None))
    }

    fn build_list_users_sql(&self, database: Option<&str>) -> Option<String> {
        let fallback = self
            .compatible_plugin()
            .and_then(|plugin| plugin.build_list_users_sql(database));
        let declares_methods = !self.driver.methods.is_empty();
        let supports_users_method = self
            .driver
            .methods
            .iter()
            .any(|method| method == wire_method::SCHEMA_USERS);

        if supports_users_method || !declares_methods {
            return Some(schema_users_wire_sql(fallback.as_deref()));
        }

        fallback
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

    fn build_paginated_query(
        &self,
        base_sql: &str,
        limit: usize,
        offset: usize,
        order_clause: &str,
    ) -> PaginatedQuery {
        if self
            .driver
            .dialect
            .compatible_database_type
            .as_ref()
            .is_some_and(|database_type| matches!(database_type, DatabaseType::Oracle))
        {
            return OraclePlugin::new().build_paginated_query(
                base_sql,
                limit,
                offset,
                order_clause,
            );
        }

        PaginatedQuery::new(format!(
            "{}{}",
            base_sql,
            self.format_pagination(limit, offset, order_clause)
        ))
    }

    fn format_table_reference(&self, database: &str, schema: Option<&str>, table: &str) -> String {
        let capabilities = self.driver.effective_capabilities();
        let prefers_schema = matches!(
            self.driver.dialect.table_reference_schema_mode,
            TableReferenceSchemaMode::PreferSchema
        );
        let uses_true_schema =
            capabilities.supports_schema && !capabilities.uses_schema_as_database;
        if prefers_schema || uses_true_schema {
            if let Some(schema) = schema.filter(|schema| !schema.trim().is_empty()) {
                return format!(
                    "{}.{}",
                    self.quote_identifier(schema),
                    self.quote_identifier(table)
                );
            }
            // True-schema databases are database-bound via the connection, so the database
            // name is never a valid table qualifier. In Informix/GBase 8s in particular,
            // `db.table` is parsed as `owner.table`, so fall back to the bare table name
            // when no schema is available instead of emitting an invalid `database.table`.
            if uses_true_schema {
                return self.quote_identifier(table);
            }
        }

        format!(
            "{}.{}",
            self.quote_identifier(database),
            self.quote_identifier(table)
        )
    }

    async fn query_table_data(
        &self,
        connection: &dyn DbConnection,
        request: TableDataRequest,
    ) -> Result<TableDataResponse> {
        let start_time = std::time::Instant::now();

        let where_clause = match request.where_clause {
            Some(ref clause) if !clause.trim().is_empty() => format!(" WHERE {}", clause.trim()),
            _ => String::new(),
        };
        let mut order_clause = match request.order_by_clause {
            Some(ref clause) if !clause.trim().is_empty() => format!(" ORDER BY {}", clause.trim()),
            _ => String::new(),
        };

        if order_clause.is_empty() {
            if let Some(default_order_by) = self
                .driver
                .dialect
                .default_order_by
                .as_deref()
                .filter(|order_by| !order_by.trim().is_empty())
            {
                order_clause = format!(" ORDER BY {}", default_order_by.trim());
            }
        }

        let offset = request.effective_offset();
        let table_ref = self.format_table_reference(
            &request.database,
            request.schema.as_deref(),
            &request.table,
        );

        let total_count = match request.known_total_count {
            Some(total_count) => total_count,
            None => {
                let count_sql = format!("SELECT COUNT(*) FROM {}{}", table_ref, where_clause);
                parse_table_data_total_count(connection.query(&count_sql).await?)?
            }
        };

        let base_sql = if let Some(row_id_column) = self
            .driver
            .dialect
            .row_id_column
            .as_deref()
            .filter(|column| !column.trim().is_empty())
        {
            let row_id_alias = self
                .driver
                .dialect
                .row_id_alias
                .as_deref()
                .filter(|alias| !alias.trim().is_empty())
                .unwrap_or("__rowid__");
            format!(
                "SELECT {} AS {}, t.* FROM {} t{}{}",
                row_id_column.trim(),
                self.quote_identifier(row_id_alias.trim()),
                table_ref,
                where_clause,
                order_clause
            )
        } else {
            format!(
                "SELECT * FROM {}{}{}",
                table_ref, where_clause, order_clause
            )
        };
        let paginated_query =
            self.build_paginated_query(&base_sql, request.page_size, offset, &order_clause);

        let sql_result = connection.query(&paginated_query.sql).await?;
        let duration = start_time.elapsed().as_millis();

        let mut query_result = match sql_result {
            SqlResult::Query(query_result) => Ok::<QueryResult, anyhow::Error>(query_result),
            SqlResult::Exec(_) => anyhow::bail!(t!("Error.query_type_error")),
            SqlResult::Error(sql_error_info) => anyhow::bail!(sql_error_info.message),
        }?;
        paginated_query.strip_hidden_result_columns(&mut query_result)?;
        crate::query_result_normalization::normalize_table_query_result(
            self,
            connection,
            &request.database,
            request.schema.as_deref(),
            &request.table,
            &mut query_result,
        )
        .await?;

        Ok(TableDataResponse {
            query_result,
            total_count,
            page: request.page,
            page_size: request.page_size,
            duration,
        })
    }

    fn generate_table_changes_sql(&self, request: &TableSaveRequest) -> String {
        if self.is_oracle_compatible() {
            let oracle_plugin = OraclePlugin::new();
            return oracle_plugin
                .generate_table_changes_sql(&self.oracle_table_save_request(request));
        }

        let mut sql_statements = Vec::new();

        for change in &request.changes {
            if let Some(sql) = self.build_table_change_sql(request, change) {
                sql_statements.push(sql);
            }
        }

        if sql_statements.is_empty() {
            t!("Error.no_changes").to_string()
        } else {
            sql_statements.join(";\n\n") + ";"
        }
    }

    fn format_boolean_value(&self, v: &str) -> String {
        if v == "1" || v.eq_ignore_ascii_case("true") {
            self.driver.dialect.bool_true.clone()
        } else {
            self.driver.dialect.bool_false.clone()
        }
    }

    fn format_table_change_value(
        &self,
        value: &TableCellValue,
        column: Option<&ColumnInfo>,
    ) -> String {
        let Some(database_type) = self.driver.dialect.compatible_database_type.as_ref() else {
            return crate::sql_literal::format_table_value_for_database(
                &self.name(),
                value,
                column,
            );
        };

        if let Some(plugin) = compatible_plugin_for(database_type.clone()) {
            plugin.format_table_change_value(value, column)
        } else {
            crate::sql_literal::format_table_value_for_database(database_type, value, column)
        }
    }

    fn format_binary_literal(&self, bytes: &[u8]) -> String {
        self.driver
            .dialect
            .compatible_database_type
            .as_ref()
            .map(|database_type| format_binary_literal_for_database(database_type, bytes))
            .unwrap_or_else(|| format_binary_literal_for_database(&self.name(), bytes))
    }

    fn build_limit_clause(&self) -> String {
        match self.driver.dialect.limit_style {
            LimitStyle::LimitOffset => "LIMIT".to_string(),
            LimitStyle::OffsetFetch => String::new(),
        }
    }

    fn build_where_and_limit_clause(
        &self,
        request: &TableSaveRequest,
        original_data: &[TableCellValue],
    ) -> (String, String) {
        (
            self.build_table_change_where_clause(request, original_data),
            String::new(),
        )
    }

    async fn drop_database_async(&self, database: &str) -> Result<String> {
        self.build_drop_database_sql_async(database).await
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
        let mut definitions = design
            .columns
            .iter()
            .map(|column| self.build_column_def(column))
            .collect::<Vec<_>>();
        definitions.extend(
            design
                .foreign_keys
                .iter()
                .map(|foreign_key| self.build_foreign_key_def(foreign_key)),
        );
        format!(
            "CREATE TABLE {} ({})",
            self.quote_identifier(&design.table_name),
            definitions.join(", ")
        )
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

    async fn build_create_table_sql_with_schema_async(
        &self,
        connection: &dyn DbConnection,
        schema: Option<&str>,
        design: &TableDesign,
    ) -> Result<String> {
        let params = wire_ddl::BuildCreateTableParams {
            conn_id: None,
            spec: table_spec_from_design_with_schema(design, schema),
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
        let original_foreign_keys: HashMap<&str, &ForeignKeyDefinition> = original
            .foreign_keys
            .iter()
            .map(|foreign_key| (foreign_key.name.as_str(), foreign_key))
            .collect();
        let new_foreign_keys: HashMap<&str, &ForeignKeyDefinition> = new
            .foreign_keys
            .iter()
            .map(|foreign_key| (foreign_key.name.as_str(), foreign_key))
            .collect();
        let mut statements = Vec::new();
        let (index_drops, index_creates) = self.build_index_change_sql(&table, original, new);

        for (name, original_foreign_key) in &original_foreign_keys {
            match new_foreign_keys.get(name) {
                Some(new_foreign_key)
                    if !self.foreign_key_changed(original_foreign_key, new_foreign_key) => {}
                _ => statements.push(self.build_drop_foreign_key_sql(&new.table_name, name)),
            }
        }
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
        for (name, new_foreign_key) in &new_foreign_keys {
            match original_foreign_keys.get(name) {
                Some(original_foreign_key)
                    if !self.foreign_key_changed(original_foreign_key, new_foreign_key) => {}
                _ => statements
                    .push(self.build_add_foreign_key_sql(&new.table_name, new_foreign_key)),
            }
        }

        if statements.is_empty() {
            "-- No changes detected".to_string()
        } else {
            statements.join("\n")
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

    async fn build_alter_table_sql_with_schema_async(
        &self,
        connection: &dyn DbConnection,
        schema: Option<&str>,
        original: &TableDesign,
        new: &TableDesign,
        column_renames: &[(String, String)],
    ) -> Result<String> {
        let params = wire_ddl::BuildAlterTableParams {
            conn_id: None,
            from_spec: table_spec_from_design_with_schema(original, schema),
            to_spec: table_spec_from_design_with_schema(new, schema),
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

    /// Prefer the driver's `schema/dump_ddl` result for structure export, falling
    /// back to the generic column-based builder whenever the driver does not
    /// implement the method, returns nothing, or the underlying DDL provider is
    /// unavailable on the server (e.g. the GBase 8s `get_ddl` SPL recipe).
    async fn export_table_create_sql(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<String> {
        if self
            .driver
            .methods
            .iter()
            .any(|method| method == wire_method::SCHEMA_DUMP_DDL)
        {
            let mut params = serde_json::to_value(wire_schema::DumpDdlParams {
                conn_id: 0,
                objects: vec![wire_schema::ObjectRef {
                    kind: wire_schema::ObjectKind::Table,
                    name: table.to_string(),
                    schema: schema.map(ToOwned::to_owned),
                    database: Some(database.to_string()),
                }],
                options: wire_schema::DumpDdlOptions::default(),
            })?;
            // The per-connection driver_request_value auto-injects the real
            // conn_id for schema-namespace methods; omit it here.
            if let Some(obj) = params.as_object_mut() {
                obj.remove("conn_id");
            }
            let dump = self
                .metadata::<wire_schema::DumpDdlResult>(
                    connection,
                    wire_method::SCHEMA_DUMP_DDL,
                    params,
                )
                .await;
            if let Ok(result) = dump {
                // Only trust statements that actually carry DDL. Some drivers
                // (e.g. openGauss) reply to `schema/dump_ddl` for tables with a
                // placeholder comment when they have no server-side provider;
                // treating that as real output would replace the exported
                // structure with a useless comment, so drop comment-only
                // statements and fall back to the generic builder.
                let real: Vec<String> = result
                    .statements
                    .into_iter()
                    .filter(|statement| dump_statement_has_ddl(statement))
                    .collect();
                let joined = real.join("\n");
                // The exporter appends `;` after the returned string, so strip
                // trailing terminators/whitespace to avoid a double semicolon.
                let joined = joined.trim_end().trim_end_matches(';').trim_end();
                if !joined.is_empty() {
                    return Ok(joined.to_string());
                }
            }
        }
        // Call the shared generic builder directly: a `DatabasePlugin::method`
        // dispatch from inside an override would resolve back to this override
        // and recurse, so opt into the default body via the free function.
        crate::plugin::default_export_table_create_sql(self, connection, database, schema, table)
            .await
    }
}

/// Returns true when `statement` contains at least one line of real DDL, i.e.
/// it is not blank and not a `--` SQL comment. Drivers that cannot provide a
/// table definition sometimes emit a placeholder comment through
/// `schema/dump_ddl`; such output must not be treated as an exported structure.
fn dump_statement_has_ddl(statement: &str) -> bool {
    statement.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty() && !trimmed.starts_with("--")
    })
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
    table_spec_from_design_with_schema(design, None)
}

// Build a TableSpec from a TableDesign, optionally qualifying the table with an
// explicit schema. When a non-empty schema is provided the connection database
// name is dropped: Oracle/PostgreSQL-compatible drivers (DM, Kingbase) treat
// the `database` qualifier as the schema/owner and would otherwise emit
// `"库名"."表名"` which neither database resolves.
fn table_spec_from_design_with_schema(
    design: &TableDesign,
    schema: Option<&str>,
) -> wire_ddl::TableSpec {
    let schema = schema.filter(|schema| !schema.trim().is_empty());
    let database = match schema {
        Some(_) => None,
        None => Some(design.database_name.clone()).filter(|database| !database.is_empty()),
    };
    wire_ddl::TableSpec {
        name: design.table_name.clone(),
        schema: schema.map(str::to_string),
        database,
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
        to_schema: foreign_key.ref_schema.clone(),
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
        object_type: match object.kind {
            wire_schema::ObjectKind::View | wire_schema::ObjectKind::MaterializedView => {
                TableObjectType::View
            }
            _ => TableObjectType::Table,
        },
        schema: empty_to_none(object.schema),
        comment: empty_to_none(object.comment),
        engine: None,
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
        is_primary: index.is_primary,
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
        ref_schema: foreign_key.to_schema,
        ref_columns: foreign_key.to_columns,
        on_delete: foreign_key.on_delete.unwrap_or_default(),
        on_update: foreign_key.on_update.unwrap_or_default(),
    }
}

fn check_info_from_wire(check: wire_schema::CheckInfo, fallback_table: Option<&str>) -> CheckInfo {
    CheckInfo {
        name: check.name,
        table_name: if check.table.is_empty() {
            fallback_table.unwrap_or_default().to_string()
        } else {
            check.table
        },
        definition: check.definition,
    }
}

fn function_info_from_wire(function: wire_schema::FunctionInfo) -> FunctionInfo {
    FunctionInfo {
        name: function.name,
        schema: None,
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
        identity_arguments: None,
        object_id: None,
        definition: function.definition,
        comment: empty_to_none(function.comment),
    }
}

fn trigger_info_from_wire(
    trigger: wire_schema::TriggerInfo,
    fallback_table: Option<&str>,
) -> TriggerInfo {
    TriggerInfo {
        name: trigger.name,
        table_name: if trigger.table.is_empty() {
            fallback_table.unwrap_or_default().to_string()
        } else {
            trigger.table
        },
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
            .map(|name| ObjectViewColumn::new(name, name))
            .collect(),
        rows,
    }
}

fn object_view_from_wire(
    db_node_type: DbNodeType,
    default_title: &str,
    view: wire_schema::ObjectView,
) -> Option<ObjectView> {
    if view.columns.is_empty() {
        return None;
    }

    let column_count = view.columns.len();
    let columns = view
        .columns
        .into_iter()
        .map(column_from_wire)
        .collect::<Vec<_>>();
    let rows = view
        .rows
        .into_iter()
        .map(|row| normalize_object_view_row(row, column_count))
        .collect();
    let title = if view.title.is_empty() {
        default_title.to_string()
    } else {
        view.title
    };

    Some(ObjectView {
        db_node_type,
        title,
        columns,
        rows,
    })
}

fn column_from_wire(column: wire_schema::ObjectViewColumn) -> ObjectViewColumn {
    let mut result = ObjectViewColumn::new(column.key, column.name);
    if let Some(width) = column
        .width_px
        .filter(|width| width.is_finite() && *width >= MIN_CUSTOM_COLUMN_WIDTH_PX)
    {
        result = result.width(width);
    }
    match column.align {
        Some(wire_schema::ObjectViewColumnAlign::Center) => result.text_center(),
        Some(wire_schema::ObjectViewColumnAlign::Right) => result.text_right(),
        Some(wire_schema::ObjectViewColumnAlign::Left) | None => result,
    }
}

fn normalize_object_view_row(mut row: Vec<String>, column_count: usize) -> Vec<String> {
    row.truncate(column_count);
    row.resize(column_count, String::new());
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::QueryResult;
    use crate::connection::StreamingProgress;
    use crate::executor::{ExecOptions, SqlResult, SqlSource};
    use crate::ipc::protocol::WIRE_PREFIX;
    use std::path::PathBuf;
    use tokio::sync::mpsc;

    struct DriverRequestOnlyConnection {
        config: DbConnectionConfig,
        supports_alter_table_builder: bool,
        object_view: Option<serde_json::Value>,
        last_alter_schema: std::sync::Mutex<Option<String>>,
    }

    struct RecordingQueryConnection {
        config: DbConnectionConfig,
        queries: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingQueryConnection {
        fn new() -> Self {
            Self {
                config: DriverRequestOnlyConnection::new().config,
                queries: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn queries(&self) -> Vec<String> {
            self.queries.lock().expect("queries mutex poisoned").clone()
        }
    }

    fn column_info(name: &str, data_type: &str, is_primary_key: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            is_nullable: !is_primary_key,
            is_primary_key,
            default_value: None,
            comment: None,
            charset: None,
            collation: None,
        }
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
                    proxy: None,
                    credential_reference: None,
                    extra_params: Default::default(),
                },
                supports_alter_table_builder: true,
                object_view: None,
                last_alter_schema: std::sync::Mutex::new(None),
            }
        }

        fn with_object_view(object_view: serde_json::Value) -> Self {
            Self {
                object_view: Some(object_view),
                ..Self::new()
            }
        }

        fn without_alter_table_builder() -> Self {
            Self {
                supports_alter_table_builder: false,
                ..Self::new()
            }
        }

        fn last_alter_schema(&self) -> Option<String> {
            self.last_alter_schema
                .lock()
                .expect("last_alter_schema mutex poisoned")
                .clone()
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
            params: serde_json::Value,
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
                    "definition": "payload IS NOT NULL",
                    "comment": "",
                    "extra": null
                }])),
                wire_method::SCHEMA_TRIGGERS if params.get("table").is_some() => {
                    Ok(serde_json::json!([{
                        "name": "events_audit_trigger",
                        "timing": "after",
                        "event": "insert",
                        "definition": "INSERT INTO audit VALUES (NEW.id)",
                        "comment": "",
                        "extra": null
                    }]))
                }
                wire_method::SCHEMA_OBJECT_VIEW => self
                    .object_view
                    .clone()
                    .ok_or_else(|| DbError::NotSupported(method.to_string())),
                wire_method::DDL_BUILD_ALTER_TABLE if self.supports_alter_table_builder => {
                    if let Some(schema) = params["from_spec"]["schema"].as_str() {
                        *self
                            .last_alter_schema
                            .lock()
                            .expect("last_alter_schema mutex poisoned") = Some(schema.to_string());
                    }
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

    #[async_trait]
    impl DbConnection for RecordingQueryConnection {
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
            Err(DbError::query("execute should not be used by table data"))
        }

        async fn query(&self, query: &str) -> Result<SqlResult, DbError> {
            self.queries
                .lock()
                .expect("queries mutex poisoned")
                .push(query.to_string());
            if query.starts_with("SELECT COUNT(*)") {
                return Ok(SqlResult::Query(QueryResult {
                    sql: query.to_string(),
                    columns: vec!["COUNT".into()],
                    column_meta: vec![],
                    rows: vec![vec![Some("1".into())]],
                    binary_cells: vec![],
                    elapsed_ms: 0,
                }));
            }
            let mut columns = vec!["__rowid__".into(), "ID".into()];
            let mut rows = vec![vec![Some("AAABBB".into()), Some("1".into())]];
            if query.contains("__navop_pagination_rownum__") {
                columns.push("__navop_pagination_rownum__".into());
                rows[0].push(Some("26".into()));
            }
            Ok(SqlResult::Query(QueryResult {
                sql: query.to_string(),
                columns,
                column_meta: vec![],
                rows,
                binary_cells: vec![],
                elapsed_ms: 0,
            }))
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

    struct ExportDdlConnection {
        config: DbConnectionConfig,
    }

    impl ExportDdlConnection {
        fn new() -> Self {
            Self {
                config: DriverRequestOnlyConnection::new().config,
            }
        }
    }

    #[async_trait]
    impl DbConnection for ExportDdlConnection {
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
            params: serde_json::Value,
        ) -> Result<serde_json::Value, DbError> {
            match method {
                wire_method::SCHEMA_COLUMNS => Ok(serde_json::json!([
                    {
                        "ordinal": 1,
                        "name": "id",
                        "type": "INTEGER",
                        "raw_type": "INTEGER",
                        "nullable": false,
                        "default": null,
                        "is_primary": true,
                        "is_unique": false,
                        "is_partition_key": false,
                        "is_clustering_key": false,
                        "max_length": null,
                        "precision": null,
                        "scale": null,
                        "comment": "event id",
                        "extra": null
                    },
                    {
                        "ordinal": 2,
                        "name": "name",
                        "type": "VARCHAR(64)",
                        "raw_type": "VARCHAR(64)",
                        "nullable": true,
                        "default": null,
                        "is_primary": false,
                        "is_unique": false,
                        "is_partition_key": false,
                        "is_clustering_key": false,
                        "max_length": 64,
                        "precision": null,
                        "scale": null,
                        "comment": "customer name",
                        "extra": null
                    }
                ])),
                wire_method::SCHEMA_OBJECTS if params["kinds"][0] == "table" => {
                    Ok(serde_json::json!([
                        {
                            "name": "events",
                            "kind": "table",
                            "schema": "",
                            "comment": "event stream",
                            "extra": null
                        }
                    ]))
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

    struct DumpDdlConnection {
        config: DbConnectionConfig,
        empty_dump: bool,
        comment_only: bool,
        dump_params: std::sync::Mutex<Option<serde_json::Value>>,
    }

    impl DumpDdlConnection {
        fn new() -> Self {
            Self::with_empty_dump(false)
        }

        fn with_comment_only_dump(comment_only: bool) -> Self {
            Self {
                config: DriverRequestOnlyConnection::new().config,
                empty_dump: false,
                comment_only,
                dump_params: std::sync::Mutex::new(None),
            }
        }

        fn with_empty_dump(empty_dump: bool) -> Self {
            Self {
                config: DriverRequestOnlyConnection::new().config,
                empty_dump,
                comment_only: false,
                dump_params: std::sync::Mutex::new(None),
            }
        }

        fn dump_params(&self) -> serde_json::Value {
            self.dump_params
                .lock()
                .expect("dump_params mutex poisoned")
                .clone()
                .unwrap_or(serde_json::Value::Null)
        }
    }

    #[async_trait]
    impl DbConnection for DumpDdlConnection {
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
            params: serde_json::Value,
        ) -> Result<serde_json::Value, DbError> {
            match method {
                wire_method::SCHEMA_DUMP_DDL => {
                    *self.dump_params.lock().expect("dump_params mutex poisoned") = Some(params);
                    if self.empty_dump {
                        Ok(serde_json::json!({ "statements": [] }))
                    } else if self.comment_only {
                        Ok(serde_json::json!({
                            "statements": [
                                "-- DDL dump for table events requires server-side pg_get_tabledef support"
                            ]
                        }))
                    } else {
                        Ok(serde_json::json!({
                            "statements": [
                                "CREATE TABLE events (\n    id INTEGER NOT NULL\n)\nALTER TABLE events ADD CONSTRAINT PRIMARY KEY (id);\nCOMMENT ON TABLE events IS 'event stream';"
                            ]
                        }))
                    }
                }
                wire_method::SCHEMA_COLUMNS => Ok(serde_json::json!([
                    {
                        "ordinal": 1,
                        "name": "id",
                        "type": "INTEGER",
                        "raw_type": "INTEGER",
                        "nullable": false,
                        "default": null,
                        "is_primary": true,
                        "is_unique": false,
                        "is_partition_key": false,
                        "is_clustering_key": false,
                        "max_length": null,
                        "precision": null,
                        "scale": null,
                        "comment": "event id",
                        "extra": null
                    },
                    {
                        "ordinal": 2,
                        "name": "name",
                        "type": "VARCHAR(64)",
                        "raw_type": "VARCHAR(64)",
                        "nullable": true,
                        "default": null,
                        "is_primary": false,
                        "is_unique": false,
                        "is_partition_key": false,
                        "is_clustering_key": false,
                        "max_length": 64,
                        "precision": null,
                        "scale": null,
                        "comment": "customer name",
                        "extra": null
                    }
                ])),
                wire_method::SCHEMA_OBJECTS if params["kinds"][0] == "table" => {
                    Ok(serde_json::json!([
                        {
                            "name": "events",
                            "kind": "table",
                            "schema": "",
                            "comment": "event stream",
                            "extra": null
                        }
                    ]))
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
    fn fixed_driver_plugin_formats_binary_for_compatible_database() {
        let mut postgres = driver_manifest("postgres-compatible", true, "postgres.connection");
        postgres.dialect.compatible_database_type = Some(DatabaseType::PostgreSQL);
        let postgres = ExternalDatabasePlugin::for_driver(postgres);
        assert_eq!(
            "decode('0001ff', 'hex')",
            postgres.format_binary_literal(&[0x00, 0x01, 0xff])
        );

        let mut duckdb = driver_manifest("duckdb-compatible", false, "duckdb.connection");
        duckdb.dialect.compatible_database_type = Some(DatabaseType::DuckDB);
        let duckdb = ExternalDatabasePlugin::for_driver(duckdb);
        assert_eq!(
            "from_hex('0001ff')",
            duckdb.format_binary_literal(&[0x00, 0x01, 0xff])
        );
    }

    #[test]
    fn compatible_external_driver_formats_table_change_literals() {
        let mut postgres = driver_manifest("postgres-compatible", true, "postgres.connection");
        postgres.dialect.compatible_database_type = Some(DatabaseType::PostgreSQL);
        let postgres = ExternalDatabasePlugin::for_driver(postgres);
        assert_eq!(
            "TRUE",
            postgres.format_table_change_value(
                &TableCellValue::Text("1".to_string()),
                Some(&column_info("enabled", "boolean", false)),
            )
        );

        let mut duckdb = driver_manifest("duckdb-compatible", false, "duckdb.connection");
        duckdb.dialect.compatible_database_type = Some(DatabaseType::DuckDB);
        let duckdb = ExternalDatabasePlugin::for_driver(duckdb);
        assert_eq!(
            "from_hex('deadbeef')",
            duckdb.format_table_change_value(
                &TableCellValue::Binary(vec![0xde, 0xad, 0xbe, 0xef]),
                Some(&column_info("payload", "blob", false)),
            )
        );

        let unknown =
            ExternalDatabasePlugin::for_driver(driver_manifest("unknown", false, "unknown"));
        assert_eq!(
            "'1'' OR 1=1'",
            unknown.format_table_change_value(
                &TableCellValue::Text("1' OR 1=1".to_string()),
                Some(&column_info("payload", "blob", false)),
            )
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
            proxy: None,
            credential_reference: None,
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
    fn external_user_listing_uses_schema_users_wire_method() {
        let mut driver = driver_manifest("users", false, "users.connection");
        driver.methods = vec![wire_method::SCHEMA_USERS.to_string()];
        driver.dialect.compatible_database_type = Some(DatabaseType::PostgreSQL);
        let plugin = ExternalDatabasePlugin::for_driver(driver);

        let sql = plugin
            .build_list_users_sql(None)
            .expect("schema/users should be available");
        let envelope: serde_json::Value =
            serde_json::from_str(sql.strip_prefix(WIRE_PREFIX).unwrap()).unwrap();

        assert_eq!(Some(wire_method::SCHEMA_USERS), envelope["method"].as_str());
        assert!(
            envelope["params"]["fallback_sql"]
                .as_str()
                .is_some_and(|sql| sql.contains("pg_catalog.pg_roles"))
        );
    }

    #[test]
    fn external_user_listing_falls_back_when_declared_method_set_excludes_users() {
        let mut driver = driver_manifest("users", false, "users.connection");
        driver.methods = vec![wire_method::SCHEMA_DATABASES.to_string()];
        driver.dialect.compatible_database_type = Some(DatabaseType::PostgreSQL);
        let plugin = ExternalDatabasePlugin::for_driver(driver);

        let sql = plugin
            .build_list_users_sql(None)
            .expect("compatible PostgreSQL fallback should be available");

        assert!(sql.contains("pg_catalog.pg_roles"));
        assert!(!sql.starts_with(WIRE_PREFIX));
    }

    #[test]
    fn external_table_reference_uses_schema_when_manifest_supports_schema() {
        let driver = driver_manifest("kingbase", true, "kingbase.connection");
        let plugin = ExternalDatabasePlugin::for_driver(driver);

        assert_eq!(
            "\"comi_ai_manager\".\"ai_agent_child_agent\"",
            plugin.format_table_reference(
                "ai_app",
                Some("comi_ai_manager"),
                "ai_agent_child_agent"
            )
        );
    }

    #[test]
    fn external_table_reference_uses_database_when_schema_is_database() {
        let mut driver = driver_manifest("mysqlish", true, "mysqlish.connection");
        driver
            .capabilities
            .as_mut()
            .expect("test manifest has capabilities")
            .uses_schema_as_database = true;
        let plugin = ExternalDatabasePlugin::for_driver(driver);

        assert_eq!(
            "\"tenant_db\".\"events\"",
            plugin.format_table_reference("tenant_db", Some("ignored_schema"), "events")
        );
    }

    #[test]
    fn external_table_reference_can_prefer_schema_from_manifest() {
        let mut driver = driver_manifest("ownerdb", true, "ownerdb.connection");
        driver.dialect.table_reference_schema_mode = TableReferenceSchemaMode::PreferSchema;
        let plugin = ExternalDatabasePlugin::for_driver(driver);

        assert_eq!(
            "\"APP\".\"EVENTS\"",
            plugin.format_table_reference("", Some("APP"), "EVENTS")
        );
    }

    #[test]
    fn external_table_reference_without_schema_omits_database_for_true_schema() {
        // GBase 8s / Informix-style driver: schema is a real owner namespace, the connection
        // is database-bound, and `db.table` is parsed as `owner.table` (so `db` is invalid).
        let mut driver = driver_manifest("gbase8s", true, "gbase8s.connection");
        driver.dialect.identifier_quote_left = String::new();
        driver.dialect.identifier_quote_right = None;
        let plugin = ExternalDatabasePlugin::for_driver(driver);

        assert_eq!(
            "informix.demo_parent",
            plugin.format_table_reference("onetcli_demo1", Some("informix"), "demo_parent")
        );
        assert_eq!(
            "demo_parent",
            plugin.format_table_reference("onetcli_demo1", None, "demo_parent")
        );
    }

    #[test]
    fn external_oracle_table_changes_use_oracle_date_literals() {
        let mut driver = driver_manifest("oracle-go", true, "oracle.connection");
        driver.dialect.compatible_database_type = Some(DatabaseType::Oracle);
        driver.dialect.uses_schema_as_database = true;
        let plugin = ExternalDatabasePlugin::for_driver(driver);
        let request = TableSaveRequest {
            database: "APP".to_string(),
            schema: None,
            table: "EVENTS".to_string(),
            columns: vec![
                column_info("ID", "NUMBER", true),
                column_info("STARTED_AT", "DATE", false),
            ],
            index_infos: vec![],
            changes: vec![TableRowChange::Added {
                data: vec!["1".into(), "2026-06-21 14:05:06".into()],
            }],
        };

        let sql = plugin.generate_table_changes_sql(&request);

        assert_eq!(
            "INSERT INTO \"APP\".\"EVENTS\" (\"ID\", \"STARTED_AT\") VALUES (1, TO_DATE('2026-06-21 14:05:06', 'YYYY-MM-DD HH24:MI:SS'));",
            sql
        );
    }

    #[test]
    fn external_oracle_table_changes_use_oracle_lob_literals() {
        let mut driver = driver_manifest("oracle-go", true, "oracle.connection");
        driver.dialect.compatible_database_type = Some(DatabaseType::Oracle);
        driver.dialect.uses_schema_as_database = true;
        let plugin = ExternalDatabasePlugin::for_driver(driver);
        let request = TableSaveRequest {
            database: "APP".to_string(),
            schema: None,
            table: "DOCS".to_string(),
            columns: vec![
                column_info("ID", "NUMBER", true),
                column_info("BODY", "CLOB", false),
            ],
            index_infos: vec![],
            changes: vec![TableRowChange::Updated {
                original_data: vec!["1".into(), "old".into()],
                changes: vec![TableCellChange {
                    column_index: 1,
                    column_name: "BODY".to_string(),
                    old_value: "old".into(),
                    new_value: "a".repeat(3_050).into(),
                }],
                rowid: Some("AAABBB".to_string()),
            }],
        };

        let sql = plugin.generate_table_changes_sql(&request);

        assert!(sql.contains("UPDATE \"APP\".\"DOCS\" SET \"BODY\" = TO_CLOB('"));
        assert!(sql.contains(" || TO_CLOB('"));
        assert!(sql.contains("WHERE ROWID = 'AAABBB'"));
    }

    #[tokio::test]
    async fn external_table_data_uses_manifest_row_id_query() {
        let mut driver = driver_manifest("ownerdb", true, "ownerdb.connection");
        driver.dialect.limit_style = LimitStyle::OffsetFetch;
        driver.dialect.table_reference_schema_mode = TableReferenceSchemaMode::PreferSchema;
        driver.dialect.row_id_column = Some("ROWID".to_string());
        driver.dialect.row_id_alias = Some("__rowid__".to_string());
        driver.dialect.default_order_by = Some("ROWID".to_string());
        let plugin = ExternalDatabasePlugin::for_driver(driver);
        let connection = RecordingQueryConnection::new();

        let response = plugin
            .query_table_data(
                &connection,
                TableDataRequest::new("", "EVENTS")
                    .with_schema("APP")
                    .with_page(1, 25),
            )
            .await
            .expect("table data query should succeed");

        assert_eq!(1, response.total_count);
        assert!(plugin.supports_rowid());
        assert_eq!("__rowid__", plugin.rowid_column_alias());
        let queries = connection.queries();
        assert_eq!("SELECT COUNT(*) FROM \"APP\".\"EVENTS\"", queries[0]);
        assert_eq!(
            "SELECT ROWID AS \"__rowid__\", t.* FROM \"APP\".\"EVENTS\" t ORDER BY ROWID OFFSET 0 ROWS FETCH NEXT 25 ROWS ONLY",
            queries[1]
        );
    }

    #[tokio::test]
    async fn external_oracle_table_data_uses_11g_rownum_pagination() {
        let mut driver = driver_manifest("oracle-ipc", true, "oracle.connection");
        driver.dialect.compatible_database_type = Some(DatabaseType::Oracle);
        driver.dialect.table_reference_schema_mode = TableReferenceSchemaMode::PreferSchema;
        driver.dialect.row_id_column = Some("ROWID".to_string());
        driver.dialect.row_id_alias = Some("__rowid__".to_string());
        driver.dialect.default_order_by = Some("ROWID".to_string());
        let plugin = ExternalDatabasePlugin::for_driver(driver);
        let connection = RecordingQueryConnection::new();

        let response = plugin
            .query_table_data(
                &connection,
                TableDataRequest::new("", "EVENTS")
                    .with_schema("APP")
                    .with_page(2, 25)
                    .with_known_total_count(100),
            )
            .await
            .expect("Oracle IPC table data query should succeed");

        assert_eq!(vec!["__rowid__", "ID"], response.query_result.columns);
        assert_eq!(
            vec![vec![Some("AAABBB".into()), Some("1".into())]],
            response.query_result.rows
        );
        let queries = connection.queries();
        assert_eq!(1, queries.len());
        assert!(!queries[0].contains(" LIMIT "));
        assert!(!queries[0].contains(" OFFSET "));
        assert!(!queries[0].contains("FETCH NEXT"));
        assert!(queries[0].contains(
            "SELECT ROWID AS \"__rowid__\", t.* FROM \"APP\".\"EVENTS\" t ORDER BY ROWID"
        ));
        assert!(queries[0].contains("WHERE ROWNUM <= 50"));
        assert!(queries[0].contains("\"__navop_pagination_rownum__\" > 25"));
    }

    #[tokio::test]
    async fn external_table_data_reports_custom_manifest_row_id_alias() {
        let mut driver = driver_manifest("ownerdb", true, "ownerdb.connection");
        driver.dialect.row_id_column = Some("ROWID".to_string());
        driver.dialect.row_id_alias = Some("dbx_rowid".to_string());
        let plugin = ExternalDatabasePlugin::for_driver(driver);

        assert!(plugin.supports_rowid());
        assert_eq!("dbx_rowid", plugin.rowid_column_alias());

        let connection = RecordingQueryConnection::new();
        plugin
            .query_table_data(
                &connection,
                TableDataRequest::new("ownerdb", "EVENTS").with_page(1, 25),
            )
            .await
            .expect("table data query should succeed");
        assert_eq!(
            "SELECT ROWID AS \"dbx_rowid\", t.* FROM \"ownerdb\".\"EVENTS\" t LIMIT 25 OFFSET 0",
            connection.queries()[1]
        );
    }

    #[tokio::test]
    async fn external_table_data_skips_count_when_total_is_known() {
        let plugin = ExternalDatabasePlugin::for_driver(driver_manifest(
            "duckdb",
            true,
            "duckdb.connection",
        ));
        let connection = RecordingQueryConnection::new();

        let response = plugin
            .query_table_data(
                &connection,
                TableDataRequest::new("main", "EVENTS")
                    .with_page(2, 25)
                    .with_offset(25)
                    .with_known_total_count(50),
            )
            .await
            .expect("table data query should reuse the known total");

        assert_eq!(50, response.total_count);
        let queries = connection.queries();
        assert_eq!(1, queries.len());
        assert_eq!(
            "SELECT * FROM \"main\".\"EVENTS\" LIMIT 25 OFFSET 25",
            queries[0]
        );
    }

    #[tokio::test]
    async fn external_table_data_ignores_legacy_driver_owned_table_data_method() {
        let mut driver: IpcDriverManifest = serde_json::from_str(
            r#"{
              "id":"legacy-query-extension",
              "name":"Legacy Query Extension",
              "entry":{"command":"./legacy"},
              "transport":{"name":"legacy.sock"},
              "query":{
                "default_language":"legacy_dsl",
                "languages":["legacy_dsl","sql"],
                "table_data_method":"x/legacy/table_data"
              },
              "methods":["x/legacy/table_data"],
              "dialect":{"supports_schema":true},
              "ui":{"form":{"schema_version":1,"forms":[{"kind":"Connection","title_i18n_key":"legacy.connection","submit_i18n_key":"submit","tabs":[]}],"actions":{"actions":[]}}}
            }"#,
        )
        .expect("legacy manifest with query extension should parse");
        driver.dialect.row_id_column = Some("ROWID".to_string());
        driver.dialect.row_id_alias = Some("__rowid__".to_string());
        let plugin = ExternalDatabasePlugin::for_driver(driver);
        let connection = RecordingQueryConnection::new();

        let response = plugin
            .query_table_data(
                &connection,
                TableDataRequest::new("main", "EVENTS").with_page(1, 25),
            )
            .await
            .expect("table data query should use host SQL");

        assert_eq!(1, response.total_count);
        let queries = connection.queries();
        assert_eq!("SELECT COUNT(*) FROM \"main\".\"EVENTS\"", queries[0]);
        assert_eq!(
            "SELECT ROWID AS \"__rowid__\", t.* FROM \"main\".\"EVENTS\" t LIMIT 25 OFFSET 0",
            queries[1]
        );
    }

    #[test]
    fn reloading_registry_finds_driver_added_after_plugin_creation() {
        let driver = driver_manifest("duckdb", true, "duckdb.connection");
        let plugin = ExternalDatabasePlugin::with_registry_reloader(
            IpcDriverRegistry::empty(),
            Arc::new(move || IpcDriverRegistry::from_drivers(vec![driver.clone()])),
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
            proxy: None,
            credential_reference: None,
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
    fn reloading_registry_prefers_updated_manifest_for_existing_driver_id() {
        let mut stale = driver_manifest("duckdb", true, "duckdb.connection");
        stale.dialect.identifier_quote_left = "`".to_string();
        stale.dialect.identifier_quote_right = Some("`".to_string());
        let mut fresh = stale.clone();
        fresh.dialect.identifier_quote_left = "\"".to_string();
        fresh.dialect.identifier_quote_right = Some("\"".to_string());

        let plugin = ExternalDatabasePlugin::with_registry_reloader(
            IpcDriverRegistry::from_drivers(vec![stale]),
            Arc::new(move || IpcDriverRegistry::from_drivers(vec![fresh.clone()])),
        );
        let config = DbConnectionConfig {
            id: "duckdb-conn".into(),
            name: "DuckDB".into(),
            database_type: DatabaseType::DuckDB,
            host: "/tmp/reloaded.duckdb".into(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            credential_reference: None,
            extra_params: Default::default(),
        };

        let driver = plugin.driver_for_config(&config).unwrap();
        let reloaded_plugin = ExternalDatabasePlugin::for_driver(driver);

        assert_eq!("\"value\"", reloaded_plugin.quote_identifier("value"));
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
            proxy: None,
            credential_reference: None,
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
    async fn default_export_table_create_sql_includes_primary_key_and_comments() {
        let plugin = ExternalDatabasePlugin::new();
        let connection = ExportDdlConnection::new();

        let ddl = plugin
            .export_table_create_sql(&connection, "main", None, "events")
            .await
            .expect("default export_table_create_sql should succeed");

        assert_eq!(
            "CREATE TABLE \"events\" (\n    \"id\" INTEGER NOT NULL,\n    \"name\" VARCHAR(64),\n    PRIMARY KEY (\"id\")\n)\nCOMMENT ON TABLE \"events\" IS 'event stream';\nCOMMENT ON COLUMN \"events\".\"id\" IS 'event id';\nCOMMENT ON COLUMN \"events\".\"name\" IS 'customer name'",
            ddl
        );
    }

    #[tokio::test]
    async fn export_table_create_sql_uses_driver_dump_ddl_when_declared() {
        let mut driver = driver_manifest("ddl-driver", true, "ddl-driver.connection");
        driver.methods = vec![wire_method::SCHEMA_DUMP_DDL.to_string()];
        let plugin = ExternalDatabasePlugin::for_driver(driver);
        let connection = DumpDdlConnection::new();

        let ddl = plugin
            .export_table_create_sql(&connection, "main", Some("public"), "events")
            .await
            .expect("driver schema/dump_ddl should succeed");

        let params = connection.dump_params();
        assert_eq!("table", params["objects"][0]["kind"].as_str().unwrap());
        assert_eq!("events", params["objects"][0]["name"].as_str().unwrap());
        assert_eq!("public", params["objects"][0]["schema"].as_str().unwrap());
        assert_eq!("main", params["objects"][0]["database"].as_str().unwrap());
        assert_eq!(Some(false), params["options"]["if_not_exists"].as_bool());
        assert!(
            params.get("conn_id").is_none(),
            "conn_id should be auto-injected by the host"
        );

        // The driver statement ends with `;`; the host strips the trailing
        // terminator because the SQL exporter appends one after the string.
        assert_eq!(
            "CREATE TABLE events (\n    id INTEGER NOT NULL\n)\nALTER TABLE events ADD CONSTRAINT PRIMARY KEY (id);\nCOMMENT ON TABLE events IS 'event stream'",
            ddl
        );
    }

    #[tokio::test]
    async fn export_table_create_sql_falls_back_when_dump_ddl_returns_nothing() {
        let mut driver = driver_manifest("ddl-driver", true, "ddl-driver.connection");
        driver.methods = vec![wire_method::SCHEMA_DUMP_DDL.to_string()];
        let plugin = ExternalDatabasePlugin::for_driver(driver);
        let connection = DumpDdlConnection::with_empty_dump(true);

        let ddl = plugin
            .export_table_create_sql(&connection, "main", None, "events")
            .await
            .expect("empty dump_ddl should fall back to the default builder");

        assert_eq!(
            "CREATE TABLE \"events\" (\n    \"id\" INTEGER NOT NULL,\n    \"name\" VARCHAR(64),\n    PRIMARY KEY (\"id\")\n)\nCOMMENT ON TABLE \"events\" IS 'event stream';\nCOMMENT ON COLUMN \"events\".\"id\" IS 'event id';\nCOMMENT ON COLUMN \"events\".\"name\" IS 'customer name'",
            ddl
        );
    }

    #[tokio::test]
    async fn export_table_create_sql_falls_back_when_dump_ddl_is_comment_only() {
        let mut driver = driver_manifest("ddl-driver", true, "ddl-driver.connection");
        driver.methods = vec![wire_method::SCHEMA_DUMP_DDL.to_string()];
        let plugin = ExternalDatabasePlugin::for_driver(driver);
        let connection = DumpDdlConnection::with_comment_only_dump(true);

        let ddl = plugin
            .export_table_create_sql(&connection, "main", None, "events")
            .await
            .expect("comment-only dump_ddl should fall back to the default builder");

        assert!(
            ddl.starts_with("CREATE TABLE \"events\""),
            "comment-only dump_ddl must not replace the exported structure with a comment"
        );
        assert!(ddl.contains("PRIMARY KEY (\"id\")"));
    }

    #[tokio::test]
    async fn object_view_uses_driver_columns_widths_and_rows() {
        let plugin = ExternalDatabasePlugin::new();
        let connection = DriverRequestOnlyConnection::with_object_view(serde_json::json!({
            "title": "Driver Columns",
            "columns": [
                {"key": "name", "name": "Field", "width_px": 220.0},
                {"key": "nullable", "name": "Null?", "width_px": 72.0, "align": "right"}
            ],
            "rows": [
                ["id", "false", "ignored"],
                ["payload"]
            ]
        }));

        let view = plugin
            .list_columns_view(&connection, "main", Some("public".to_string()), "events")
            .await
            .unwrap();

        assert_eq!(DbNodeType::Column, view.db_node_type);
        assert_eq!("Driver Columns", view.title);
        assert_eq!(2, view.columns.len());
        assert_eq!("Field", view.columns[0].label);
        assert_eq!(220.0, view.columns[0].width_px);
        assert_eq!("Null?", view.columns[1].label);
        assert_eq!(ObjectViewColumnAlign::Right, view.columns[1].align);
        assert_eq!(
            vec![
                vec!["id".to_string(), "false".to_string()],
                vec!["payload".to_string(), String::new()],
            ],
            view.rows
        );
    }

    #[tokio::test]
    async fn object_view_falls_back_when_driver_does_not_support_method() {
        let plugin = ExternalDatabasePlugin::new();
        let connection = DriverRequestOnlyConnection::new();

        let view = plugin.list_databases_view(&connection).await.unwrap();

        assert_eq!(DbNodeType::Database, view.db_node_type);
        assert_eq!("Databases", view.title);
        assert_eq!(2, view.columns.len());
        assert_eq!("Name", view.columns[0].label);
        assert_eq!(vec![vec!["mockdb".to_string(), String::new()]], view.rows);
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
    async fn list_views_returns_empty_when_driver_does_not_support_method() {
        let plugin = ExternalDatabasePlugin::new();
        let connection = DriverRequestOnlyConnection::new();

        let views = plugin.list_views(&connection, "main", None).await.unwrap();

        assert!(views.is_empty());
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

    #[tokio::test]
    async fn table_triggers_fill_table_from_request_when_driver_omits_it() {
        let plugin = ExternalDatabasePlugin::new();
        let connection = DriverRequestOnlyConnection::new();

        let triggers = plugin
            .list_table_triggers(&connection, "main", None, "events")
            .await
            .unwrap();

        assert_eq!(1, triggers.len());
        assert_eq!("events_audit_trigger", triggers[0].name);
        assert_eq!("events", triggers[0].table_name);
        assert_eq!("insert", triggers[0].event);
        assert_eq!("after", triggers[0].timing);
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
    fn conn_test_params_use_resolved_connection_target() {
        let mut extra_params = HashMap::new();
        extra_params.insert("ssh_tunnel_enabled".to_string(), "true".to_string());
        extra_params.insert("ssh_target_host".to_string(), "db.internal".to_string());
        extra_params.insert("ssh_target_port".to_string(), "6667".to_string());
        let config = DbConnectionConfig {
            id: "iotdb-conn".into(),
            name: "IoTDB".into(),
            database_type: DatabaseType::external("iotdb"),
            host: "db.internal".into(),
            port: 6667,
            username: "root".into(),
            password: "secret".into(),
            database: Some("main".into()),
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            credential_reference: None,
            extra_params,
        };

        let params = conn_test_params_value(&config, "iotdb", ("127.0.0.1", 16667));

        assert_eq!(serde_json::json!("iotdb"), params["driver_id"]);
        assert_eq!(serde_json::json!("127.0.0.1"), params["config"]["host"]);
        assert_eq!(serde_json::json!(16667), params["config"]["port"]);
        assert_eq!(
            serde_json::json!("db.internal"),
            params["config"]["extra_params"]["ssh_target_host"]
        );
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
    async fn async_alter_with_schema_forwards_schema_to_driver() {
        let plugin = ExternalDatabasePlugin::new();
        let connection = DriverRequestOnlyConnection::new();
        let mut original = TableDesign::new("comi_ai_manager", "AI_SCHEDULED_TASK");
        original.add_column(ColumnDefinition::new("payload").data_type("VARCHAR"));
        let mut current = TableDesign::new("comi_ai_manager", "AI_SCHEDULED_TASK");
        current.add_column(ColumnDefinition::new("body").data_type("VARCHAR"));

        let sql = plugin
            .build_alter_table_sql_with_schema_async(
                &connection,
                Some("comi_ai_manager"),
                &original,
                &current,
                &[("payload".to_string(), "body".to_string())],
            )
            .await
            .unwrap();

        assert_eq!("DRIVER RENAME SQL;", sql);
        assert_eq!(
            Some("comi_ai_manager".to_string()),
            connection.last_alter_schema()
        );
    }

    #[test]
    fn table_spec_with_schema_sets_schema_and_drops_database() {
        let mut design = TableDesign::new("comi_ai_manager", "AI_SCHEDULED_TASK");
        design.add_column(ColumnDefinition::new("id").data_type("INTEGER"));

        // Explicit schema: qualify with schema and drop the connection database.
        let spec = table_spec_from_design_with_schema(&design, Some("comi_ai_manager"));
        assert_eq!(Some("comi_ai_manager".to_string()), spec.schema);
        assert_eq!(None, spec.database);

        // No schema: fall back to the connection database name.
        let spec = table_spec_from_design_with_schema(&design, None);
        assert_eq!(None, spec.schema);
        assert_eq!(Some("comi_ai_manager".to_string()), spec.database);
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

    #[test]
    fn sync_create_table_builder_includes_foreign_keys() {
        let plugin = ExternalDatabasePlugin::new();
        let mut design = TableDesign::new("main", "order_items");
        design.add_column(ColumnDefinition::new("id").data_type("INTEGER"));
        design.add_column(ColumnDefinition::new("order_id").data_type("INTEGER"));
        design.foreign_keys.push(ForeignKeyDefinition {
            name: "fk_order_items_order".to_string(),
            columns: vec!["order_id".to_string()],
            ref_table: "orders".to_string(),
            ref_schema: None,
            ref_columns: vec!["id".to_string()],
            on_delete: "CASCADE".to_string(),
            on_update: "NO ACTION".to_string(),
        });

        let sql = plugin.build_create_table_sql(&design);

        assert!(sql.contains(
            "CONSTRAINT \"fk_order_items_order\" FOREIGN KEY (\"order_id\") REFERENCES \"orders\" (\"id\") ON DELETE CASCADE ON UPDATE NO ACTION"
        ));
    }

    #[test]
    fn sync_alter_table_builder_includes_foreign_key_changes() {
        let plugin = ExternalDatabasePlugin::new();
        let mut original = TableDesign::new("main", "order_items");
        original.add_column(ColumnDefinition::new("id").data_type("INTEGER"));
        original.add_column(ColumnDefinition::new("order_id").data_type("INTEGER"));
        original.add_column(ColumnDefinition::new("legacy_order_id").data_type("INTEGER"));
        original.foreign_keys.push(ForeignKeyDefinition {
            name: "fk_order_items_legacy".to_string(),
            columns: vec!["legacy_order_id".to_string()],
            ref_table: "orders".to_string(),
            ref_schema: None,
            ref_columns: vec!["id".to_string()],
            on_delete: String::new(),
            on_update: String::new(),
        });

        let mut current = TableDesign::new("main", "order_items");
        current.add_column(ColumnDefinition::new("id").data_type("INTEGER"));
        current.add_column(ColumnDefinition::new("order_id").data_type("INTEGER"));
        current.foreign_keys.push(ForeignKeyDefinition {
            name: "fk_order_items_order".to_string(),
            columns: vec!["order_id".to_string()],
            ref_table: "orders".to_string(),
            ref_schema: None,
            ref_columns: vec!["id".to_string()],
            on_delete: "CASCADE".to_string(),
            on_update: "NO ACTION".to_string(),
        });

        let sql = plugin.build_alter_table_sql(&original, &current);

        assert!(
            sql.contains("ALTER TABLE \"order_items\" DROP CONSTRAINT \"fk_order_items_legacy\";")
        );
        assert!(
            sql.find("DROP CONSTRAINT \"fk_order_items_legacy\"")
                .unwrap()
                < sql.find("DROP COLUMN \"legacy_order_id\"").unwrap()
        );
        assert!(sql.contains(
            "ALTER TABLE \"order_items\" ADD CONSTRAINT \"fk_order_items_order\" FOREIGN KEY (\"order_id\") REFERENCES \"orders\" (\"id\") ON DELETE CASCADE ON UPDATE NO ACTION;"
        ));
    }
}
