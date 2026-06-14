#![allow(dead_code)]

use async_trait::async_trait;
use db::{ExecResult, GlobalDbState, QueryResult, SqlResult};
use extension_component::{
    DbSessionResource, ExtensionDbHost, PermissionSet, SqlAccess,
    protocol::{
        Column, ConnectionInfo, DbError, DbValue, ExecuteSqlRequest, OpenSessionRequest, RowBatch,
    },
};

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
        Ok(sql_results_to_row_batch(results))
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

    pub async fn close_session(&self, session: &mut DbSessionResource) -> Result<(), DbError> {
        if session.extension_id() != self.extension_id {
            return Err(DbError::permission_denied("foreign session resource"));
        }
        if session.is_closed() {
            return Ok(());
        }
        self.db_state
            .connection_manager
            .close_session(session.session_id())
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

fn sql_results_to_row_batch(results: Vec<SqlResult>) -> RowBatch {
    if let Some(query) = results.iter().find_map(|result| match result {
        SqlResult::Query(query) => Some(query),
        _ => None,
    }) {
        return query_to_row_batch(query);
    }
    exec_results_to_row_batch(results)
}

fn query_to_row_batch(query: &QueryResult) -> RowBatch {
    let columns = query
        .columns
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let meta = query.column_meta.get(index);
            Column {
                name: name.clone(),
                type_name: meta.map(|meta| meta.db_type.clone()).unwrap_or_default(),
                nullable: meta.map(|meta| meta.nullable).unwrap_or(true),
            }
        })
        .collect();
    let rows = query
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| match cell {
                    Some(value) => DbValue::Text(value.clone()),
                    None => DbValue::Null,
                })
                .collect()
        })
        .collect();
    RowBatch {
        columns,
        rows,
        next_cursor: None,
    }
}

fn exec_results_to_row_batch(results: Vec<SqlResult>) -> RowBatch {
    let rows = results
        .into_iter()
        .filter_map(|result| match result {
            SqlResult::Exec(exec) => Some(exec_to_row(exec)),
            SqlResult::Error(error) => Some(vec![
                DbValue::Text(error.sql),
                DbValue::Text("error".to_string()),
                DbValue::Text(error.message),
            ]),
            SqlResult::Query(_) => None,
        })
        .collect();
    RowBatch {
        columns: vec![
            Column {
                name: "sql".to_string(),
                type_name: "text".to_string(),
                nullable: false,
            },
            Column {
                name: "status".to_string(),
                type_name: "text".to_string(),
                nullable: false,
            },
            Column {
                name: "message".to_string(),
                type_name: "text".to_string(),
                nullable: true,
            },
        ],
        rows,
        next_cursor: None,
    }
}

fn exec_to_row(exec: ExecResult) -> Vec<DbValue> {
    vec![
        DbValue::Text(exec.sql),
        DbValue::Text("ok".to_string()),
        DbValue::Text(
            exec.message
                .unwrap_or_else(|| format!("{} row(s) affected", exec.rows_affected)),
        ),
    ]
}

#[cfg(test)]
#[path = "extension_db_gateway_tests.rs"]
mod tests;
