mod schema;

use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, ConnectionType, DbConnectionConfig};
use serde_json::{Value, json};
use std::sync::Arc;
use tool_runtime::{
    ToolAdapter, ToolContext, ToolDescriptor, ToolError, ToolHandler, ToolMode, ToolRegistry,
    ToolResult,
};

use schema::descriptor_parts;

#[derive(Clone, Copy)]
enum DatabaseTool {
    Schema,
    Query,
    Exec,
}

#[derive(Clone)]
struct DatabaseToolHandler {
    repo: Arc<ConnectionRepository>,
    tool: DatabaseTool,
}

pub fn database_tool_registry(repo: Arc<ConnectionRepository>) -> ToolRegistry {
    ToolRegistry::new(vec![
        Arc::new(DatabaseToolHandler::new(repo.clone(), DatabaseTool::Schema)),
        Arc::new(DatabaseToolHandler::new(repo.clone(), DatabaseTool::Query)),
        Arc::new(DatabaseToolHandler::new(repo, DatabaseTool::Exec)),
    ])
}

impl DatabaseToolHandler {
    fn new(repo: Arc<ConnectionRepository>, tool: DatabaseTool) -> Self {
        Self { repo, tool }
    }

    async fn call_tool(&self, input: Value) -> Result<ToolResult, ToolError> {
        match self.tool {
            DatabaseTool::Schema => self.schema(input).await,
            DatabaseTool::Query => self.query(input).await,
            DatabaseTool::Exec => self.exec(input).await,
        }
    }

    async fn schema(&self, input: Value) -> Result<ToolResult, ToolError> {
        let connection = required_str(&input, "connection")?;
        let config = scoped_database_config(self.database_config(&connection)?, &input)?;
        let plugin = db::DbManager::new()
            .get_plugin(&config.database_type)
            .map_err(tool_error)?;
        let mut db_connection = plugin
            .create_connection(config.clone())
            .await
            .map_err(tool_error)?;
        db_connection.connect().await.map_err(tool_error)?;
        let databases = plugin.list_databases(&*db_connection).await;
        let _ = db_connection.disconnect().await;
        let databases = databases.map_err(tool_error)?;
        Ok(ToolResult::structured(json!({
            "connection": connection,
            "database_type": config.database_type,
            "database": config.database,
            "schema": optional_str(&input, "schema")?,
            "databases": databases
        })))
    }

    async fn query(&self, input: Value) -> Result<ToolResult, ToolError> {
        let connection = required_str(&input, "connection")?;
        let sql = required_str(&input, "sql")?;
        let config = scoped_database_config(self.database_config(&connection)?, &input)?;
        let plugin = db::DbManager::new()
            .get_plugin(&config.database_type)
            .map_err(tool_error)?;
        if !plugin.is_query_statement(&sql) {
            return Err(ToolError::Failed {
                message:
                    "db.query only accepts query statements; use db.exec for write-capable SQL"
                        .to_string(),
            });
        }
        let database = config.database.clone();
        let schema = optional_str(&input, "schema")?;
        let results = execute_sql(plugin, config, &sql, db::ExecOptions::default()).await?;
        Ok(ToolResult::structured(json!({
            "connection": connection,
            "database": database,
            "schema": schema,
            "sql": sql,
            "results": results
        })))
    }

    async fn exec(&self, input: Value) -> Result<ToolResult, ToolError> {
        let connection = required_str(&input, "connection")?;
        let sql = exec_sql(&input)?;
        let config = scoped_database_config(self.database_config(&connection)?, &input)?;
        let plugin = db::DbManager::new()
            .get_plugin(&config.database_type)
            .map_err(tool_error)?;
        let database = config.database.clone();
        let schema = optional_str(&input, "schema")?;
        let results = execute_sql(plugin, config, &sql, db::ExecOptions::default()).await?;
        Ok(ToolResult::structured(json!({
            "connection": connection,
            "database": database,
            "schema": schema,
            "results": results
        })))
    }

    fn database_config(&self, connection: &str) -> Result<DbConnectionConfig, ToolError> {
        let stored = find_connection(&self.repo, connection)?;
        if stored.connection_type != ConnectionType::Database {
            return Err(ToolError::Failed {
                message: format!("connection is not database: {connection}"),
            });
        }
        stored.to_db_connection().map_err(tool_error)
    }
}

impl ToolHandler for DatabaseToolHandler {
    fn descriptor(&self) -> ToolDescriptor {
        let (id, title, description, schema, annotations) = descriptor_parts(self.tool);
        ToolDescriptor {
            id: id.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            input_schema: schema,
            output_schema: json!({ "type": "object" }),
            permissions: Vec::new(),
            mode: ToolMode::Deterministic,
            adapters: vec![
                ToolAdapter::Mcp,
                ToolAdapter::FunctionCalling,
                ToolAdapter::Cli,
            ],
            annotations,
        }
    }

    fn call(&self, input: Value, _context: ToolContext) -> tool_runtime::ToolFuture {
        let handler = self.clone();
        Box::pin(async move { handler.call_tool(input).await })
    }
}

async fn execute_sql(
    plugin: Arc<dyn db::DatabasePlugin>,
    config: DbConnectionConfig,
    sql: &str,
    options: db::ExecOptions,
) -> Result<Vec<db::SqlResult>, ToolError> {
    let mut connection = plugin.create_connection(config).await.map_err(tool_error)?;
    connection.connect().await.map_err(tool_error)?;
    let result = connection
        .execute(plugin.as_ref(), sql, options)
        .await
        .map_err(tool_error);
    let _ = connection.disconnect().await;
    result
}

fn find_connection(
    repo: &ConnectionRepository,
    connection: &str,
) -> Result<one_core::storage::StoredConnection, ToolError> {
    if let Ok(id) = connection.parse::<i64>() {
        return repo
            .get(id)
            .map_err(tool_error)?
            .ok_or_else(|| unknown_connection(connection));
    }
    repo.list()
        .map_err(tool_error)?
        .into_iter()
        .find(|stored| stored.name == connection)
        .ok_or_else(|| unknown_connection(connection))
}

fn exec_sql(input: &Value) -> Result<String, ToolError> {
    if let Some(sql) = optional_str(input, "sql")? {
        return Ok(sql);
    }
    let file = required_str(input, "file")?;
    std::fs::read_to_string(&file).map_err(|error| ToolError::Failed {
        message: format!("failed to read SQL file `{file}`: {error}"),
    })
}

fn scoped_database_config(
    mut config: DbConnectionConfig,
    input: &Value,
) -> Result<DbConnectionConfig, ToolError> {
    if let Some(database) = optional_str(input, "database")?.filter(|value| !value.is_empty()) {
        config.database = Some(database);
    }
    Ok(config)
}

fn required_str(input: &Value, key: &str) -> Result<String, ToolError> {
    optional_str(input, key)?.ok_or_else(|| ToolError::Failed {
        message: format!("missing required string field `{key}`"),
    })
}

fn optional_str(input: &Value, key: &str) -> Result<Option<String>, ToolError> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(ToolError::Failed {
            message: format!("field `{key}` must be a string"),
        }),
    }
}

fn unknown_connection(connection: &str) -> ToolError {
    ToolError::Failed {
        message: format!("unknown connection: {connection}"),
    }
}

fn tool_error(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed {
        message: error.to_string(),
    }
}
