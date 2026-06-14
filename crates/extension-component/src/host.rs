use async_trait::async_trait;

use crate::{
    DbSessionResource,
    protocol::{ConnectionInfo, DbError, ExecOptions, OpenSessionRequest, RowBatch},
};

#[async_trait]
pub trait ExtensionDbHost: Send + Sync {
    fn list_connections(&self) -> Result<Vec<ConnectionInfo>, DbError>;

    async fn open_session(&self, request: OpenSessionRequest)
    -> Result<DbSessionResource, DbError>;

    async fn execute(
        &self,
        session: &DbSessionResource,
        sql: String,
        options: ExecOptions,
    ) -> Result<RowBatch, DbError>;

    async fn list_databases(&self, session: &DbSessionResource) -> Result<Vec<String>, DbError>;

    async fn list_schemas(
        &self,
        session: &DbSessionResource,
        database: String,
    ) -> Result<Vec<String>, DbError>;

    async fn close_session(&self, session: &mut DbSessionResource) -> Result<(), DbError>;
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub struct MockDbHost;

    #[async_trait]
    impl ExtensionDbHost for MockDbHost {
        fn list_connections(&self) -> Result<Vec<ConnectionInfo>, DbError> {
            Ok(Vec::new())
        }

        async fn open_session(
            &self,
            request: OpenSessionRequest,
        ) -> Result<DbSessionResource, DbError> {
            Ok(DbSessionResource::new(
                "ext",
                request.connection_id,
                "session-1",
            ))
        }

        async fn execute(
            &self,
            _session: &DbSessionResource,
            _sql: String,
            _options: ExecOptions,
        ) -> Result<RowBatch, DbError> {
            Ok(RowBatch {
                columns: Vec::new(),
                rows: Vec::new(),
                next_cursor: None,
            })
        }

        async fn list_databases(
            &self,
            _session: &DbSessionResource,
        ) -> Result<Vec<String>, DbError> {
            Ok(Vec::new())
        }

        async fn list_schemas(
            &self,
            _session: &DbSessionResource,
            _database: String,
        ) -> Result<Vec<String>, DbError> {
            Ok(Vec::new())
        }

        async fn close_session(&self, session: &mut DbSessionResource) -> Result<(), DbError> {
            session.close();
            Ok(())
        }
    }

    #[test]
    fn db_host_trait_is_resource_shaped() {
        fn assert_host<T: ExtensionDbHost>() {}

        assert_host::<MockDbHost>();
    }
}
