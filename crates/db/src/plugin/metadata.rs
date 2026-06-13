use super::DatabasePlugin;
use crate::connection::{DbConnection, DbError};
use crate::types::{
    CheckInfo, ColumnInfo, DatabaseInfo, ForeignKeyDefinition, FunctionInfo, IndexInfo, ObjectView,
    SequenceInfo, TableInfo, TriggerInfo, ViewInfo,
};
use anyhow::Result;
use async_trait::async_trait;
use one_core::storage::DbConnectionConfig;

/// Asynchronous database metadata and connection operations.
#[async_trait]
pub trait DatabaseMetadataOps: Send + Sync {
    async fn create_connection(
        &self,
        config: DbConnectionConfig,
    ) -> Result<Box<dyn DbConnection + Send + Sync>, DbError>;
    async fn test_connection(&self, config: DbConnectionConfig) -> Result<(), DbError>;
    async fn list_databases(&self, connection: &dyn DbConnection) -> Result<Vec<String>>;
    async fn list_databases_view(&self, connection: &dyn DbConnection) -> Result<ObjectView>;
    async fn list_databases_detailed(
        &self,
        connection: &dyn DbConnection,
    ) -> Result<Vec<DatabaseInfo>>;
    async fn list_schemas(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<String>>;
    async fn list_schemas_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView>;
    async fn list_tables(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<TableInfo>>;
    async fn list_tables_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<ObjectView>;
    async fn list_columns(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<ColumnInfo>>;
    async fn list_columns_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<ObjectView>;
    async fn list_indexes(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<IndexInfo>>;
    async fn list_indexes_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<ObjectView>;
    async fn list_foreign_keys(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<ForeignKeyDefinition>>;
    async fn list_table_triggers(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<TriggerInfo>>;
    async fn list_table_checks(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<CheckInfo>>;
    async fn list_views(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<ViewInfo>>;
    async fn list_views_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView>;
    async fn list_functions(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>>;
    async fn list_functions_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView>;
    async fn list_procedures(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>>;
    async fn list_procedures_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView>;
    async fn list_triggers(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<TriggerInfo>>;
    async fn list_triggers_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView>;
    async fn list_sequences(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<SequenceInfo>>;
    async fn list_sequences_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView>;
}

#[async_trait]
impl<T> DatabaseMetadataOps for T
where
    T: DatabasePlugin + ?Sized,
{
    async fn create_connection(
        &self,
        config: DbConnectionConfig,
    ) -> Result<Box<dyn DbConnection + Send + Sync>, DbError> {
        DatabasePlugin::create_connection(self, config).await
    }

    async fn test_connection(&self, config: DbConnectionConfig) -> Result<(), DbError> {
        DatabasePlugin::test_connection(self, config).await
    }

    async fn list_databases(&self, connection: &dyn DbConnection) -> Result<Vec<String>> {
        DatabasePlugin::list_databases(self, connection).await
    }

    async fn list_databases_view(&self, connection: &dyn DbConnection) -> Result<ObjectView> {
        DatabasePlugin::list_databases_view(self, connection).await
    }

    async fn list_databases_detailed(
        &self,
        connection: &dyn DbConnection,
    ) -> Result<Vec<DatabaseInfo>> {
        DatabasePlugin::list_databases_detailed(self, connection).await
    }

    async fn list_schemas(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<String>> {
        DatabasePlugin::list_schemas(self, connection, database).await
    }

    async fn list_schemas_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        DatabasePlugin::list_schemas_view(self, connection, database).await
    }

    async fn list_tables(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<TableInfo>> {
        DatabasePlugin::list_tables(self, connection, database, schema).await
    }

    async fn list_tables_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<ObjectView> {
        DatabasePlugin::list_tables_view(self, connection, database, schema).await
    }

    async fn list_columns(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<ColumnInfo>> {
        DatabasePlugin::list_columns(self, connection, database, schema, table).await
    }

    async fn list_columns_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<ObjectView> {
        DatabasePlugin::list_columns_view(self, connection, database, schema, table).await
    }

    async fn list_indexes(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<IndexInfo>> {
        DatabasePlugin::list_indexes(self, connection, database, schema, table).await
    }

    async fn list_indexes_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<ObjectView> {
        DatabasePlugin::list_indexes_view(self, connection, database, schema, table).await
    }

    async fn list_foreign_keys(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<ForeignKeyDefinition>> {
        DatabasePlugin::list_foreign_keys(self, connection, database, schema, table).await
    }

    async fn list_table_triggers(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<TriggerInfo>> {
        DatabasePlugin::list_table_triggers(self, connection, database, schema, table).await
    }

    async fn list_table_checks(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
        table: &str,
    ) -> Result<Vec<CheckInfo>> {
        DatabasePlugin::list_table_checks(self, connection, database, schema, table).await
    }

    async fn list_views(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<ViewInfo>> {
        DatabasePlugin::list_views(self, connection, database, schema).await
    }

    async fn list_views_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        DatabasePlugin::list_views_view(self, connection, database).await
    }

    async fn list_functions(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>> {
        DatabasePlugin::list_functions(self, connection, database).await
    }

    async fn list_functions_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        DatabasePlugin::list_functions_view(self, connection, database).await
    }

    async fn list_procedures(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>> {
        DatabasePlugin::list_procedures(self, connection, database).await
    }

    async fn list_procedures_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        DatabasePlugin::list_procedures_view(self, connection, database).await
    }

    async fn list_triggers(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<TriggerInfo>> {
        DatabasePlugin::list_triggers(self, connection, database).await
    }

    async fn list_triggers_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        DatabasePlugin::list_triggers_view(self, connection, database).await
    }

    async fn list_sequences(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<SequenceInfo>> {
        DatabasePlugin::list_sequences(self, connection, database, schema).await
    }

    async fn list_sequences_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView> {
        DatabasePlugin::list_sequences_view(self, connection, database).await
    }
}
