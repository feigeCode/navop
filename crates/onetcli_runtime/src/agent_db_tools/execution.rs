use super::context::{OpenedDb, require_database, resolve_db_target};
use super::schema::{self, DEFAULT_QUERY_MAX_ROWS};
use super::{AgentDbTool, ToolError};
use agent_runtime::{
    ToolObservation,
    tools::{ObservationData, ToolInvocation},
};
use db::{ExecOptions, SqlResult};
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionType, DbConnectionConfig};
use serde_json::{Value, json};

impl AgentDbTool {
    pub(super) async fn query(
        &self,
        invocation: ToolInvocation,
    ) -> Result<ToolObservation, ToolError> {
        let sql = required_str(&invocation.arguments, "sql")?;
        let max_rows = schema::bounded_query_rows(&invocation.arguments, "max_rows")?;
        let mut opened = self.open_target(&invocation).await?;
        if !opened.plugin.is_query_statement(&sql) {
            return Err(ToolError::Execution(
                "db_query only accepts read-only query statements; use db_execute_sql for mutating SQL with approval".into(),
            ));
        }
        let results = execute_sql(&mut opened, &sql, Some(max_rows)).await?;
        let value = query_output(&opened, sql, results);
        Ok(success_json(invocation, "database query executed", value))
    }

    pub(super) async fn execute_sql(
        &self,
        invocation: ToolInvocation,
    ) -> Result<ToolObservation, ToolError> {
        let sql = required_str(&invocation.arguments, "sql")?;
        let mut opened = self.open_target(&invocation).await?;
        let results = execute_sql(&mut opened, &sql, Some(DEFAULT_QUERY_MAX_ROWS)).await?;
        let value = query_output(&opened, sql, results);
        Ok(success_json(invocation, "database SQL executed", value))
    }

    pub(super) async fn list_databases(
        &self,
        invocation: ToolInvocation,
    ) -> Result<ToolObservation, ToolError> {
        let mut opened = self.open_target(&invocation).await?;
        let databases = opened
            .plugin
            .list_databases(&*opened.connection)
            .await
            .map_err(tool_error)?;
        let _ = opened.connection.disconnect().await;
        Ok(success_json(
            invocation,
            "database list loaded",
            json!({"connection": opened.connection_id, "databases": databases}),
        ))
    }

    pub(super) async fn list_tables(
        &self,
        invocation: ToolInvocation,
    ) -> Result<ToolObservation, ToolError> {
        let mut opened = self.open_target(&invocation).await?;
        let database = require_database(&opened)?;
        let schema = opened.config_schema();
        let tables = opened
            .plugin
            .list_tables(&*opened.connection, &database, schema.clone())
            .await
            .map_err(tool_error)?;
        let _ = opened.connection.disconnect().await;
        Ok(success_json(
            invocation,
            "database tables loaded",
            json!({
                "connection": opened.connection_id,
                "database": database,
                "schema": schema,
                "tables": tables,
            }),
        ))
    }

    pub(super) async fn describe_table(
        &self,
        invocation: ToolInvocation,
    ) -> Result<ToolObservation, ToolError> {
        let table = required_str(&invocation.arguments, "table")?;
        let mut opened = self.open_target(&invocation).await?;
        let database = require_database(&opened)?;
        let schema = opened.config_schema();
        let columns = opened
            .plugin
            .list_columns(&*opened.connection, &database, schema.clone(), &table)
            .await
            .map_err(tool_error)?;
        let indexes = opened
            .plugin
            .list_indexes(&*opened.connection, &database, schema.clone(), &table)
            .await
            .unwrap_or_default();
        let foreign_keys = opened
            .plugin
            .list_foreign_keys(&*opened.connection, &database, schema.clone(), &table)
            .await
            .unwrap_or_default();
        let _ = opened.connection.disconnect().await;
        Ok(success_json(
            invocation,
            "database table described",
            json!({
                "connection": opened.connection_id,
                "database": database,
                "schema": schema,
                "table": table,
                "columns": columns,
                "indexes": indexes,
                "foreign_keys": foreign_keys,
            }),
        ))
    }

    pub(super) async fn sample_rows(
        &self,
        invocation: ToolInvocation,
    ) -> Result<ToolObservation, ToolError> {
        let table = required_str(&invocation.arguments, "table")?;
        let limit = schema::bounded_sample_rows(&invocation.arguments, "limit")?;
        let mut opened = self.open_target(&invocation).await?;
        let database = require_database(&opened)?;
        let schema = opened.config_schema();
        let table_ref = opened
            .plugin
            .format_table_reference(&database, schema.as_deref(), &table);
        let sql = format!("SELECT * FROM {table_ref}");
        let results = execute_sql(&mut opened, &sql, Some(limit)).await?;
        let value = query_output(&opened, sql, results);
        Ok(success_json(
            invocation,
            "database sample rows loaded",
            value,
        ))
    }

    async fn open_target(&self, invocation: &ToolInvocation) -> Result<OpenedDb, ToolError> {
        let target = resolve_db_target(invocation)?;
        let mut config = self.database_config(&target.connection_id)?;
        if let Some(database) = target.database {
            config.database = Some(database);
        }
        if let Some(schema) = target.schema {
            config.extra_params.insert("schema".to_string(), schema);
        }
        let plugin = db::DbManager::new()
            .get_plugin(&config.database_type)
            .map_err(tool_error)?;
        let mut connection = plugin
            .create_connection(config.clone())
            .await
            .map_err(tool_error)?;
        connection.connect().await.map_err(tool_error)?;
        Ok(OpenedDb {
            connection_id: target.connection_id,
            config,
            plugin,
            connection,
        })
    }

    fn database_config(&self, connection_id: &str) -> Result<DbConnectionConfig, ToolError> {
        let id = connection_id.parse::<i64>().map_err(|_| {
            ToolError::MissingResource(format!(
                "Agent database tools require a database connection id, got `{connection_id}`"
            ))
        })?;
        let stored = self
            .repo
            .get(id)
            .map_err(tool_error)?
            .ok_or_else(|| ToolError::MissingResource(format!("unknown connection: {id}")))?;
        if stored.connection_type != ConnectionType::Database {
            return Err(ToolError::MissingResource(format!(
                "current Agent resource is not a database connection: {id}"
            )));
        }
        stored.to_db_connection().map_err(tool_error)
    }
}

async fn execute_sql(
    opened: &mut OpenedDb,
    sql: &str,
    max_rows: Option<usize>,
) -> Result<Vec<SqlResult>, ToolError> {
    let results = opened
        .connection
        .execute(
            opened.plugin.as_ref(),
            sql,
            ExecOptions {
                max_rows,
                ..ExecOptions::default()
            },
        )
        .await
        .map_err(tool_error)?;
    let _ = opened.connection.disconnect().await;
    Ok(results)
}

fn query_output(opened: &OpenedDb, sql: String, results: Vec<SqlResult>) -> Value {
    json!({
        "connection": opened.connection_id,
        "database": opened.config.database,
        "schema": opened.config_schema(),
        "sql": sql,
        "results": results,
    })
}

fn success_json(
    invocation: ToolInvocation,
    summary: impl Into<String>,
    value: Value,
) -> ToolObservation {
    ToolObservation::success(
        invocation.call_id,
        invocation.tool_name,
        summary,
        ObservationData::Json(value),
    )
}

fn required_str(input: &Value, key: &str) -> Result<String, ToolError> {
    schema::optional_str(input, key)?.ok_or_else(|| {
        ToolError::InvalidArguments(format!("missing required string field `{key}`"))
    })
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Execution(error.to_string())
}
