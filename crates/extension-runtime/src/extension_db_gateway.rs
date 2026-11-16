#![allow(dead_code)]

use async_trait::async_trait;
use db::GlobalDbState;
use extension_component::{
    DbSessionResource, ExtensionDbHost, PermissionSet, SqlAccess,
    protocol::{ConnectionInfo, DbError, ExecuteSqlRequest, OpenSessionRequest, RowBatch},
};

use crate::extension_db_gateway_conversion::sql_results_to_row_batch;

pub struct ExtensionDbGateway {
    extension_id: String,
    permissions: PermissionSet,
    db_state: GlobalDbState,
}

impl ExtensionDbGateway {
    pub fn new(
        extension_id: impl Into<String>,
        permissions: PermissionSet,
        db_state: GlobalDbState,
    ) -> Self {
        Self {
            extension_id: extension_id.into(),
            permissions,
            db_state,
        }
    }

    pub fn list_connections(&self) -> Result<Vec<ConnectionInfo>, DbError> {
        if !self.permissions.allows_connection_list() {
            return Err(DbError::permission_denied("db:connections:list"));
        }
        Ok(self
            .db_state
            .list_connection_summaries()
            .into_iter()
            .map(|connection| ConnectionInfo {
                id: connection.id,
                name: connection.name,
                driver: connection.database_type.as_str().to_string(),
                database: connection.database,
            })
            .collect())
    }

    pub async fn open_session(
        &self,
        request: OpenSessionRequest,
    ) -> Result<DbSessionResource, DbError> {
        self.ensure_db_permission(SqlAccess::Read, &request.connection_id)?;
        let session_id = self
            .db_state
            .create_session_direct(request.connection_id.clone(), request.database)
            .await
            .map_err(|error| DbError::query_failed(error.to_string()))?;
        Ok(DbSessionResource::new(
            self.extension_id.clone(),
            request.connection_id,
            session_id,
        ))
    }

    pub async fn execute(&self, request: ExecuteSqlRequest) -> Result<RowBatch, DbError> {
        self.ensure_db_permission(request.access(), &request.connection_id)?;
        let results = self
            .db_state
            .execute_session(
                request.session_id,
                request.sql,
                Some(db_exec_options(request.options)),
            )
            .await
            .map_err(|error| DbError::query_failed(error.to_string()))?;
        sql_results_to_row_batch(results)
    }

    pub async fn list_databases(&self, connection_id: String) -> Result<Vec<String>, DbError> {
        self.ensure_db_permission(SqlAccess::Schema, &connection_id)?;
        self.db_state
            .list_databases_direct(connection_id)
            .await
            .map_err(|error| DbError::query_failed(error.to_string()))
    }

    pub async fn list_schemas(
        &self,
        connection_id: String,
        database: String,
    ) -> Result<Vec<String>, DbError> {
        self.ensure_db_permission(SqlAccess::Schema, &connection_id)?;
        self.db_state
            .list_schemas_direct(connection_id, database)
            .await
            .map_err(|error| DbError::query_failed(error.to_string()))
    }

    /// 连接配置里的默认库（调用方已通过权限检查后才用）。
    fn configured_database(&self, connection_id: &str) -> Option<String> {
        self.db_state
            .list_connection_summaries()
            .into_iter()
            .find(|summary| summary.id == connection_id)
            .and_then(|summary| summary.database)
    }

    /// 把可选的 `database` 解析成实际库名（缺省回落到连接配置）。
    fn resolve_database(
        &self,
        connection_id: &str,
        database: Option<String>,
    ) -> Result<String, DbError> {
        database
            .or_else(|| self.configured_database(connection_id))
            .ok_or_else(|| {
                DbError::invalid_resource(format!(
                    "连接 {connection_id} 没有默认库，请显式传 database"
                ))
            })
    }

    /// 列出库里的表/视图（只读元数据，需要 `db:schema:<conn>`）。
    pub async fn list_tables(
        &self,
        connection_id: String,
        database: Option<String>,
        schema: Option<String>,
    ) -> Result<Vec<db::types::TableInfo>, DbError> {
        self.ensure_db_permission(SqlAccess::Schema, &connection_id)?;
        let database = self.resolve_database(&connection_id, database)?;
        self.db_state
            .list_tables_direct(&connection_id, &database, schema)
            .await
            .map_err(|error| DbError::query_failed(error.to_string()))
    }

    /// 描述一张表：列、索引、外键（一次会话取全）。
    pub async fn describe_table(
        &self,
        connection_id: String,
        database: Option<String>,
        schema: Option<String>,
        table: String,
    ) -> Result<db::types::DirectTableMetadata, DbError> {
        self.ensure_db_permission(SqlAccess::Schema, &connection_id)?;
        let database = self.resolve_database(&connection_id, database)?;
        self.db_state
            .load_table_metadata_direct(db::types::DirectTableMetadataRequest {
                connection_id,
                database,
                schema,
                table,
                include_table_metadata: true,
            })
            .await
            .map_err(|error| DbError::query_failed(error.to_string()))
    }

    /// 列出库里的函数/过程（只读元数据）。
    pub async fn list_functions(
        &self,
        connection_id: String,
        database: Option<String>,
        schema: Option<String>,
    ) -> Result<Vec<db::types::FunctionInfo>, DbError> {
        self.ensure_db_permission(SqlAccess::Schema, &connection_id)?;
        let database = self.resolve_database(&connection_id, database)?;
        self.db_state
            .list_functions_in_schema_direct(&connection_id, &database, schema)
            .await
            .map_err(|error| DbError::query_failed(error.to_string()))
    }

    pub async fn close_session(&self, session: &mut DbSessionResource) -> Result<(), DbError> {
        if session.extension_id() != self.extension_id {
            return Err(DbError::permission_denied("foreign session resource"));
        }
        if session.is_closed() {
            return Ok(());
        }
        self.db_state
            .close_session_direct(session.session_id())
            .await
            .map_err(|error| DbError::query_failed(error.to_string()))?;
        session.close();
        Ok(())
    }

    fn ensure_session_resource(&self, session: &DbSessionResource) -> Result<(), DbError> {
        if session.extension_id() != self.extension_id {
            return Err(DbError::permission_denied("foreign session resource"));
        }
        if session.is_closed() {
            return Err(DbError::invalid_resource("closed session resource"));
        }
        Ok(())
    }

    fn ensure_db_permission(&self, access: SqlAccess, connection_id: &str) -> Result<(), DbError> {
        if self.permissions.allows_db(access, connection_id) {
            return Ok(());
        }
        Err(DbError::permission_denied(format!(
            "db:{access:?}:{connection_id}"
        )))
    }
}

#[async_trait]
impl ExtensionDbHost for ExtensionDbGateway {
    fn list_connections(&self) -> Result<Vec<ConnectionInfo>, DbError> {
        ExtensionDbGateway::list_connections(self)
    }

    async fn open_session(
        &self,
        request: OpenSessionRequest,
    ) -> Result<DbSessionResource, DbError> {
        ExtensionDbGateway::open_session(self, request).await
    }

    async fn execute(
        &self,
        session: &DbSessionResource,
        sql: String,
        options: extension_component::protocol::ExecOptions,
    ) -> Result<RowBatch, DbError> {
        self.ensure_session_resource(session)?;
        ExtensionDbGateway::execute(
            self,
            ExecuteSqlRequest {
                session_id: session.session_id().to_string(),
                connection_id: session.connection_id().to_string(),
                sql,
                options,
            },
        )
        .await
    }

    async fn list_databases(&self, session: &DbSessionResource) -> Result<Vec<String>, DbError> {
        self.ensure_session_resource(session)?;
        ExtensionDbGateway::list_databases(self, session.connection_id().to_string()).await
    }

    async fn list_schemas(
        &self,
        session: &DbSessionResource,
        database: String,
    ) -> Result<Vec<String>, DbError> {
        self.ensure_session_resource(session)?;
        ExtensionDbGateway::list_schemas(self, session.connection_id().to_string(), database).await
    }

    async fn close_session(&self, session: &mut DbSessionResource) -> Result<(), DbError> {
        ExtensionDbGateway::close_session(self, session).await
    }
}

fn db_exec_options(options: extension_component::protocol::ExecOptions) -> db::ExecOptions {
    db::ExecOptions {
        max_rows: options.max_rows.map(|rows| rows as usize),
        streaming: options.stream,
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "extension_db_gateway_tests.rs"]
mod tests;
