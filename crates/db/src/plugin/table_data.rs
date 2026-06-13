use super::DatabasePlugin;
use crate::connection::DbConnection;
use crate::types::{TableDataRequest, TableDataResponse};
use anyhow::Result;
use async_trait::async_trait;

/// Table data query operations.
#[async_trait]
pub trait DatabaseTableDataOps: Send + Sync {
    async fn query_table_data(
        &self,
        connection: &dyn DbConnection,
        request: TableDataRequest,
    ) -> Result<TableDataResponse>;
}

#[async_trait]
impl<T> DatabaseTableDataOps for T
where
    T: DatabasePlugin + ?Sized,
{
    async fn query_table_data(
        &self,
        connection: &dyn DbConnection,
        request: TableDataRequest,
    ) -> Result<TableDataResponse> {
        DatabasePlugin::query_table_data(self, connection, request).await
    }
}
