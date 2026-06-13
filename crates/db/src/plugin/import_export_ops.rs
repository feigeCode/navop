use super::DatabasePlugin;
use crate::connection::DbConnection;
use crate::import_export::{
    ExportConfig, ExportProgressSender, ExportResult, ImportConfig, ImportProgressSender,
    ImportResult,
};
use anyhow::Result;
use async_trait::async_trait;

/// Data import/export operations and SQL dump helpers.
#[async_trait]
pub trait DatabaseImportExportOps: Send + Sync {
    async fn export_table_create_sql(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<String>;
    async fn export_table_data_sql(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<&str>,
        table: &str,
        where_clause: Option<&str>,
        limit: Option<usize>,
    ) -> Result<String>;
    fn build_insert_statement(&self, table: &str, columns: &[String], values: &[String]) -> String;
    fn escape_sql_value(&self, value: &str) -> String;
    async fn import_data(
        &self,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
    ) -> Result<ImportResult>;
    async fn import_data_with_progress(
        &self,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
        file_name: &str,
        progress_tx: Option<ImportProgressSender>,
    ) -> Result<ImportResult>;
    async fn export_data(
        &self,
        connection: &dyn DbConnection,
        config: &ExportConfig,
    ) -> Result<ExportResult>;
    async fn export_data_with_progress(
        &self,
        connection: &dyn DbConnection,
        config: &ExportConfig,
        progress_tx: Option<ExportProgressSender>,
    ) -> Result<ExportResult>;
}

#[async_trait]
impl<T> DatabaseImportExportOps for T
where
    T: DatabasePlugin + ?Sized,
{
    async fn export_table_create_sql(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<String> {
        DatabasePlugin::export_table_create_sql(self, connection, database, schema, table).await
    }

    async fn export_table_data_sql(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<&str>,
        table: &str,
        where_clause: Option<&str>,
        limit: Option<usize>,
    ) -> Result<String> {
        DatabasePlugin::export_table_data_sql(
            self,
            connection,
            database,
            schema,
            table,
            where_clause,
            limit,
        )
        .await
    }

    fn build_insert_statement(&self, table: &str, columns: &[String], values: &[String]) -> String {
        DatabasePlugin::build_insert_statement(self, table, columns, values)
    }

    fn escape_sql_value(&self, value: &str) -> String {
        DatabasePlugin::escape_sql_value(self, value)
    }

    async fn import_data(
        &self,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
    ) -> Result<ImportResult> {
        DatabasePlugin::import_data(self, connection, config, data).await
    }

    async fn import_data_with_progress(
        &self,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
        file_name: &str,
        progress_tx: Option<ImportProgressSender>,
    ) -> Result<ImportResult> {
        DatabasePlugin::import_data_with_progress(
            self,
            connection,
            config,
            data,
            file_name,
            progress_tx,
        )
        .await
    }

    async fn export_data(
        &self,
        connection: &dyn DbConnection,
        config: &ExportConfig,
    ) -> Result<ExportResult> {
        DatabasePlugin::export_data(self, connection, config).await
    }

    async fn export_data_with_progress(
        &self,
        connection: &dyn DbConnection,
        config: &ExportConfig,
        progress_tx: Option<ExportProgressSender>,
    ) -> Result<ExportResult> {
        DatabasePlugin::export_data_with_progress(self, connection, config, progress_tx).await
    }
}
