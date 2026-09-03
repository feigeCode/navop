use crate::QueryResult;
use crate::SqlFormatOptions;
use crate::connection::{DbConnection, DbError};
use crate::executor::{SqlResult, SqlSource, StatementType};
use crate::import_export::{
    DataFormat, ExportConfig, ExportProgressSender, ExportResult, FormatHandler, ImportConfig,
    ImportProgressSender, ImportResult,
    formats::{
        CsvFormatHandler, JsonFormatHandler, SqlFormatHandler, TxtFormatHandler, XmlFormatHandler,
    },
};
use crate::max_rows::apply_query_max_rows;
use crate::plugin_manifest::{
    DatabaseCapabilities, DatabaseUiCapabilities, DatabaseUiManifest, FormSelectOption,
    ReferenceDataKind,
};
use crate::streaming_parser::StreamingSqlParser;
use crate::types::*;
use anyhow::{Error, Result, anyhow, bail};
use async_trait::async_trait;
use one_core::storage::{
    DatabaseType, DbConnectionConfig, QueryDirectoryEntryKind, QueryDirectoryScope,
    added_query_directories, default_query_directory, list_query_directory,
    query_directory_display_name,
};
use rust_i18n::t;
use sqlparser::ast;
use sqlparser::ast::{Expr, SetExpr, Statement, TableFactor};
use sqlparser::dialect::Dialect;
use sqlparser::parser::Parser;
use std::collections::HashMap;
use std::io;
use tracing::log::error;

/// Capabilities inferred from a SELECT statement.
///
/// `editable` answers whether the result can safely be written back to the
/// source table. `schema_metadata_safe` is deliberately independent: a
/// read-only query such as `SELECT DISTINCT body FROM articles` can still
/// expose a result projection whose types can be reconciled with the source
/// table schema.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectQueryAnalysis {
    pub table_name: Option<String>,
    pub editable: bool,
    pub schema_metadata_safe: bool,
}

pub(crate) fn parse_table_data_total_count(result: SqlResult) -> Result<usize> {
    let query_result = match result {
        SqlResult::Query(query_result) => query_result,
        SqlResult::Exec(_) => bail!("table row count query returned an execution result"),
        SqlResult::Error(error) => bail!(error.message),
    };
    let value = query_result
        .rows
        .first()
        .and_then(|row| row.first())
        .ok_or_else(|| anyhow!("table row count query returned no scalar value"))?
        .as_deref()
        .ok_or_else(|| anyhow!("table row count query returned NULL"))?;
    value
        .trim()
        .parse::<usize>()
        .map_err(|error| anyhow!("invalid table row count `{value}`: {error}"))
}

/// A complete paginated query together with any result columns used only to implement pagination.
///
/// Most databases only append a pagination clause and therefore have no hidden columns. Databases
/// such as Oracle 11g need to wrap the base query and expose an internal `ROWNUM` column while
/// applying an offset. Callers must pass the returned [`QueryResult`] through
/// [`PaginatedQuery::strip_hidden_result_columns`] before displaying or exporting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaginatedQuery {
    pub sql: String,
    hidden_result_columns: Vec<String>,
}

impl PaginatedQuery {
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            hidden_result_columns: Vec::new(),
        }
    }

    pub fn with_hidden_result_column(mut self, column: impl Into<String>) -> Self {
        self.hidden_result_columns.push(column.into());
        self
    }

    pub fn strip_hidden_result_columns(&self, query_result: &mut QueryResult) -> Result<()> {
        if self.hidden_result_columns.is_empty() {
            return Ok(());
        }

        let column_count = query_result.columns.len();
        anyhow::ensure!(
            query_result.column_meta.is_empty() || query_result.column_meta.len() == column_count,
            "pagination result column metadata is inconsistent"
        );
        for row in &query_result.rows {
            anyhow::ensure!(
                row.len() == column_count,
                "pagination result row has an inconsistent column count"
            );
        }

        for hidden_column in self.hidden_result_columns.iter().rev() {
            let column_index = query_result
                .columns
                .iter()
                .rposition(|column| column.eq_ignore_ascii_case(hidden_column))
                .ok_or_else(|| anyhow!("pagination result column `{hidden_column}` is missing"))?;

            query_result.columns.remove(column_index);
            if !query_result.column_meta.is_empty() {
                query_result.column_meta.remove(column_index);
            }
            for row in &mut query_result.rows {
                row.remove(column_index);
            }
            query_result
                .binary_cells
                .retain(|cell| cell.column_index != column_index);
            for cell in &mut query_result.binary_cells {
                if cell.column_index > column_index {
                    cell.column_index -= 1;
                }
            }
        }

        Ok(())
    }
}

/// Standard SQL functions common to most databases
pub const STANDARD_SQL_FUNCTIONS: &[(&str, &str)] = &[
    // String functions
    ("CONCAT(str1, str2, ...)", "Concatenate strings"),
    ("SUBSTRING(str, pos, len)", "Extract substring"),
    ("LENGTH(str)", "String length"),
    ("UPPER(str)", "Convert to uppercase"),
    ("LOWER(str)", "Convert to lowercase"),
    ("TRIM(str)", "Remove leading/trailing spaces"),
    ("LTRIM(str)", "Remove leading spaces"),
    ("RTRIM(str)", "Remove trailing spaces"),
    ("REPLACE(str, from, to)", "Replace occurrences"),
    ("REVERSE(str)", "Reverse string"),
    ("LEFT(str, len)", "Left substring"),
    ("RIGHT(str, len)", "Right substring"),
    // Numeric functions
    ("ABS(x)", "Absolute value"),
    ("CEIL(x)", "Round up"),
    ("FLOOR(x)", "Round down"),
    ("ROUND(x, d)", "Round to decimal places"),
    ("MOD(x, y)", "Modulo operation"),
    ("POWER(x, y)", "Power function"),
    ("SQRT(x)", "Square root"),
    ("SIGN(x)", "Sign of number (-1, 0, 1)"),
    // Date/Time functions
    ("NOW()", "Current date and time"),
    ("CURRENT_DATE", "Current date"),
    ("CURRENT_TIME", "Current time"),
    ("CURRENT_TIMESTAMP", "Current timestamp"),
    // Aggregate functions
    ("COUNT(*)", "Count rows"),
    ("COUNT(DISTINCT col)", "Count distinct values"),
    ("SUM(col)", "Sum of values"),
    ("AVG(col)", "Average value"),
    ("MIN(col)", "Minimum value"),
    ("MAX(col)", "Maximum value"),
    // Control flow
    ("COALESCE(val1, val2, ...)", "First non-NULL value"),
    ("NULLIF(val1, val2)", "Return NULL if equal"),
    ("CASE WHEN ... THEN ... END", "Case expression"),
    // Type conversion
    ("CAST(expr AS type)", "Type conversion"),
];

/// Standard SQL keywords common to most databases
pub const STANDARD_SQL_KEYWORDS: &[(&str, &str)] = &[
    ("IF EXISTS", "Conditional existence check"),
    ("IF NOT EXISTS", "Conditional non-existence check"),
];

/// SQL completion information for a specific database type
#[derive(Clone, Default)]
pub struct SqlCompletionInfo {
    /// Database-specific keywords (e.g., LIMIT for MySQL, FETCH for PostgreSQL)
    pub keywords: Vec<(&'static str, &'static str)>,
    /// Database-specific functions with documentation
    pub functions: Vec<(&'static str, &'static str)>,
    /// Database-specific operators
    pub operators: Vec<(&'static str, &'static str)>,
    /// Database-specific data types for CREATE TABLE etc.
    pub data_types: Vec<(&'static str, &'static str)>,
    /// Database-specific snippets (e.g., common query patterns)
    pub snippets: Vec<(&'static str, &'static str, &'static str)>, // (label, insert_text, doc)
}

/// Database operation request
#[derive(Clone, Debug)]
pub struct DatabaseOperationRequest {
    pub database_name: String,
    pub field_values: HashMap<String, String>,
}

/// Database user operation request.
#[derive(Clone, Debug)]
pub struct DatabaseUserOperationRequest {
    pub user_name: String,
    pub host: Option<String>,
    pub database: Option<String>,
    pub field_values: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectionLifecycle {
    pub close_on_release: bool,
    pub physical_open_lock_key: Option<String>,
}

impl ConnectionLifecycle {
    pub fn single_file(
        driver_id: &str,
        config: &DbConnectionConfig,
        path_fields: &[String],
    ) -> Self {
        let path = first_config_value(config, path_fields)
            .or_else(|| first_config_value(config, &default_file_path_fields()))
            .unwrap_or(config.id.as_str());

        Self {
            close_on_release: true,
            physical_open_lock_key: Some(format!("{driver_id}:{}", normalize_file_lock_path(path))),
        }
    }
}

fn default_file_path_fields() -> Vec<String> {
    vec![
        "host".to_string(),
        "database".to_string(),
        "extra_params.path".to_string(),
    ]
}

fn first_config_value<'a>(config: &'a DbConnectionConfig, fields: &[String]) -> Option<&'a str> {
    fields
        .iter()
        .filter_map(|field| config_value_for_field(config, field))
        .map(str::trim)
        .find(|value| !value.is_empty())
}

fn config_value_for_field<'a>(config: &'a DbConnectionConfig, field: &str) -> Option<&'a str> {
    match field {
        "host" => Some(config.host.as_str()),
        "database" => config.database.as_deref(),
        other => other
            .strip_prefix("extra_params.")
            .and_then(|key| config.extra_params.get(key).map(String::as_str)),
    }
}

fn normalize_file_lock_path(path: &str) -> &str {
    path.strip_prefix("file:").unwrap_or(path)
}

impl SqlCompletionInfo {
    /// Create completion info with standard SQL functions and keywords included
    pub fn with_standard_sql(mut self) -> Self {
        // Prepend standard functions
        let mut all_functions = STANDARD_SQL_FUNCTIONS.to_vec();
        all_functions.extend(self.functions);
        self.functions = all_functions;

        // Prepend standard keywords
        let mut all_keywords = STANDARD_SQL_KEYWORDS.to_vec();
        all_keywords.extend(self.keywords);
        self.keywords = all_keywords;

        self
    }
}

fn routine_node(
    routine: FunctionInfo,
    node_type: DbNodeType,
    id_prefix: &str,
    parent_id: &str,
    connection_id: &str,
    database_type: DatabaseType,
    base_metadata: &HashMap<String, String>,
) -> DbNode {
    let display_name = match routine.identity_arguments.as_deref() {
        Some(arguments) => format!("{}({})", routine.name, arguments),
        None => routine.name.clone(),
    };
    let id_component = if let Some(object_id) = routine.object_id.as_deref() {
        format!("{}#oid:{}", routine.name, object_id)
    } else if let Some(arguments) = routine.identity_arguments.as_deref() {
        format!("{}({})", routine.name, arguments)
    } else {
        routine.name.clone()
    };

    let mut metadata = base_metadata.clone();
    if let Some(schema) = routine
        .schema
        .as_ref()
        .filter(|schema| !schema.trim().is_empty())
    {
        metadata.insert("schema".to_string(), schema.clone());
    }
    metadata.insert(ROUTINE_NAME_METADATA_KEY.to_string(), routine.name);
    if let Some(arguments) = routine.identity_arguments {
        metadata.insert(
            ROUTINE_IDENTITY_ARGUMENTS_METADATA_KEY.to_string(),
            arguments,
        );
    }
    if let Some(object_id) = routine.object_id {
        metadata.insert(ROUTINE_OBJECT_ID_METADATA_KEY.to_string(), object_id);
    }

    DbNode::new(
        format!("{}:{}", id_prefix, id_component),
        display_name,
        node_type,
        connection_id.to_string(),
        database_type,
    )
    .with_parent_context(parent_id)
    .with_metadata(metadata)
}

/// Database plugin trait for supporting multiple database types
#[async_trait]
pub trait DatabasePlugin: Send + Sync {
    fn name(&self) -> DatabaseType;

    /// Quote an identifier (table name, column name, etc.) according to database syntax
    fn quote_identifier(&self, identifier: &str) -> String;

    /// Get database-specific SQL completion information
    fn get_completion_info(&self) -> SqlCompletionInfo {
        SqlCompletionInfo::default()
    }

    async fn create_connection(
        &self,
        config: DbConnectionConfig,
    ) -> Result<Box<dyn DbConnection + Send + Sync>, DbError>;

    fn connection_lifecycle(&self, _config: &DbConnectionConfig) -> ConnectionLifecycle {
        ConnectionLifecycle::default()
    }

    async fn test_connection(&self, config: DbConnectionConfig) -> Result<(), DbError> {
        let mut connection = self.create_connection(config).await?;
        let ping_result = connection.ping().await;
        let _ = connection.disconnect().await;
        ping_result
    }

    // === Database/Schema Level Operations ===
    async fn list_databases(&self, connection: &dyn DbConnection) -> Result<Vec<String>>;

    async fn list_databases_view(&self, connection: &dyn DbConnection) -> Result<ObjectView>;
    async fn list_databases_detailed(
        &self,
        connection: &dyn DbConnection,
    ) -> Result<Vec<DatabaseInfo>>;

    /// Whether this database supports rowid for row identification (e.g., Oracle, SQLite)
    fn supports_rowid(&self) -> bool {
        false
    }

    /// Get the rowid column name for this database
    fn rowid_column_name(&self) -> &'static str {
        "rowid"
    }

    /// Get the alias used for the synthetic rowid projection in table-data
    /// queries. External drivers may override this when their manifest uses a
    /// custom alias; compare consumers must use the same value when removing
    /// the projection from query results.
    fn rowid_column_alias(&self) -> &str {
        "__rowid__"
    }

    /// Get the SQL dialect for this database type
    fn sql_dialect(&self) -> Box<dyn Dialect>;

    /// 创建 SQL 解析器（统一接口，支持脚本和文件）
    fn create_parser(&self, source: SqlSource) -> io::Result<StreamingSqlParser> {
        StreamingSqlParser::from_source(source, self.name())
    }

    /// Format SQL for display (each database can customize this)
    fn format_sql(&self, sql: &str) -> String {
        self.format_sql_with_options(sql, SqlFormatOptions::default())
    }

    /// Format SQL with user-configurable options (each database can customize this)
    fn format_sql_with_options(&self, sql: &str, options: SqlFormatOptions) -> String {
        crate::format_sql_with_options(sql, options)
    }

    /// Check if a SQL statement is a query (returns rows)
    fn is_query_statement(&self, sql: &str) -> bool {
        if let Ok(statements) = Parser::parse_sql(self.sql_dialect().as_ref(), sql) {
            if let Some(stmt) = statements.first() {
                return is_query_stmt(stmt);
            }
        }
        is_query_statement_fallback(sql)
    }

    fn apply_query_max_rows(&self, sql: &str, max_rows: Option<usize>) -> String {
        let Some(max_rows) = max_rows.filter(|rows| *rows > 0) else {
            return sql.to_string();
        };
        apply_query_max_rows(&self.name(), sql, max_rows)
    }

    /// Split SQL text into statements using the database-specific parser.
    fn split_sql_statements(&self, sql: &str) -> Vec<String> {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        let Ok(parser) = self.create_parser(SqlSource::Script(trimmed.to_string())) else {
            return vec![trimmed.to_string()];
        };

        let mut statements = Vec::new();
        for statement in parser {
            match statement {
                Ok(statement) => {
                    let statement = statement.trim();
                    if !statement.is_empty() {
                        statements.push(statement.to_string());
                    }
                }
                Err(_) => return vec![trimmed.to_string()],
            }
        }

        if statements.is_empty() {
            return Vec::new();
        }

        statements
    }

    /// Build a single EXPLAIN statement for this database type.
    fn build_explain_statement(&self, sql: &str) -> String {
        let sql = sql.trim();
        match self.name() {
            DatabaseType::MySQL
            | DatabaseType::PostgreSQL
            | DatabaseType::DuckDB
            | DatabaseType::ClickHouse
            // TDengine 同样支持 EXPLAIN 前缀。
            | DatabaseType::TDengine => {
                format!("EXPLAIN {sql}")
            }
            DatabaseType::SQLite => format!("EXPLAIN QUERY PLAN {sql}"),
            DatabaseType::MSSQL => {
                format!("SET SHOWPLAN_TEXT ON;\n{sql}\nSET SHOWPLAN_TEXT OFF;")
            }
            DatabaseType::Oracle => {
                format!(
                    "EXPLAIN PLAN FOR {sql};\nSELECT PLAN_TABLE_OUTPUT FROM TABLE(DBMS_XPLAN.DISPLAY())"
                )
            }
            _ => "".to_string(),
        }
    }

    /// Check whether SQL already is an EXPLAIN/SHOWPLAN statement or script.
    fn is_explain_statement(&self, sql: &str) -> bool {
        let trimmed = sql.trim_start();
        let upper = trimmed.to_ascii_uppercase();

        match self.name() {
            DatabaseType::MSSQL => upper.starts_with("SET SHOWPLAN_TEXT ON"),
            _ => upper.starts_with("EXPLAIN"),
        }
    }

    /// Build EXPLAIN SQL for all query statements in a SQL script.
    fn build_explain_sql(&self, sql: &str) -> Option<String> {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            return None;
        }

        if matches!(self.name(), DatabaseType::MSSQL) && self.is_explain_statement(trimmed) {
            return Some(trimmed.to_string());
        }

        let separator = if matches!(self.name(), DatabaseType::MSSQL) {
            "\n"
        } else {
            ";\n"
        };

        let explain_statements = self
            .split_sql_statements(trimmed)
            .into_iter()
            .filter_map(|statement| {
                if self.is_explain_statement(&statement) {
                    return Some(statement.trim().to_string());
                }
                if self.is_query_statement(&statement) {
                    return Some(self.build_explain_statement(&statement));
                }
                None
            })
            .collect::<Vec<_>>();

        if explain_statements.is_empty() {
            return None;
        }

        Some(explain_statements.join(separator))
    }

    /// Determine the statement category
    fn classify_statement(&self, sql: &str) -> StatementType {
        if let Ok(statements) = Parser::parse_sql(self.sql_dialect().as_ref(), sql) {
            if let Some(stmt) = statements.first() {
                return classify_stmt(stmt);
            }
        }
        classify_fallback(sql)
    }

    /// Analyze a SELECT query for editability and safe source-schema mapping.
    fn analyze_select_query(&self, sql: &str) -> SelectQueryAnalysis {
        if let Ok(statements) = Parser::parse_sql(self.sql_dialect().as_ref(), sql) {
            if let Some(Statement::Query(query)) = statements.first() {
                return analyze_query_capabilities(query);
            }
        }

        // Keep the legacy fallback useful for the editability affordance, but
        // never use string heuristics as a source of schema identity. A
        // parser failure must not enable LONGTEXT/BLOB reconciliation.
        let table_name = analyze_select_editability_fallback(sql);
        SelectQueryAnalysis {
            editable: table_name.is_some(),
            table_name,
            schema_metadata_safe: false,
        }
    }

    /// Check if a SELECT query might be editable.
    ///
    /// This compatibility wrapper intentionally exposes only the old
    /// `Option<table_name>` view. Callers that need schema reconciliation must
    /// use [`DatabasePlugin::analyze_select_query`] instead.
    fn analyze_select_editability(&self, sql: &str) -> Option<String> {
        let analysis = self.analyze_select_query(sql);
        analysis.editable.then_some(analysis.table_name).flatten()
    }

    /// List schemas in a database (for databases that support schemas)
    async fn list_schemas(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
    ) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// List schemas view (for databases that support schemas)
    async fn list_schemas_view(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
    ) -> Result<ObjectView> {
        Ok(ObjectView::default())
    }

    // === Table Operations ===
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

    /// List foreign keys for a table
    async fn list_foreign_keys(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
        _schema: Option<String>,
        _table: &str,
    ) -> Result<Vec<ForeignKeyDefinition>> {
        Ok(Vec::new())
    }

    /// List triggers for a specific table
    async fn list_table_triggers(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
        _schema: Option<String>,
        _table: &str,
    ) -> Result<Vec<TriggerInfo>> {
        Ok(Vec::new())
    }

    /// List check constraints for a specific table
    async fn list_table_checks(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
        _schema: Option<String>,
        _table: &str,
    ) -> Result<Vec<CheckInfo>> {
        Ok(Vec::new())
    }

    // === View Operations ===
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

    // === Function Operations ===

    async fn list_functions(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>>;

    async fn list_functions_in_schema(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<FunctionInfo>> {
        let _ = schema;
        self.list_functions(connection, database).await
    }

    async fn list_functions_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView>;

    async fn list_functions_view_in_schema(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<ObjectView> {
        let _ = schema;
        self.list_functions_view(connection, database).await
    }

    async fn get_function_definition(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
        _function: &str,
    ) -> Result<String> {
        bail!("Function definition is not supported")
    }

    async fn get_function_definition_for_routine(
        &self,
        connection: &dyn DbConnection,
        routine: &RoutineIdentity,
    ) -> Result<String> {
        self.get_function_definition(connection, &routine.database, &routine.name)
            .await
    }

    fn build_function_edit_script(
        &self,
        _routine: &RoutineIdentity,
        create_sql: &str,
    ) -> Result<String> {
        Ok(format!("{}\n", create_sql.trim()))
    }

    async fn get_function_edit_script(
        &self,
        connection: &dyn DbConnection,
        routine: &RoutineIdentity,
    ) -> Result<String> {
        let definition = self
            .get_function_definition_for_routine(connection, routine)
            .await?;
        self.build_function_edit_script(routine, &definition)
    }

    fn capabilities(&self) -> DatabaseCapabilities {
        DatabaseUiCapabilities {
            supports_functions: true,
            supports_procedures: true,
            table_engines: self.engines(),
            ..DatabaseUiCapabilities::default()
        }
    }

    fn ui_manifest(&self) -> DatabaseUiManifest {
        DatabaseUiManifest::default()
    }

    fn external_driver_manifest(&self) -> Option<crate::ipc::IpcDriverManifest> {
        None
    }

    fn resolve_reference_data(
        &self,
        kind: ReferenceDataKind,
        context: &HashMap<String, String>,
    ) -> Vec<FormSelectOption> {
        let _ = (kind, context);
        Vec::new()
    }

    // === Procedure Operations ===
    async fn list_procedures(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<FunctionInfo>>;

    async fn list_procedures_in_schema(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<FunctionInfo>> {
        let _ = schema;
        self.list_procedures(connection, database).await
    }

    async fn list_procedures_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView>;

    async fn list_procedures_view_in_schema(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<ObjectView> {
        let _ = schema;
        self.list_procedures_view(connection, database).await
    }

    async fn get_procedure_definition(
        &self,
        _connection: &dyn DbConnection,
        _database: &str,
        _procedure: &str,
    ) -> Result<String> {
        bail!("Procedure definition is not supported")
    }

    async fn get_procedure_definition_for_routine(
        &self,
        connection: &dyn DbConnection,
        routine: &RoutineIdentity,
    ) -> Result<String> {
        self.get_procedure_definition(connection, &routine.database, &routine.name)
            .await
    }

    fn build_procedure_edit_script(
        &self,
        _routine: &RoutineIdentity,
        create_sql: &str,
    ) -> Result<String> {
        Ok(format!("{}\n", create_sql.trim()))
    }

    async fn get_procedure_edit_script(
        &self,
        connection: &dyn DbConnection,
        routine: &RoutineIdentity,
    ) -> Result<String> {
        let definition = self
            .get_procedure_definition_for_routine(connection, routine)
            .await?;
        self.build_procedure_edit_script(routine, &definition)
    }

    // === Trigger Operations ===
    async fn list_triggers(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<Vec<TriggerInfo>>;

    async fn list_triggers_in_schema(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<String>,
    ) -> Result<Vec<TriggerInfo>> {
        let _ = schema;
        self.list_triggers(connection, database).await
    }

    async fn list_triggers_view(
        &self,
        connection: &dyn DbConnection,
        database: &str,
    ) -> Result<ObjectView>;

    // === Sequence Operations ===
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

    // === Helper Methods ===
    fn build_column_definition(&self, column: &ColumnInfo, include_name: bool) -> String;

    // === Database Management Operations ===
    /// Build SQL for creating a new database
    fn build_create_database_sql(&self, request: &DatabaseOperationRequest) -> String;

    async fn build_create_database_sql_async(
        &self,
        request: &DatabaseOperationRequest,
    ) -> Result<String> {
        Ok(self.build_create_database_sql(request))
    }

    /// Build SQL for modifying an existing database
    fn build_modify_database_sql(&self, request: &DatabaseOperationRequest) -> String;

    /// Build SQL for dropping a database
    fn build_drop_database_sql(&self, database_name: &str) -> String;

    async fn build_drop_database_sql_async(&self, database_name: &str) -> Result<String> {
        Ok(self.build_drop_database_sql(database_name))
    }

    // === User Management Operations ===
    /// Build SQL for listing database users.
    fn build_list_users_sql(&self, _database: Option<&str>) -> Option<String> {
        None
    }

    /// Columns used by the database users view. The order must match `build_list_users_sql`.
    fn user_list_columns(&self) -> Vec<ObjectViewColumn> {
        Vec::new()
    }

    /// List database users as a view model with localized column labels.
    async fn list_users_view(
        &self,
        connection: &dyn DbConnection,
        database: Option<&str>,
    ) -> Result<ObjectView> {
        let sql = self
            .build_list_users_sql(database)
            .ok_or_else(|| anyhow!("database users are not supported"))?;
        let query = match connection.query(&sql).await? {
            SqlResult::Query(query) => query,
            SqlResult::Error(error) => return Err(anyhow!(error.message)),
            SqlResult::Exec(_) => bail!("user listing did not return a result set"),
        };
        let columns = self.user_list_columns();
        let columns = if columns.is_empty() {
            query
                .columns
                .iter()
                .map(|name| ObjectViewColumn::new(name.clone(), name.clone()))
                .map(|column| column.width(180.0))
                .collect()
        } else {
            columns
        };
        let rows = query
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|value| value.unwrap_or_default())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        Ok(ObjectView {
            db_node_type: DbNodeType::Connection,
            title: format!("{} user(s)", rows.len()),
            columns,
            rows,
        })
    }

    /// Build SQL for creating a database user.
    fn build_create_user_sql(&self, _request: &DatabaseUserOperationRequest) -> Option<String> {
        None
    }

    /// Build SQL for modifying a database user.
    fn build_modify_user_sql(&self, _request: &DatabaseUserOperationRequest) -> Option<String> {
        None
    }

    /// Build SQL for dropping a database user.
    fn build_drop_user_sql(&self, _request: &DatabaseUserOperationRequest) -> Option<String> {
        None
    }

    /// Build SQL for changing database user privileges.
    fn build_user_privileges_sql(&self, _request: &DatabaseUserOperationRequest) -> Option<String> {
        None
    }

    // === Schema Management Operations ===
    /// Build SQL for creating a new schema
    fn build_create_schema_sql(&self, schema_name: &str) -> String {
        format!("CREATE SCHEMA {}", self.quote_identifier(schema_name))
    }

    /// Build SQL for dropping a schema
    fn build_drop_schema_sql(&self, schema_name: &str) -> String {
        format!("DROP SCHEMA {}", self.quote_identifier(schema_name))
    }

    /// Build SQL for adding/updating schema comment
    /// Returns None if the database doesn't support schema comments
    fn build_comment_schema_sql(&self, _schema_name: &str, _comment: &str) -> Option<String> {
        None
    }

    async fn build_database_tree(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
    ) -> Result<Vec<DbNode>> {
        let id = &node.id;
        let databases = self.list_databases(connection).await?;
        Ok(databases
            .into_iter()
            .map(|db| {
                DbNode::new(
                    format!("{}:{}", &node.id, db),
                    db.clone(),
                    DbNodeType::Database,
                    node.id.clone(),
                    node.database_type.clone(),
                )
                .with_parent_context(id)
            })
            .collect())
    }

    async fn build_schema_tree(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
    ) -> Result<Vec<DbNode>> {
        let id = &node.id;
        let schemas;
        let mut metadata: HashMap<String, String> = HashMap::new();
        if self.capabilities().uses_schema_as_database {
            schemas = self.list_schemas(connection, "").await?;
            metadata.insert("database".to_string(), "".to_string());
        } else {
            let database = node
                .get_database_name()
                .ok_or_else(|| anyhow!("Database name is required"))?;
            schemas = self.list_schemas(connection, &database).await?;
            metadata.insert("database".to_string(), database);
        }
        let mut nodes = Vec::new();
        for schema in schemas {
            let schema_node = DbNode::new(
                format!("{}:{}", id, schema),
                schema.clone(),
                DbNodeType::Schema,
                node.connection_id.clone(),
                node.database_type.clone(),
            )
            .with_parent_context(id)
            .with_metadata(metadata.clone());
            nodes.push(schema_node);
        }

        Ok(nodes)
    }

    async fn build_database_or_schema_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        schema: Option<String>,
    ) -> Result<Vec<DbNode>> {
        let mut nodes = Vec::new();
        let database = &*node
            .get_database_name()
            .ok_or_else(|| anyhow!("Database name not"))?;
        let id = &node.id;
        let mut metadata: HashMap<String, String> = HashMap::new();
        metadata.insert("database".to_string(), database.to_string());
        if let Some(s) = schema.clone() {
            metadata.insert("schema".to_string(), s.to_string());
        }

        let tables = self
            .list_tables(connection, database, schema.clone())
            .await?;
        let table_count = tables.len();
        let mut table_folder = DbNode::new(
            format!("{}:table_folder", id),
            "DbTree.Tables".to_string(),
            DbNodeType::TablesFolder,
            node.connection_id.clone(),
            node.database_type.clone(),
        )
        .with_parent_context(id)
        .with_metadata(metadata.clone());
        if table_count > 0 {
            let mut children: Vec<DbNode> = tables
                .into_iter()
                .map(|table_info| {
                    let mut meta: HashMap<String, String> = metadata.clone();
                    if let Some(comment) = &table_info.comment {
                        if !comment.is_empty() {
                            meta.insert("comment".to_string(), comment.clone());
                        }
                    }

                    DbNode::new(
                        format!("{}:table_folder:{}", id, table_info.name),
                        table_info.name.clone(),
                        DbNodeType::Table,
                        node.connection_id.clone(),
                        node.database_type.clone(),
                    )
                    .with_parent_context(format!("{}:table_folder", id))
                    .with_metadata(meta)
                })
                .collect();
            children.sort();
            table_folder.set_children(children)
        }
        nodes.push(table_folder);

        let capabilities = self.capabilities();
        if capabilities.supports_views {
            let views = self
                .list_views(connection, database, schema.clone())
                .await?;
            let view_count = views.len();
            let mut views_folder = DbNode::new(
                format!("{}:views_folder", id),
                "DbTree.Views".to_string(),
                DbNodeType::ViewsFolder,
                node.connection_id.clone(),
                node.database_type.clone(),
            )
            .with_parent_context(id)
            .with_metadata(metadata.clone());
            if view_count > 0 {
                let mut children: Vec<DbNode> = views
                    .into_iter()
                    .map(|view| {
                        let mut meta: HashMap<String, String> = metadata.clone();
                        if let Some(comment) = view.comment {
                            meta.insert("comment".to_string(), comment);
                        }

                        let mut vnode = DbNode::new(
                            format!("{}:views_folder:{}", id, view.name),
                            view.name.clone(),
                            DbNodeType::View,
                            node.connection_id.clone(),
                            node.database_type.clone(),
                        )
                        .with_parent_context(format!("{}:views_folder", id));

                        if !meta.is_empty() {
                            vnode = vnode.with_metadata(meta);
                        }
                        vnode
                    })
                    .collect();
                children.sort();
                views_folder.set_children(children);
            }
            nodes.push(views_folder);
        }

        // Functions folder
        if capabilities.supports_functions {
            let functions = self
                .list_functions_in_schema(connection, database, schema.clone())
                .await
                .unwrap_or_default();
            let function_count = functions.len();
            let mut functions_folder = DbNode::new(
                format!("{}:functions_folder", id),
                "DbTree.Functions".to_string(),
                DbNodeType::FunctionsFolder,
                node.connection_id.clone(),
                node.database_type.clone(),
            )
            .with_parent_context(id)
            .with_metadata(metadata.clone());
            if function_count > 0 {
                let mut children: Vec<DbNode> = functions
                    .into_iter()
                    .map(|func| {
                        routine_node(
                            func,
                            DbNodeType::Function,
                            &format!("{}:functions_folder", id),
                            &format!("{}:functions_folder", id),
                            &node.connection_id,
                            node.database_type.clone(),
                            &metadata,
                        )
                    })
                    .collect();
                children.sort();
                functions_folder.set_children(children);
            }
            nodes.push(functions_folder);
        }

        // Procedures folder
        if capabilities.supports_procedures {
            let procedures = self
                .list_procedures_in_schema(connection, database, schema.clone())
                .await
                .unwrap_or_default();
            let procedure_count = procedures.len();
            let mut procedures_folder = DbNode::new(
                format!("{}:procedures_folder", id),
                "DbTree.Procedures".to_string(),
                DbNodeType::ProceduresFolder,
                node.connection_id.clone(),
                node.database_type.clone(),
            )
            .with_parent_context(id)
            .with_metadata(metadata.clone());
            if procedure_count > 0 {
                let mut children: Vec<DbNode> = procedures
                    .into_iter()
                    .map(|procedure| {
                        routine_node(
                            procedure,
                            DbNodeType::Procedure,
                            &format!("{}:procedures_folder", id),
                            &format!("{}:procedures_folder", id),
                            &node.connection_id,
                            node.database_type.clone(),
                            &metadata,
                        )
                    })
                    .collect();
                children.sort();
                procedures_folder.set_children(children);
            }
            nodes.push(procedures_folder);
        }

        // Sequences folder (only for databases that support sequences)
        if capabilities.supports_sequences {
            let sequences = self
                .list_sequences(connection, database, schema)
                .await
                .unwrap_or_default();
            let sequence_count = sequences.len();
            let mut sequences_folder = DbNode::new(
                format!("{}:sequences_folder", id),
                "DbTree.Sequences".to_string(),
                DbNodeType::SequencesFolder,
                node.connection_id.clone(),
                node.database_type.clone(),
            )
            .with_parent_context(id)
            .with_metadata(metadata.clone());
            if sequence_count > 0 {
                let mut children: Vec<DbNode> = sequences
                    .into_iter()
                    .map(|seq| {
                        let mut seq_meta: HashMap<String, String> = metadata.clone();
                        if let Some(start) = seq.start_value {
                            seq_meta.insert("start_value".to_string(), start.to_string());
                        }
                        if let Some(inc) = seq.increment {
                            seq_meta.insert("increment".to_string(), inc.to_string());
                        }
                        if let Some(min) = seq.min_value {
                            seq_meta.insert("min_value".to_string(), min.to_string());
                        }
                        if let Some(max) = seq.max_value {
                            seq_meta.insert("max_value".to_string(), max.to_string());
                        }
                        DbNode::new(
                            format!("{}:sequences_folder:{}", id, seq.name),
                            seq.name.clone(),
                            DbNodeType::Sequence,
                            node.connection_id.clone(),
                            node.database_type.clone(),
                        )
                        .with_parent_context(format!("{}:sequences_folder", id))
                        .with_metadata(seq_meta)
                    })
                    .collect();
                children.sort();
                sequences_folder.set_children(children);
            }
            nodes.push(sequences_folder);
        }

        let queries_folder = self.load_queries(node, metadata.clone()).await?;
        nodes.push(queries_folder);
        Ok(nodes)
    }

    async fn load_queries(
        &self,
        node: &DbNode,
        metadata: HashMap<String, String>,
    ) -> Result<DbNode> {
        let node_id_for_queries = node.id.clone();
        let connection_id_for_queries = node.connection_id.clone();

        let queries_folder_node = DbNode::new(
            format!("{}:queries_folder", &node_id_for_queries),
            "DbTree.Queries".to_string(),
            DbNodeType::QueriesFolder,
            connection_id_for_queries.clone(),
            node.database_type.clone(),
        )
        .with_parent_context(node_id_for_queries.clone())
        .with_metadata(metadata);

        Ok(queries_folder_node)
    }

    async fn load_node_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
    ) -> Result<Vec<DbNode>> {
        let id = &node.id;
        match node.node_type {
            DbNodeType::Connection => {
                if self.capabilities().uses_schema_as_database {
                    self.build_schema_tree(connection, node).await
                } else {
                    self.build_database_tree(connection, node).await
                }
            }
            DbNodeType::Database => {
                if self.capabilities().supports_schema {
                    self.build_schema_tree(connection, node).await
                } else {
                    self.build_database_or_schema_children(connection, node, None)
                        .await
                }
            }
            DbNodeType::Schema => {
                let schema_name = node.get_schema_name();
                self.build_database_or_schema_children(connection, node, schema_name)
                    .await
            }
            DbNodeType::TablesFolder
            | DbNodeType::ViewsFolder
            | DbNodeType::FunctionsFolder
            | DbNodeType::ProceduresFolder
            | DbNodeType::SequencesFolder => {
                if node.children_loaded {
                    return Ok(node.children.clone());
                }
                self.load_schema_folder_children(connection, node, id).await
            }
            DbNodeType::QueriesFolder | DbNodeType::QueryFolder => {
                if node.children_loaded {
                    return Ok(node.children.clone());
                }
                self.load_queries_children(node, id).await
            }
            DbNodeType::Table => self.load_table_children(connection, node, id).await,
            DbNodeType::ColumnsFolder
            | DbNodeType::IndexesFolder
            | DbNodeType::ForeignKeysFolder
            | DbNodeType::TriggersFolder
            | DbNodeType::ChecksFolder => {
                if node.children_loaded {
                    return Ok(node.children.clone());
                }
                self.load_table_folder_children(connection, node, id).await
            }
            _ => Ok(Vec::new()),
        }
    }

    async fn load_schema_folder_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        id: &str,
    ) -> Result<Vec<DbNode>> {
        let database = &*node.get_database_name().unwrap_or_default();
        let schema = node.get_schema_name();
        match node.node_type {
            DbNodeType::TablesFolder => {
                let tables = self.list_tables(connection, database, schema).await?;
                let mut children: Vec<DbNode> = tables
                    .into_iter()
                    .map(|t| {
                        let mut meta = node.metadata.clone();
                        if let Some(comment) = &t.comment {
                            if !comment.is_empty() {
                                meta.insert("comment".to_string(), comment.clone());
                            }
                        }
                        DbNode::new(
                            format!("{}:{}", id, t.name),
                            t.name.clone(),
                            DbNodeType::Table,
                            node.connection_id.clone(),
                            node.database_type.clone(),
                        )
                        .with_parent_context(id)
                        .with_metadata(meta)
                    })
                    .collect();
                children.sort();
                Ok(children)
            }
            DbNodeType::ViewsFolder => {
                if !self.capabilities().supports_views {
                    return Ok(Vec::new());
                }
                let views = self.list_views(connection, database, schema).await?;
                let mut children: Vec<DbNode> = views
                    .into_iter()
                    .map(|v| {
                        let mut meta = node.metadata.clone();
                        if let Some(comment) = v.comment {
                            meta.insert("comment".to_string(), comment);
                        }
                        DbNode::new(
                            format!("{}:{}", id, v.name),
                            v.name.clone(),
                            DbNodeType::View,
                            node.connection_id.clone(),
                            node.database_type.clone(),
                        )
                        .with_parent_context(id)
                        .with_metadata(meta)
                    })
                    .collect();
                children.sort();
                Ok(children)
            }
            DbNodeType::FunctionsFolder => {
                let functions = self
                    .list_functions_in_schema(connection, database, schema.clone())
                    .await
                    .unwrap_or_default();
                let mut children: Vec<DbNode> = functions
                    .into_iter()
                    .map(|f| {
                        routine_node(
                            f,
                            DbNodeType::Function,
                            id,
                            id,
                            &node.connection_id,
                            node.database_type.clone(),
                            &node.metadata,
                        )
                    })
                    .collect();
                children.sort();
                Ok(children)
            }
            DbNodeType::ProceduresFolder => {
                let procedures = self
                    .list_procedures_in_schema(connection, database, schema.clone())
                    .await
                    .unwrap_or_default();
                let mut children: Vec<DbNode> = procedures
                    .into_iter()
                    .map(|p| {
                        routine_node(
                            p,
                            DbNodeType::Procedure,
                            id,
                            id,
                            &node.connection_id,
                            node.database_type.clone(),
                            &node.metadata,
                        )
                    })
                    .collect();
                children.sort();
                Ok(children)
            }
            DbNodeType::SequencesFolder => {
                let sequences = self
                    .list_sequences(connection, database, schema.clone())
                    .await
                    .unwrap_or_default();
                let filtered: Vec<_> = match schema {
                    Some(s) => sequences
                        .into_iter()
                        .filter(|seq| seq.name.starts_with(&format!("{}.", s)))
                        .collect(),
                    None => sequences,
                };
                let mut children: Vec<DbNode> = filtered
                    .into_iter()
                    .map(|seq| {
                        let mut meta = node.metadata.clone();
                        if let Some(v) = seq.start_value {
                            meta.insert("start_value".to_string(), v.to_string());
                        }
                        if let Some(v) = seq.increment {
                            meta.insert("increment".to_string(), v.to_string());
                        }
                        if let Some(v) = seq.min_value {
                            meta.insert("min_value".to_string(), v.to_string());
                        }
                        if let Some(v) = seq.max_value {
                            meta.insert("max_value".to_string(), v.to_string());
                        }
                        DbNode::new(
                            format!("{}:{}", id, seq.name),
                            seq.name.clone(),
                            DbNodeType::Sequence,
                            node.connection_id.clone(),
                            node.database_type.clone(),
                        )
                        .with_parent_context(id)
                        .with_metadata(meta)
                    })
                    .collect();
                children.sort();
                Ok(children)
            }
            _ => Ok(Vec::new()),
        }
    }

    async fn load_queries_children(&self, node: &DbNode, id: &str) -> Result<Vec<DbNode>> {
        let metadata = node.metadata.clone();
        let is_query_root = node.node_type != DbNodeType::QueryFolder;
        let (query_path, scope) = if node.node_type == DbNodeType::QueryFolder {
            match node.metadata.get("directory_path") {
                Some(path) => (std::path::PathBuf::from(path), None),
                None => return Ok(Vec::new()),
            }
        } else {
            let scope = QueryDirectoryScope::new(
                node.database_type.path_key(),
                node.connection_id.clone(),
                node.get_database_name().unwrap_or_default(),
            );
            match default_query_directory(&scope) {
                Ok(path) => (path, Some(scope)),
                Err(e) => {
                    error!("Failed to resolve queries directory: {}", e);
                    return Ok(Vec::new());
                }
            }
        };

        let entries = match list_query_directory(&query_path) {
            Ok(entries) => entries,
            Err(e) => {
                error!("Failed to read queries directory {:?}: {}", query_path, e);
                return Ok(Vec::new());
            }
        };

        let mut query_nodes = Vec::new();
        for entry in entries {
            let mut meta = metadata.clone();
            let (node_type, path_key) = match entry.kind {
                QueryDirectoryEntryKind::Directory => {
                    meta.insert(
                        "directory_path".to_string(),
                        entry.path.to_string_lossy().to_string(),
                    );
                    (DbNodeType::QueryFolder, "directory")
                }
                QueryDirectoryEntryKind::SqlFile => {
                    meta.insert(
                        "file_path".to_string(),
                        entry.path.to_string_lossy().to_string(),
                    );
                    (DbNodeType::NamedQuery, "file")
                }
            };

            let query_node = DbNode::new(
                format!("{}:{}:{}", id, path_key, entry.path.to_string_lossy()),
                entry.name,
                node_type,
                node.connection_id.clone(),
                node.database_type.clone(),
            )
            .with_parent_context(id)
            .with_metadata(meta);

            query_nodes.push(query_node);
        }

        if is_query_root {
            if let Some(scope) = scope {
                match added_query_directories(&scope) {
                    Ok(directories) => {
                        for directory in directories {
                            let mut meta = metadata.clone();
                            meta.insert(
                                "directory_path".to_string(),
                                directory.to_string_lossy().to_string(),
                            );
                            meta.insert("query_directory_root".to_string(), "added".to_string());
                            query_nodes.push(
                                DbNode::new(
                                    format!(
                                        "{}:added-directory:{}",
                                        id,
                                        directory.to_string_lossy()
                                    ),
                                    query_directory_display_name(&directory),
                                    DbNodeType::QueryFolder,
                                    node.connection_id.clone(),
                                    node.database_type.clone(),
                                )
                                .with_parent_context(id)
                                .with_metadata(meta),
                            );
                        }
                    }
                    Err(e) => {
                        error!("Failed to load added query directories: {}", e);
                    }
                }
            }
        }

        query_nodes.sort();
        Ok(query_nodes)
    }

    async fn load_table_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        id: &str,
    ) -> Result<Vec<DbNode>> {
        let db = &*node
            .get_database_name()
            .ok_or_else(|| anyhow::anyhow!("Database name not found"))?;
        let schema = node.get_schema_name();
        let table = &*node
            .get_table_name()
            .ok_or_else(|| anyhow::anyhow!("Table name not found"))?;

        let mut folder_metadata: HashMap<String, String> = node.metadata.clone();
        folder_metadata.insert("table".to_string(), table.to_string());

        let mut children = Vec::new();

        let columns = self
            .list_columns(connection, db, schema.clone(), table)
            .await?;
        children.push(
            self.build_table_subfolder(
                node,
                id,
                "columns_folder",
                "DbTree.Columns",
                DbNodeType::ColumnsFolder,
                &folder_metadata,
                columns
                    .into_iter()
                    .map(|c| {
                        (c.name.clone(), DbNodeType::Column, {
                            let mut m = folder_metadata.clone();
                            m.insert("type".to_string(), c.data_type);
                            m.insert("is_nullable".to_string(), c.is_nullable.to_string());
                            m.insert("is_primary_key".to_string(), c.is_primary_key.to_string());
                            m
                        })
                    })
                    .collect(),
            ),
        );

        let indexes: Vec<_> = self
            .list_indexes(connection, db, schema.clone(), table)
            .await?
            .into_iter()
            .filter(|idx| idx.name.to_uppercase() != "PRIMARY")
            .collect();
        children.push(
            self.build_table_subfolder(
                node,
                id,
                "indexes_folder",
                "DbTree.Indexes",
                DbNodeType::IndexesFolder,
                &folder_metadata,
                indexes
                    .into_iter()
                    .map(|idx| {
                        (idx.name.clone(), DbNodeType::Index, {
                            let mut m = folder_metadata.clone();
                            m.insert("unique".to_string(), idx.is_unique.to_string());
                            m.insert("columns".to_string(), idx.columns.join(", "));
                            m
                        })
                    })
                    .collect(),
            ),
        );

        let foreign_keys = self
            .list_foreign_keys(connection, db, schema.clone(), table)
            .await
            .unwrap_or_default();
        children.push(
            self.build_table_subfolder(
                node,
                id,
                "foreign_keys_folder",
                "DbTree.ForeignKeys",
                DbNodeType::ForeignKeysFolder,
                &folder_metadata,
                foreign_keys
                    .into_iter()
                    .map(|fk| {
                        (fk.name.clone(), DbNodeType::ForeignKey, {
                            let mut m = folder_metadata.clone();
                            m.insert("columns".to_string(), fk.columns.join(", "));
                            m.insert("ref_table".to_string(), fk.ref_table.clone());
                            if let Some(schema) = fk.ref_schema.as_deref() {
                                m.insert("ref_schema".to_string(), schema.to_string());
                            }
                            m.insert("ref_columns".to_string(), fk.ref_columns.join(", "));
                            m
                        })
                    })
                    .collect(),
            ),
        );

        let triggers = self
            .list_table_triggers(connection, db, schema.clone(), table)
            .await
            .unwrap_or_default();
        children.push(
            self.build_table_subfolder(
                node,
                id,
                "triggers_folder",
                "DbTree.Triggers",
                DbNodeType::TriggersFolder,
                &folder_metadata,
                triggers
                    .into_iter()
                    .map(|t| {
                        (t.name.clone(), DbNodeType::Trigger, {
                            let mut m = folder_metadata.clone();
                            m.insert("event".to_string(), t.event.clone());
                            m.insert("timing".to_string(), t.timing.clone());
                            m
                        })
                    })
                    .collect(),
            ),
        );

        let checks = self
            .list_table_checks(connection, db, schema.clone(), table)
            .await
            .unwrap_or_default();
        children.push(
            self.build_table_subfolder(
                node,
                id,
                "checks_folder",
                "DbTree.Checks",
                DbNodeType::ChecksFolder,
                &folder_metadata,
                checks
                    .into_iter()
                    .map(|c| {
                        (c.name.clone(), DbNodeType::Check, {
                            let mut m = folder_metadata.clone();
                            if let Some(def) = &c.definition {
                                m.insert("definition".to_string(), def.clone());
                            }
                            m
                        })
                    })
                    .collect(),
            ),
        );

        Ok(children)
    }

    fn build_table_subfolder(
        &self,
        node: &DbNode,
        parent_id: &str,
        folder_suffix: &str,
        display_prefix: &str,
        folder_type: DbNodeType,
        folder_metadata: &HashMap<String, String>,
        items: Vec<(String, DbNodeType, HashMap<String, String>)>,
    ) -> DbNode {
        let folder_id = format!("{}:{}", parent_id, folder_suffix);
        let count = items.len();
        let mut folder = DbNode::new(
            folder_id.clone(),
            display_prefix,
            folder_type,
            node.connection_id.clone(),
            node.database_type.clone(),
        )
        .with_parent_context(parent_id)
        .with_metadata(folder_metadata.clone());
        if count > 0 {
            let child_nodes: Vec<DbNode> = items
                .into_iter()
                .map(|(name, node_type, meta)| {
                    DbNode::new(
                        format!("{}:{}", folder_id, name),
                        name,
                        node_type,
                        node.connection_id.clone(),
                        node.database_type.clone(),
                    )
                    .with_metadata(meta)
                    .with_parent_context(&folder_id)
                })
                .collect();
            folder.set_children(child_nodes);
        }
        folder
    }

    async fn load_table_folder_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        id: &str,
    ) -> Result<Vec<DbNode>> {
        let database = &*node
            .get_database_name()
            .ok_or_else(|| anyhow::anyhow!("Database name not found"))?;
        let schema = node.get_schema_name();
        let table = &*node
            .get_table_name()
            .ok_or_else(|| anyhow::anyhow!("Table name not found"))?;
        match node.node_type {
            DbNodeType::ColumnsFolder => {
                let columns = self
                    .list_columns(connection, database, schema, table)
                    .await?;
                Ok(columns
                    .into_iter()
                    .map(|c| {
                        let mut meta = node.metadata.clone();
                        meta.insert("type".to_string(), c.data_type);
                        meta.insert("is_nullable".to_string(), c.is_nullable.to_string());
                        meta.insert("is_primary_key".to_string(), c.is_primary_key.to_string());
                        DbNode::new(
                            format!("{}:{}", id, c.name),
                            c.name,
                            DbNodeType::Column,
                            node.connection_id.clone(),
                            node.database_type.clone(),
                        )
                        .with_metadata(meta)
                        .with_parent_context(id)
                    })
                    .collect())
            }
            DbNodeType::IndexesFolder => {
                let indexes: Vec<_> = self
                    .list_indexes(connection, database, schema, table)
                    .await?
                    .into_iter()
                    .filter(|idx| idx.name.to_uppercase() != "PRIMARY")
                    .collect();
                Ok(indexes
                    .into_iter()
                    .map(|idx| {
                        let mut meta = node.metadata.clone();
                        meta.insert("unique".to_string(), idx.is_unique.to_string());
                        meta.insert("columns".to_string(), idx.columns.join(", "));
                        DbNode::new(
                            format!("{}:{}", id, idx.name),
                            idx.name,
                            DbNodeType::Index,
                            node.connection_id.clone(),
                            node.database_type.clone(),
                        )
                        .with_metadata(meta)
                        .with_parent_context(id)
                    })
                    .collect())
            }
            DbNodeType::ForeignKeysFolder => {
                let foreign_keys = self
                    .list_foreign_keys(connection, database, schema, table)
                    .await
                    .unwrap_or_default();
                Ok(foreign_keys
                    .into_iter()
                    .map(|fk| {
                        let mut meta = node.metadata.clone();
                        meta.insert("columns".to_string(), fk.columns.join(", "));
                        meta.insert("ref_table".to_string(), fk.ref_table.clone());
                        if let Some(schema) = fk.ref_schema.as_deref() {
                            meta.insert("ref_schema".to_string(), schema.to_string());
                        }
                        meta.insert("ref_columns".to_string(), fk.ref_columns.join(", "));
                        DbNode::new(
                            format!("{}:{}", id, fk.name),
                            fk.name,
                            DbNodeType::ForeignKey,
                            node.connection_id.clone(),
                            node.database_type.clone(),
                        )
                        .with_metadata(meta)
                        .with_parent_context(id)
                    })
                    .collect())
            }
            DbNodeType::TriggersFolder => {
                let triggers = self
                    .list_table_triggers(connection, database, schema, table)
                    .await
                    .unwrap_or_default();
                Ok(triggers
                    .into_iter()
                    .map(|t| {
                        let mut meta = node.metadata.clone();
                        meta.insert("event".to_string(), t.event.clone());
                        meta.insert("timing".to_string(), t.timing.clone());
                        DbNode::new(
                            format!("{}:{}", id, t.name),
                            t.name,
                            DbNodeType::Trigger,
                            node.connection_id.clone(),
                            node.database_type.clone(),
                        )
                        .with_metadata(meta)
                        .with_parent_context(id)
                    })
                    .collect())
            }
            DbNodeType::ChecksFolder => {
                let checks = self
                    .list_table_checks(connection, database, schema, table)
                    .await
                    .unwrap_or_default();
                Ok(checks
                    .into_iter()
                    .map(|c| {
                        let mut meta = node.metadata.clone();
                        if let Some(def) = &c.definition {
                            meta.insert("definition".to_string(), def.clone());
                        }
                        DbNode::new(
                            format!("{}:{}", id, c.name),
                            c.name,
                            DbNodeType::Check,
                            node.connection_id.clone(),
                            node.database_type.clone(),
                        )
                        .with_metadata(meta)
                        .with_parent_context(id)
                    })
                    .collect())
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Format pagination SQL clause. Override for databases with different syntax.
    fn format_pagination(&self, limit: usize, offset: usize, _order_clause: &str) -> String {
        format!(" LIMIT {} OFFSET {}", limit, offset)
    }

    /// Build a complete paginated query.
    ///
    /// The default implementation preserves the existing suffix-based pagination behavior.
    /// Implementations that cannot express pagination as a SQL suffix (for example Oracle 11g)
    /// should override this method and wrap `base_sql`.
    fn build_paginated_query(
        &self,
        base_sql: &str,
        limit: usize,
        offset: usize,
        order_clause: &str,
    ) -> PaginatedQuery {
        PaginatedQuery::new(format!(
            "{}{}",
            base_sql,
            self.format_pagination(limit, offset, order_clause)
        ))
    }

    /// Format table reference for queries. Override for databases with different syntax.
    /// - MySQL: `database`.`table`
    /// - PostgreSQL: "schema"."table" (uses schema, ignores database since connection is db-specific)
    /// - MSSQL: [database]..[table] or [database].[schema].[table]
    fn format_table_reference(&self, database: &str, _schema: Option<&str>, table: &str) -> String {
        format!(
            "{}.{}",
            self.quote_identifier(database),
            self.quote_identifier(table)
        )
    }

    /// Format table reference for SQL export output (omit database, keep schema when present).
    fn format_export_table_reference(
        &self,
        _database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> String {
        match schema {
            Some(schema) => format!(
                "{}.{}",
                self.quote_identifier(schema),
                self.quote_identifier(table)
            ),
            None => self.quote_identifier(table),
        }
    }

    // === Table Data Operations ===
    /// Query table data with pagination, filtering and sorting
    async fn query_table_data(
        &self,
        connection: &dyn DbConnection,
        request: TableDataRequest,
    ) -> Result<TableDataResponse> {
        let start_time = std::time::Instant::now();

        let where_clause = match request.where_clause {
            Some(ref c) if !c.trim().is_empty() => format!(" WHERE {}", c.trim()),
            _ => String::new(),
        };
        let order_clause = match request.order_by_clause {
            Some(ref c) if !c.trim().is_empty() => format!(" ORDER BY {}", c.trim()),
            _ => String::new(),
        };

        let offset = request.effective_offset();

        // Build table reference
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

        // Query with pagination, include rowid if supported
        let base_sql = if self.supports_rowid() {
            let rowid_col = self.rowid_column_name();
            format!(
                "SELECT {} AS __rowid__, t.* FROM {} t{}{}",
                rowid_col, table_ref, where_clause, order_clause
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
            SqlResult::Query(query_result) => Ok::<QueryResult, Error>(query_result),
            SqlResult::Exec(_) => bail!(t!("Error.query_type_error")),
            SqlResult::Error(sql_error_info) => bail!(sql_error_info.message),
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

    /// Generate SQL preview for table changes without executing them
    fn generate_table_changes_sql(&self, request: &TableSaveRequest) -> String {
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

    // === Copy SQL Generation Methods ===

    /// Generate INSERT SQL statements for copying
    fn generate_copy_insert_sql(&self, request: &CopySqlRequest) -> String {
        if request.rows.is_empty()
            || request.column_names.is_empty()
            || request
                .rows
                .iter()
                .any(|row| row.len() != request.column_names.len())
        {
            return String::new();
        }

        let table_name = self.format_copy_table_name(request.schema.as_deref(), &request.table);
        let quoted_columns: Vec<String> = request
            .column_names
            .iter()
            .map(|c| self.quote_identifier(c))
            .collect();
        let columns_str = quoted_columns.join(", ");

        let mut statements = Vec::new();

        for row in &request.rows {
            let values: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(i, val)| {
                    let col_info = request.columns.get(i);
                    self.format_copy_value(val, col_info)
                })
                .collect();
            let values_str = values.join(", ");

            statements.push(format!(
                "INSERT INTO {} ({}) VALUES ({});",
                table_name, columns_str, values_str
            ));
        }

        statements.join("\n")
    }

    /// Generate INSERT SQL statements with column comments for copying
    fn generate_copy_insert_with_comments_sql(&self, request: &CopySqlRequest) -> String {
        if request.rows.is_empty()
            || request.column_names.is_empty()
            || request
                .rows
                .iter()
                .any(|row| row.len() != request.column_names.len())
        {
            return String::new();
        }

        let table_name = self.format_copy_table_name(request.schema.as_deref(), &request.table);

        // Generate column names with comments
        let columns_with_comments: Vec<String> = request
            .column_names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let quoted = self.quote_identifier(name);
                if let Some(col_info) = request.columns.get(i) {
                    if let Some(comment) = &col_info.comment {
                        if !comment.is_empty() {
                            return format!("{} /* {} */", quoted, comment);
                        }
                    }
                }
                quoted
            })
            .collect();
        let columns_str = columns_with_comments.join(", ");

        let mut statements = Vec::new();

        for row in &request.rows {
            let values: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(i, val)| {
                    let col_info = request.columns.get(i);
                    self.format_copy_value(val, col_info)
                })
                .collect();
            let values_str = values.join(", ");

            statements.push(format!(
                "INSERT INTO {} ({}) VALUES ({});",
                table_name, columns_str, values_str
            ));
        }

        statements.join("\n")
    }

    /// Generate UPDATE SQL statements for copying
    fn generate_copy_update_sql(&self, request: &CopySqlRequest) -> String {
        if request.rows.is_empty() || request.column_names.is_empty() {
            return String::new();
        }

        let original_rows = request.original_rows.as_ref().unwrap_or(&request.rows);
        if original_rows.len() != request.rows.len()
            || request
                .rows
                .iter()
                .chain(original_rows.iter())
                .any(|row| row.len() != request.column_names.len())
        {
            return String::new();
        }
        let table_name = self.format_copy_table_name(request.schema.as_deref(), &request.table);
        let mut statements = Vec::new();

        for (row, original_row) in request.rows.iter().zip(original_rows.iter()) {
            // Generate SET clause
            let set_parts: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(i, val)| {
                    let col_name = self.quote_identifier(
                        request
                            .column_names
                            .get(i)
                            .map(|s| s.as_str())
                            .unwrap_or(""),
                    );
                    let col_info = request.columns.get(i);
                    let value = self.format_copy_value(val, col_info);
                    format!("{} = {}", col_name, value)
                })
                .collect();
            let set_str = set_parts.join(", ");

            // Generate WHERE clause
            let Some(where_str) = self.generate_copy_where_clause(request, original_row) else {
                return String::new();
            };

            statements.push(format!(
                "UPDATE {} SET {} WHERE {};",
                table_name, set_str, where_str
            ));
        }

        statements.join("\n")
    }

    /// Generate DELETE SQL statements for copying
    fn generate_copy_delete_sql(&self, request: &CopySqlRequest) -> String {
        let original_rows = request.original_rows.as_ref().unwrap_or(&request.rows);
        if original_rows.is_empty()
            || request.column_names.is_empty()
            || original_rows
                .iter()
                .any(|row| row.len() != request.column_names.len())
        {
            return String::new();
        }
        let table_name = self.format_copy_table_name(request.schema.as_deref(), &request.table);
        let mut statements = Vec::new();

        for row in original_rows {
            let Some(where_str) = self.generate_copy_where_clause(request, row) else {
                return String::new();
            };
            statements.push(format!("DELETE FROM {} WHERE {};", table_name, where_str));
        }

        statements.join("\n")
    }

    /// Format table name for copy SQL (with optional schema)
    fn format_copy_table_name(&self, schema: Option<&str>, table: &str) -> String {
        let quoted_table = self.quote_identifier(table);
        match schema {
            Some(s) if !s.is_empty() => {
                format!("{}.{}", self.quote_identifier(s), quoted_table)
            }
            _ => quoted_table,
        }
    }

    /// Format a value for copy SQL based on column type
    fn format_copy_value(&self, value: &TableCellValue, col_info: Option<&ColumnInfo>) -> String {
        self.format_table_change_value(value, col_info)
    }

    /// Check if data type is numeric
    fn is_numeric_type(&self, data_type: &str) -> bool {
        let numeric_types = [
            "INT",
            "INTEGER",
            "BIGINT",
            "SMALLINT",
            "TINYINT",
            "MEDIUMINT",
            "DECIMAL",
            "NUMERIC",
            "FLOAT",
            "DOUBLE",
            "REAL",
            "NUMBER",
            "MONEY",
            "SMALLMONEY",
            "BIT",
        ];
        numeric_types.iter().any(|t| data_type.contains(t))
    }

    /// Check if data type is boolean
    fn is_boolean_type(&self, data_type: &str) -> bool {
        data_type.contains("BOOL") || data_type == "BIT"
    }

    /// Check if data type is binary
    fn is_binary_type(&self, data_type: &str) -> bool {
        let binary_types = ["BLOB", "BINARY", "VARBINARY", "BYTEA", "RAW"];
        binary_types.iter().any(|t| data_type.contains(t))
    }

    /// Format boolean value (database-specific, can be overridden)
    fn format_boolean_value(&self, v: &str) -> String {
        if v == "1" || v.eq_ignore_ascii_case("true") {
            "TRUE".to_string()
        } else {
            "FALSE".to_string()
        }
    }

    /// Format binary value (database-specific, can be overridden)
    fn format_binary_value(&self, v: &str) -> String {
        self.escape_copy_string(v)
    }

    /// Format exact binary bytes as a database-specific SQL expression.
    ///
    /// This is intentionally separate from [`DatabasePlugin::format_binary_value`], which receives
    /// a display string used by copy SQL. Import/export callers with lossless bytes should use this
    /// method instead of trying to infer binary values from their display representation.
    fn format_binary_literal(&self, bytes: &[u8]) -> String {
        format_binary_literal_for_database(&self.name(), bytes)
    }

    /// Escape string for copy SQL (database-specific, can be overridden)
    fn escape_copy_string(&self, s: &str) -> String {
        let escaped = s.replace('\'', "''");
        format!("'{}'", escaped)
    }

    /// Generate WHERE clause for copy SQL
    fn generate_copy_where_clause(
        &self,
        request: &CopySqlRequest,
        row: &[TableCellValue],
    ) -> Option<String> {
        if row.len() != request.column_names.len() {
            return None;
        }

        // Prefer primary key columns
        let primary_key_indices: Vec<usize> = request
            .columns
            .iter()
            .enumerate()
            .filter(|(index, col)| *index < request.column_names.len() && col.is_primary_key)
            .map(|(i, _)| i)
            .collect();

        let indices_to_use = if !primary_key_indices.is_empty() {
            primary_key_indices
        } else {
            // If no primary key, use all columns
            (0..request.column_names.len()).collect()
        };

        let conditions: Option<Vec<String>> = indices_to_use
            .iter()
            .map(|&i| {
                let col_name = request.column_names.get(i)?;
                let val = row.get(i)?;
                let col_info = request.columns.get(i);

                let quoted_col = self.quote_identifier(col_name);
                match val {
                    TableCellValue::Null => Some(format!("{} IS NULL", quoted_col)),
                    TableCellValue::Text(_) | TableCellValue::Binary(_) => {
                        let formatted = self.format_copy_value(val, col_info);
                        Some(format!("{} = {}", quoted_col, formatted))
                    }
                }
            })
            .collect();
        let conditions = conditions?;
        (!conditions.is_empty()).then(|| conditions.join(" AND "))
    }

    fn build_table_change_sql(
        &self,
        request: &TableSaveRequest,
        change: &TableRowChange,
    ) -> Option<String> {
        let table_ident = self.format_table_reference(
            &request.database,
            request.schema.as_deref(),
            &request.table,
        );

        match change {
            TableRowChange::Added { data } => {
                if data.is_empty() {
                    return None;
                }
                let columns: Vec<String> = request
                    .columns
                    .iter()
                    .map(|column| self.quote_identifier(&*column.name))
                    .collect();
                let values: Vec<String> = data
                    .iter()
                    .enumerate()
                    .map(|(column_index, value)| {
                        self.format_table_change_value(value, request.columns.get(column_index))
                    })
                    .collect();

                Some(format!(
                    "INSERT INTO {} ({}) VALUES ({})",
                    table_ident,
                    columns.join(", "),
                    values.join(", ")
                ))
            }
            TableRowChange::Updated {
                original_data,
                changes,
                rowid,
            } => {
                if changes.is_empty() {
                    return None;
                }

                let set_clause: Vec<String> = changes
                    .iter()
                    .map(|change| {
                        let column_name = if change.column_name.is_empty() {
                            request
                                .columns
                                .get(change.column_index)
                                .map(|c| c.name.clone())
                                .unwrap_or_default()
                        } else {
                            change.column_name.clone()
                        };
                        let ident = self.quote_identifier(&column_name);
                        let value = self.format_table_change_value(
                            &change.new_value,
                            request.columns.get(change.column_index),
                        );
                        format!("{} = {}", ident, value)
                    })
                    .collect();

                if let Some(rid) = rowid {
                    let rowid_col = self.rowid_column_name();
                    return Some(format!(
                        "UPDATE {} SET {} WHERE {} = '{}'",
                        table_ident,
                        set_clause.join(", "),
                        rowid_col,
                        rid.replace('\'', "''")
                    ));
                }

                let (where_clause, limit_clause) =
                    self.build_where_and_limit_clause(request, original_data);

                if limit_clause == " __SQLITE_ROWID_LIMIT__" {
                    let simple_table = self.quote_identifier(&request.table);
                    Some(format!(
                        "UPDATE {} SET {} WHERE rowid IN (SELECT rowid FROM {} WHERE {} LIMIT 1)",
                        table_ident,
                        set_clause.join(", "),
                        simple_table,
                        where_clause
                    ))
                } else {
                    Some(format!(
                        "UPDATE {} SET {}{}{}{}",
                        table_ident,
                        set_clause.join(", "),
                        if where_clause.is_empty() {
                            ""
                        } else {
                            " WHERE "
                        },
                        where_clause,
                        limit_clause
                    ))
                }
            }
            TableRowChange::Deleted {
                original_data,
                rowid,
            } => {
                if let Some(rid) = rowid {
                    let rowid_col = self.rowid_column_name();
                    return Some(format!(
                        "DELETE FROM {} WHERE {} = '{}'",
                        table_ident,
                        rowid_col,
                        rid.replace('\'', "''")
                    ));
                }

                let (where_clause, limit_clause) =
                    self.build_where_and_limit_clause(request, original_data);

                if limit_clause == " __SQLITE_ROWID_LIMIT__" {
                    let simple_table = self.quote_identifier(&request.table);
                    Some(format!(
                        "DELETE FROM {} WHERE rowid IN (SELECT rowid FROM {} WHERE {} LIMIT 1)",
                        table_ident, simple_table, where_clause
                    ))
                } else {
                    Some(format!(
                        "DELETE FROM {}{}{}{}",
                        table_ident,
                        if where_clause.is_empty() {
                            ""
                        } else {
                            " WHERE "
                        },
                        where_clause,
                        limit_clause
                    ))
                }
            }
        }
    }

    fn build_limit_clause(&self) -> String;

    fn build_where_and_limit_clause(
        &self,
        request: &TableSaveRequest,
        original_data: &[TableCellValue],
    ) -> (String, String);

    /// Format a value used by table-editor INSERT/UPDATE/DELETE statements.
    ///
    /// The default keeps the historical behavior of treating text as a quoted
    /// SQL string. Database implementations can override this when a column's
    /// display value has a database-specific literal representation.
    fn format_table_change_value(
        &self,
        value: &TableCellValue,
        column: Option<&ColumnInfo>,
    ) -> String {
        crate::sql_literal::format_table_value_for_database(&self.name(), value, column)
    }

    fn build_table_change_where_clause(
        &self,
        request: &TableSaveRequest,
        original_data: &[TableCellValue],
    ) -> String {
        let column_names: Vec<&str> = request.columns.iter().map(|c| c.name.as_str()).collect();

        let primary_key_indices: Vec<usize> = request
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.is_primary_key)
            .map(|(i, _)| i)
            .collect();

        let unique_key_indices: Vec<usize> = request
            .index_infos
            .iter()
            .filter(|idx| idx.is_unique)
            .flat_map(|idx| {
                idx.columns
                    .iter()
                    .filter_map(|col_name| column_names.iter().position(|n| n == col_name))
            })
            .collect();

        let indices: Vec<usize> = if !primary_key_indices.is_empty() {
            primary_key_indices
        } else if !unique_key_indices.is_empty() {
            unique_key_indices
        } else {
            (0..column_names.len()).collect()
        };

        let mut parts = Vec::new();
        for index in indices {
            if let (Some(column), Some(value)) = (column_names.get(index), original_data.get(index))
            {
                let ident = self.quote_identifier(column);
                match value {
                    TableCellValue::Null => parts.push(format!("{} IS NULL", ident)),
                    TableCellValue::Text(_) | TableCellValue::Binary(_) => {
                        let formatted =
                            self.format_table_change_value(value, request.columns.get(index));
                        parts.push(format!("{} = {}", ident, formatted));
                    }
                }
            }
        }

        parts.join(" AND ")
    }

    // === Export Operations ===
    /// Export table CREATE statement
    async fn export_table_create_sql(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<&str>,
        table: &str,
    ) -> Result<String> {
        default_export_table_create_sql(self, connection, database, schema, table).await
    }

    /// Export table data as INSERT statements
    async fn export_table_data_sql(
        &self,
        connection: &dyn DbConnection,
        database: &str,
        schema: Option<&str>,
        table: &str,
        where_clause: Option<&str>,
        limit: Option<usize>,
    ) -> Result<String> {
        let table_ref = self.format_table_reference(database, schema, table);
        let mut sql = format!("SELECT * FROM {table_ref}");
        if let Some(where_clause) = where_clause {
            sql.push_str(" WHERE ");
            sql.push_str(where_clause);
        }
        let query = limit
            .map(|limit| self.build_paginated_query(&sql, limit, 0, ""))
            .unwrap_or_else(|| PaginatedQuery::new(sql));
        let result = connection
            .query(&query.sql)
            .await
            .map_err(|error| anyhow::anyhow!("Query failed: {error}"))?;
        let SqlResult::Query(mut query_result) = result else {
            return Ok(String::new());
        };
        query.strip_hidden_result_columns(&mut query_result)?;
        crate::query_result_normalization::normalize_table_query_result(
            self,
            connection,
            database,
            schema,
            table,
            &mut query_result,
        )
        .await?;
        let table_ident = self.format_export_table_reference(database, schema, table);
        crate::import_export::formats::sql_export::render_insert_statements(
            self,
            &table_ident,
            &query_result,
        )
    }

    // === Charset and Collation ===
    /// Get list of available character sets for this database
    fn get_charsets(&self) -> Vec<CharsetInfo> {
        vec![]
    }

    /// Get collations for a specific charset
    fn get_collations(&self, _charset: &str) -> Vec<CollationInfo> {
        vec![]
    }

    /// Get supported table engines for this database.
    fn engines(&self) -> Vec<String> {
        vec![]
    }

    // === Data Types ===
    /// Get list of available data types for this database
    /// Returns a slice of (type_name, description) tuples
    fn get_data_types(&self) -> &[(&'static str, &'static str)] {
        // Default implementation with common types
        &[
            ("INT", "Integer number"),
            ("VARCHAR", "Variable-length string"),
            ("TEXT", "Long text"),
            ("DATE", "Date"),
            ("DATETIME", "Date and time"),
            ("BOOLEAN", "True/False"),
            ("DECIMAL", "Decimal number"),
        ]
    }

    /// Parse a column type string into its components
    /// e.g., "VARCHAR(255)" -> ParsedColumnType { base_type: "VARCHAR", length: Some(255), ... }
    fn parse_column_type(&self, type_str: &str) -> ParsedColumnType {
        let upper = type_str.to_uppercase();
        let is_unsigned = upper.contains("UNSIGNED");
        let is_auto_increment = upper.contains("AUTO_INCREMENT");

        if let Some(start) = type_str.find('(') {
            if let Some(end) = type_str.find(')') {
                let base_type = type_str[..start].trim().to_string();
                let params = &type_str[start + 1..end];

                if let Some(comma) = params.find(',') {
                    let length = params[..comma].trim().parse().ok();
                    let scale = params[comma + 1..].trim().parse().ok();
                    return ParsedColumnType {
                        base_type,
                        length,
                        scale,
                        enum_values: None,
                        is_unsigned,
                        is_auto_increment,
                    };
                }

                let length = params.trim().parse().ok();
                return ParsedColumnType {
                    base_type,
                    length,
                    scale: None,
                    enum_values: None,
                    is_unsigned,
                    is_auto_increment,
                };
            }
        }

        ParsedColumnType {
            base_type: type_str
                .split_whitespace()
                .next()
                .unwrap_or(type_str)
                .to_string(),
            length: None,
            scale: None,
            enum_values: None,
            is_unsigned,
            is_auto_increment,
        }
    }

    /// Check if a data type is an enum or set type (database-specific)
    fn is_enum_type(&self, _type_name: &str) -> bool {
        false
    }

    // === DDL Operations ===
    /// Drop database
    fn drop_database(&self, database: &str) -> String {
        format!(
            "DROP DATABASE IF EXISTS {}",
            self.quote_identifier(database)
        )
    }

    async fn drop_database_async(&self, database: &str) -> Result<String> {
        Ok(self.drop_database(database))
    }

    /// Drop table
    fn drop_table(&self, database: &str, schema: Option<&str>, table: &str) -> String {
        // Default implementation for MySQL/ClickHouse: database.table
        // PostgreSQL/SQL Server with schema: database.schema.table or schema.table
        // Oracle: schema.table (database is ignored)
        if let Some(schema) = schema {
            format!(
                "DROP TABLE IF EXISTS {}.{}.{}",
                self.quote_identifier(database),
                self.quote_identifier(schema),
                self.quote_identifier(table)
            )
        } else {
            format!(
                "DROP TABLE IF EXISTS {}.{}",
                self.quote_identifier(database),
                self.quote_identifier(table)
            )
        }
    }

    /// Truncate table
    fn truncate_table(&self, _database: &str, table: &str) -> String {
        format!("TRUNCATE TABLE {}", self.quote_identifier(table))
    }

    /// Truncate table with an optional schema.
    fn truncate_table_with_schema(
        &self,
        database: &str,
        _schema: Option<&str>,
        table: &str,
    ) -> String {
        self.truncate_table(database, table)
    }

    /// Rename table
    fn rename_table(&self, database: &str, old_name: &str, new_name: &str) -> String;

    /// Build native backup-table SQL.
    /// 默认实现使用 `CREATE TABLE ... AS SELECT ...`，数据库插件可按方言覆盖。
    fn build_backup_table_sql(
        &self,
        _database: &str,
        _schema: Option<&str>,
        source_table: &str,
        target_table: &str,
    ) -> String {
        format!(
            "CREATE TABLE {} AS SELECT * FROM {};",
            self.quote_identifier(target_table),
            self.quote_identifier(source_table)
        )
    }

    /// Drop view
    fn drop_view(&self, _database: &str, view: &str) -> String {
        format!("DROP VIEW IF EXISTS {}", self.quote_identifier(view))
    }

    /// Build column definition from ColumnDefinition (for table designer)
    fn build_column_def(&self, col: &ColumnDefinition) -> String;

    /// Build FOREIGN KEY constraint definition from ForeignKeyDefinition.
    fn build_foreign_key_def(&self, foreign_key: &ForeignKeyDefinition) -> String {
        let columns = foreign_key
            .columns
            .iter()
            .map(|column| self.quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let ref_columns = foreign_key
            .ref_columns
            .iter()
            .map(|column| self.quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let referenced_table = match foreign_key.ref_schema.as_deref() {
            Some(schema) if !schema.trim().is_empty() => format!(
                "{}.{}",
                self.quote_identifier(schema),
                self.quote_identifier(&foreign_key.ref_table)
            ),
            _ => self.quote_identifier(&foreign_key.ref_table),
        };
        let mut definition = format!(
            "CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({})",
            self.quote_identifier(&foreign_key.name),
            columns,
            referenced_table,
            ref_columns
        );
        if let Some(action) = foreign_key_action_sql(&foreign_key.on_delete) {
            definition.push_str(&format!(" ON DELETE {action}"));
        }
        if let Some(action) = foreign_key_action_sql(&foreign_key.on_update) {
            definition.push_str(&format!(" ON UPDATE {action}"));
        }
        definition
    }

    /// Compare two foreign keys for SQL-relevant differences.
    fn foreign_key_changed(
        &self,
        left: &ForeignKeyDefinition,
        right: &ForeignKeyDefinition,
    ) -> bool {
        left.columns != right.columns
            || left.ref_table != right.ref_table
            || left.ref_schema != right.ref_schema
            || left.ref_columns != right.ref_columns
            || foreign_key_action_sql(&left.on_delete) != foreign_key_action_sql(&right.on_delete)
            || foreign_key_action_sql(&left.on_update) != foreign_key_action_sql(&right.on_update)
    }

    /// Build SQL for adding a foreign key to an existing table.
    fn build_add_foreign_key_sql(
        &self,
        table_name: &str,
        foreign_key: &ForeignKeyDefinition,
    ) -> String {
        format!(
            "ALTER TABLE {} ADD {};",
            self.quote_identifier(table_name),
            self.build_foreign_key_def(foreign_key)
        )
    }

    /// Build SQL for dropping a foreign key from an existing table.
    fn build_drop_foreign_key_sql(&self, table_name: &str, foreign_key_name: &str) -> String {
        format!(
            "ALTER TABLE {} DROP CONSTRAINT {};",
            self.quote_identifier(table_name),
            self.quote_identifier(foreign_key_name)
        )
    }

    /// Build CREATE TABLE SQL from TableDesign
    fn build_create_table_sql(&self, design: &TableDesign) -> String;

    /// Build CREATE TABLE SQL through an async-capable path.
    ///
    /// The default implementation deliberately calls the synchronous local builder.
    /// External IPC plugins can override this to ask the driver for dialect-specific
    /// SQL without forcing synchronous UI preview code to block on IPC.
    async fn build_create_table_sql_async(
        &self,
        _connection: &dyn DbConnection,
        design: &TableDesign,
    ) -> Result<String> {
        Ok(self.build_create_table_sql(design))
    }

    /// Build CREATE TABLE SQL through an async-capable path with an explicit
    /// target schema.
    ///
    /// The default implementation delegates to
    /// [`DatabasePlugin::build_create_table_sql_async`]. External IPC plugins
    /// override this so the driver can qualify the table with the target
    /// schema instead of falling back to the connection database name, which
    /// Oracle/PostgreSQL-compatible drivers (DM, Kingbase) otherwise treat as
    /// the schema/owner.
    async fn build_create_table_sql_with_schema_async(
        &self,
        connection: &dyn DbConnection,
        _schema: Option<&str>,
        design: &TableDesign,
    ) -> Result<String> {
        self.build_create_table_sql_async(connection, design).await
    }

    /// Build ALTER TABLE SQL from original and new TableDesign
    /// Returns a series of ALTER TABLE statements for the differences
    fn build_alter_table_sql(&self, original: &TableDesign, new: &TableDesign) -> String;

    /// 生成单列重命名 SQL，默认使用标准 RENAME COLUMN 语法。
    /// MySQL 需使用 CHANGE COLUMN，MSSQL 需使用 sp_rename，应覆盖此方法。
    fn build_column_rename_sql(
        &self,
        table_name: &str,
        old_name: &str,
        new_name: &str,
        _new_column: Option<&ColumnDefinition>,
    ) -> String {
        let quoted_table = self.quote_identifier(table_name);
        let quoted_old = self.quote_identifier(old_name);
        let quoted_new = self.quote_identifier(new_name);
        format!(
            "ALTER TABLE {} RENAME COLUMN {} TO {};",
            quoted_table, quoted_old, quoted_new
        )
    }

    /// 带列重命名支持的 ALTER TABLE SQL 生成。
    /// 默认实现：map_design_for_diff → build_alter_table_sql → 追加 rename。
    fn build_alter_table_sql_with_renames(
        &self,
        original: &TableDesign,
        new: &TableDesign,
        column_renames: &[(String, String)],
    ) -> String {
        let design_for_diff = map_design_for_diff(new, column_renames);
        let base_sql = self.build_alter_table_sql(original, &design_for_diff);
        let rename_statements: Vec<String> = column_renames
            .iter()
            .map(|(old_name, new_name)| {
                let new_column = new.columns.iter().find(|col| col.name == *new_name);
                self.build_column_rename_sql(&new.table_name, old_name, new_name, new_column)
            })
            .collect();
        merge_alter_sql(base_sql, rename_statements)
    }

    /// Async-capable ALTER TABLE builder.
    ///
    /// Use this in execution paths that already run off the UI thread. Synchronous
    /// preview paths should keep using [`DatabasePlugin::build_alter_table_sql_with_renames`]
    /// so methods that do not require a connection never block on IPC.
    async fn build_alter_table_sql_with_renames_async(
        &self,
        _connection: &dyn DbConnection,
        original: &TableDesign,
        new: &TableDesign,
        column_renames: &[(String, String)],
    ) -> Result<String> {
        Ok(self.build_alter_table_sql_with_renames(original, new, column_renames))
    }

    /// Async-capable ALTER TABLE builder with an explicit target schema.
    ///
    /// The default implementation delegates to
    /// [`DatabasePlugin::build_alter_table_sql_with_renames_async`]. External
    /// IPC plugins override this so the driver can qualify the table with the
    /// target schema instead of the connection database name.
    async fn build_alter_table_sql_with_schema_async(
        &self,
        connection: &dyn DbConnection,
        _schema: Option<&str>,
        original: &TableDesign,
        new: &TableDesign,
        column_renames: &[(String, String)],
    ) -> Result<String> {
        self.build_alter_table_sql_with_renames_async(connection, original, new, column_renames)
            .await
    }

    /// Check if a column definition has changed
    fn column_changed(&self, original: &ColumnDefinition, new: &ColumnDefinition) -> bool {
        original.data_type.to_uppercase() != new.data_type.to_uppercase()
            || original.length != new.length
            || original.precision != new.precision
            || original.scale != new.scale
            || original.is_nullable != new.is_nullable
            || original.is_auto_increment != new.is_auto_increment
            || original.is_unsigned != new.is_unsigned
            || original.default_value != new.default_value
            || original.comment != new.comment
            || original.charset != new.charset
            || original.collation != new.collation
    }

    /// Build type string for a column (used in ALTER statements)
    fn build_type_string(&self, col: &ColumnDefinition) -> String {
        let mut type_str = col.data_type.clone();
        if let Some(precision) = col.precision {
            if let Some(scale) = col.scale {
                type_str = format!("{}({},{})", type_str, precision, scale);
            } else {
                type_str = format!("{}({})", type_str, precision);
            }
        } else if let Some(len) = col.length {
            if let Some(scale) = col.scale {
                type_str = format!("{}({},{})", type_str, len, scale);
            } else {
                type_str = format!("{}({})", type_str, len);
            }
        }
        type_str
    }

    // === Import/Export Operations ===

    /// Build INSERT statement for a single row
    fn build_insert_statement(&self, table: &str, columns: &[String], values: &[String]) -> String {
        let mut sql = format!("INSERT INTO {} (", self.quote_identifier(table));
        for (i, col) in columns.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&self.quote_identifier(col));
        }
        sql.push_str(") VALUES (");
        for (i, val) in values.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&self.escape_sql_value(val));
        }
        sql.push(')');
        sql
    }

    /// Escape a string value for SQL (override for database-specific escaping)
    fn escape_sql_value(&self, value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    /// Import data from the specified format
    async fn import_data(
        &self,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
    ) -> Result<ImportResult> {
        self.import_data_with_progress(connection, config, data, "", None)
            .await
    }

    /// Import data with progress callback
    async fn import_data_with_progress(
        &self,
        connection: &dyn DbConnection,
        config: &ImportConfig,
        data: &str,
        file_name: &str,
        progress_tx: Option<ImportProgressSender>,
    ) -> Result<ImportResult>;

    /// Export data to the specified format
    async fn export_data(
        &self,
        connection: &dyn DbConnection,
        config: &ExportConfig,
    ) -> Result<ExportResult> {
        self.export_data_with_progress(connection, config, None)
            .await
    }

    /// Export data with progress callback
    async fn export_data_with_progress(
        &self,
        connection: &dyn DbConnection,
        config: &ExportConfig,
        progress_tx: Option<ExportProgressSender>,
    ) -> Result<ExportResult>;
}

/// Default column-based CREATE TABLE export shared by the `DatabasePlugin`
/// trait default. Driver overrides that need to opt back into the generic
/// builder (e.g. when the driver's own structure export is unavailable) call
/// this free function directly: a `Trait::method(self)` call from inside an
/// override would dispatch back to the override instead of the default body.
pub(crate) async fn default_export_table_create_sql<P>(
    plugin: &P,
    connection: &dyn DbConnection,
    database: &str,
    schema: Option<&str>,
    table: &str,
) -> Result<String>
where
    P: DatabasePlugin + ?Sized,
{
    let columns = plugin
        .list_columns(connection, database, schema.map(|s| s.to_string()), table)
        .await?;
    if columns.is_empty() {
        return Ok(String::new());
    }

    let table_ref = plugin.format_export_table_reference(database, schema, table);
    let mut definitions = columns
        .iter()
        .map(|column| format!("    {}", plugin.build_column_definition(column, true)))
        .collect::<Vec<_>>();
    let primary_keys = columns
        .iter()
        .filter(|column| column.is_primary_key)
        .map(|column| plugin.quote_identifier(&column.name))
        .collect::<Vec<_>>();
    if !primary_keys.is_empty() {
        definitions.push(format!("    PRIMARY KEY ({})", primary_keys.join(", ")));
    }

    let mut sql = format!(
        "CREATE TABLE {} (\n{}\n)",
        table_ref,
        definitions.join(",\n")
    );

    // Best-effort table comment: drivers that cannot list table metadata are
    // still able to export the structure (columns + primary key + column comments).
    let mut statements = Vec::new();
    if let Ok(tables) = plugin
        .list_tables(connection, database, schema.map(|s| s.to_string()))
        .await
    {
        if let Some(comment) = tables
            .iter()
            .find(|info| info.name == table)
            .and_then(|info| info.comment.clone())
        {
            statements.push(format!(
                "COMMENT ON TABLE {} IS {}",
                table_ref,
                plugin.escape_sql_value(&comment)
            ));
        }
    }
    statements.extend(columns.iter().filter_map(|column| {
        column.comment.as_ref().map(|comment| {
            format!(
                "COMMENT ON COLUMN {}.{} IS {}",
                table_ref,
                plugin.quote_identifier(&column.name),
                plugin.escape_sql_value(comment)
            )
        })
    }));
    if !statements.is_empty() {
        sql.push('\n');
        sql.push_str(&statements.join(";\n"));
    }
    Ok(sql)
}

fn foreign_key_action_sql(action: &str) -> Option<String> {
    let action = action.trim();
    if action.is_empty() {
        return None;
    }
    let action = action
        .split_whitespace()
        .map(str::to_ascii_uppercase)
        .collect::<Vec<_>>()
        .join(" ");
    match action.as_str() {
        "CASCADE" | "RESTRICT" | "NO ACTION" | "SET NULL" | "SET DEFAULT" => Some(action),
        _ => None,
    }
}

pub(crate) fn format_binary_literal_for_database(
    database_type: &DatabaseType,
    bytes: &[u8],
) -> String {
    crate::sql_literal::format_binary_literal_for_database(database_type, bytes)
}

/// 将 design 中被重命名的列名回退为旧名，以便与 original 做 diff 时不会产生误删/误增。
pub fn map_design_for_diff(
    design: &TableDesign,
    normalized_renames: &[(String, String)],
) -> TableDesign {
    let mut design_for_diff = design.clone();
    for (old_name, new_name) in normalized_renames {
        if let Some(column) = design_for_diff
            .columns
            .iter_mut()
            .find(|column| column.name == *new_name)
        {
            column.name = old_name.clone();
        }
        for index in &mut design_for_diff.indexes {
            for idx_col in &mut index.columns {
                if idx_col == new_name {
                    *idx_col = old_name.clone();
                }
            }
        }
    }
    design_for_diff
}

/// 合并 base ALTER SQL 和 rename 语句，跳过 "-- No changes" 前缀。
pub fn merge_alter_sql(base_sql: String, rename_statements: Vec<String>) -> String {
    let mut statements = Vec::new();
    let trimmed = base_sql.trim();
    if !trimmed.is_empty() && !trimmed.starts_with("-- No changes") {
        statements.push(trimmed.to_string());
    }
    statements.extend(rename_statements);

    if statements.is_empty() {
        "-- No changes detected".to_string()
    } else {
        statements.join("\n")
    }
}

/// Default import data implementation - can be called by database plugins
pub async fn default_import_data_with_progress(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ImportConfig,
    data: &str,
    file_name: &str,
    progress_tx: Option<ImportProgressSender>,
) -> Result<ImportResult> {
    match config.format {
        DataFormat::Sql => {
            SqlFormatHandler
                .import_with_progress(plugin, connection, config, data, file_name, progress_tx)
                .await
        }
        DataFormat::Json => {
            JsonFormatHandler
                .import_with_progress(plugin, connection, config, data, file_name, progress_tx)
                .await
        }
        DataFormat::Csv => {
            CsvFormatHandler
                .import_with_progress(plugin, connection, config, data, file_name, progress_tx)
                .await
        }
        DataFormat::Txt => {
            TxtFormatHandler
                .import_with_progress(plugin, connection, config, data, file_name, progress_tx)
                .await
        }
        DataFormat::Xml => {
            XmlFormatHandler
                .import_with_progress(plugin, connection, config, data, file_name, progress_tx)
                .await
        }
    }
}

/// Default export data implementation - can be called by database plugins
pub async fn default_export_data_with_progress(
    plugin: &dyn DatabasePlugin,
    connection: &dyn DbConnection,
    config: &ExportConfig,
    progress_tx: Option<ExportProgressSender>,
) -> Result<ExportResult> {
    match config.format {
        DataFormat::Sql => {
            SqlFormatHandler
                .export_with_progress(plugin, connection, config, progress_tx)
                .await
        }
        DataFormat::Json => {
            JsonFormatHandler
                .export_with_progress(plugin, connection, config, progress_tx)
                .await
        }
        DataFormat::Csv => {
            CsvFormatHandler
                .export_with_progress(plugin, connection, config, progress_tx)
                .await
        }
        DataFormat::Txt => {
            TxtFormatHandler
                .export_with_progress(plugin, connection, config, progress_tx)
                .await
        }
        DataFormat::Xml => {
            XmlFormatHandler
                .export_with_progress(plugin, connection, config, progress_tx)
                .await
        }
    }
}

pub fn is_query_stmt(stmt: &Statement) -> bool {
    matches!(
        stmt,
        Statement::Query(_)
            | Statement::ShowTables { .. }
            | Statement::ShowColumns { .. }
            | Statement::ShowDatabases { .. }
            | Statement::ShowFunctions { .. }
            | Statement::ShowVariable { .. }
            | Statement::ShowVariables { .. }
            | Statement::ShowCreate { .. }
            | Statement::ShowStatus { .. }
            | Statement::ShowCollation { .. }
            | Statement::ExplainTable { .. }
            | Statement::Explain { .. }
            | Statement::Pragma { .. }
    )
}

pub fn is_query_statement_fallback(sql: &str) -> bool {
    let trimmed = sql.trim().to_uppercase();
    trimmed.starts_with("SELECT")
        || trimmed.starts_with("SHOW")
        || trimmed.starts_with("DESC")
        || trimmed.starts_with("DESCRIBE")
        || trimmed.starts_with("EXPLAIN")
        || trimmed.starts_with("WITH")
        || trimmed.starts_with("TABLE")
        || trimmed.starts_with("PRAGMA")
}

pub fn classify_stmt(stmt: &Statement) -> StatementType {
    if is_query_stmt(stmt) {
        return StatementType::Query;
    }

    match stmt {
        Statement::Insert(_)
        | Statement::Update { .. }
        | Statement::Delete(_)
        | Statement::Merge { .. } => StatementType::Dml,

        Statement::CreateTable { .. }
        | Statement::CreateView { .. }
        | Statement::CreateIndex(_)
        | Statement::CreateFunction { .. }
        | Statement::CreateProcedure { .. }
        | Statement::CreateTrigger { .. }
        | Statement::CreateSchema { .. }
        | Statement::CreateDatabase { .. }
        | Statement::CreateSequence { .. }
        | Statement::AlterTable { .. }
        | Statement::AlterView { .. }
        | Statement::AlterIndex { .. }
        | Statement::Drop { .. }
        | Statement::DropFunction { .. }
        | Statement::DropProcedure { .. }
        | Statement::DropTrigger { .. }
        | Statement::DropSecret { .. }
        | Statement::Truncate { .. }
        | Statement::RenameTable { .. } => StatementType::Ddl,

        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. }
        | Statement::Savepoint { .. } => StatementType::Transaction,

        Statement::Use(_) | Statement::Set(_) => StatementType::Command,

        _ => StatementType::Exec,
    }
}

pub fn classify_fallback(sql: &str) -> StatementType {
    let trimmed = sql.trim().to_uppercase();

    if is_query_statement_fallback(sql) {
        return StatementType::Query;
    }

    if trimmed.starts_with("INSERT")
        || trimmed.starts_with("UPDATE")
        || trimmed.starts_with("DELETE")
        || trimmed.starts_with("REPLACE")
    {
        return StatementType::Dml;
    }

    if trimmed.starts_with("CREATE")
        || trimmed.starts_with("ALTER")
        || trimmed.starts_with("DROP")
        || trimmed.starts_with("TRUNCATE")
        || trimmed.starts_with("RENAME")
    {
        return StatementType::Ddl;
    }

    if trimmed.starts_with("BEGIN")
        || trimmed.starts_with("COMMIT")
        || trimmed.starts_with("ROLLBACK")
        || trimmed.starts_with("START TRANSACTION")
    {
        return StatementType::Transaction;
    }

    if trimmed.starts_with("USE") || trimmed.starts_with("SET") {
        return StatementType::Command;
    }

    StatementType::Exec
}

pub fn analyze_query_capabilities(query: &ast::Query) -> SelectQueryAnalysis {
    let select = match query.body.as_ref() {
        SetExpr::Select(select) => select,
        _ => return SelectQueryAnalysis::default(),
    };

    let Some(table_with_joins) = (select.from.len() == 1).then(|| &select.from[0]) else {
        return SelectQueryAnalysis::default();
    };
    if !table_with_joins.joins.is_empty() {
        return SelectQueryAnalysis::default();
    }

    let Some((table_name, alias)) = direct_table_identity(&table_with_joins.relation) else {
        return SelectQueryAnalysis::default();
    };

    let schema_metadata_safe =
        query.with.is_none() && select_projection_is_direct(select, &table_name, alias.as_deref());
    let editable = schema_metadata_safe
        && select.distinct.is_none()
        && !select_has_group_by(select)
        && select.having.is_none();

    SelectQueryAnalysis {
        table_name: Some(table_name),
        editable,
        schema_metadata_safe,
    }
}

/// Compatibility API retained for existing callers and tests.
pub fn analyze_query_editability(query: &Box<ast::Query>) -> Option<String> {
    let analysis = analyze_query_capabilities(query);
    analysis.editable.then_some(analysis.table_name).flatten()
}

fn direct_table_identity(relation: &TableFactor) -> Option<(String, Option<String>)> {
    let TableFactor::Table {
        name,
        alias,
        args,
        with_hints,
        version,
        with_ordinality,
        partitions,
        json_path,
        sample,
        index_hints,
    } = relation
    else {
        return None;
    };

    if args.is_some()
        || !with_hints.is_empty()
        || version.is_some()
        || *with_ordinality
        || !partitions.is_empty()
        || json_path.is_some()
        || sample.is_some()
        || !index_hints.is_empty()
        || name.0.is_empty()
    {
        return None;
    }

    // Build the table name from the unquoted identifier values. Using
    // `name.to_string()` would preserve the original quoting characters (e.g.
    // `` `ADDRESSBOOK` ``), which later get quoted again by `quote_identifier`
    // when generating INSERT/UPDATE/DELETE statements, producing doubled
    // quote symbols. `.value` holds the identifier without its quotes.
    let table_name = name
        .0
        .iter()
        .map(|part| part.as_ident().map(|ident| ident.value.as_str()))
        .collect::<Option<Vec<_>>>()?
        .join(".");

    Some((
        table_name,
        alias.as_ref().map(|alias| alias.name.value.clone()),
    ))
}

fn select_projection_is_direct(
    select: &ast::Select,
    table_name: &str,
    alias: Option<&str>,
) -> bool {
    if select.exclude.is_some() {
        return false;
    }

    select.projection.iter().all(|item| match item {
        ast::SelectItem::Wildcard(options) => wildcard_options_are_plain(options),
        ast::SelectItem::QualifiedWildcard(kind, options) => {
            wildcard_options_are_plain(options)
                && match kind {
                    ast::SelectItemQualifiedWildcardKind::ObjectName(name) => {
                        object_name_matches_table(name, table_name, alias)
                    }
                    ast::SelectItemQualifiedWildcardKind::Expr(_) => false,
                }
        }
        // Aliases change the result column name, so the source schema cannot
        // be mapped by name without additional result-column lineage support.
        ast::SelectItem::ExprWithAlias { .. } | ast::SelectItem::ExprWithAliases { .. } => false,
        ast::SelectItem::UnnamedExpr(expr) => direct_column_reference(expr, table_name, alias),
    })
}

fn wildcard_options_are_plain(options: &ast::WildcardAdditionalOptions) -> bool {
    options.opt_ilike.is_none()
        && options.opt_exclude.is_none()
        && options.opt_except.is_none()
        && options.opt_replace.is_none()
        && options.opt_rename.is_none()
}

fn direct_column_reference(expr: &Expr, table_name: &str, alias: Option<&str>) -> bool {
    match expr {
        Expr::Identifier(_) => true,
        Expr::CompoundIdentifier(parts) if parts.len() >= 2 => {
            let qualifier = parts[..parts.len() - 1]
                .iter()
                .map(|part| part.value.as_str())
                .collect::<Vec<_>>()
                .join(".");
            identifier_matches(&qualifier, table_name)
                || alias.is_some_and(|alias| identifier_matches(alias, &qualifier))
        }
        _ => false,
    }
}

fn object_name_matches_table(
    object_name: &ast::ObjectName,
    table_name: &str,
    alias: Option<&str>,
) -> bool {
    let qualified_name = object_name
        .0
        .iter()
        .map(|part| part.as_ident().map(|ident| ident.value.as_str()))
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join("."));
    qualified_name
        .as_deref()
        .is_some_and(|name| identifier_matches(name, table_name))
        || (object_name.0.len() == 1
            && alias.is_some_and(|alias| {
                qualified_name
                    .as_deref()
                    .is_some_and(|name| identifier_matches(name, alias))
            }))
}

fn identifier_matches(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn select_has_group_by(select: &ast::Select) -> bool {
    match &select.group_by {
        ast::GroupByExpr::All(_) => true,
        ast::GroupByExpr::Expressions(exprs, _) => !exprs.is_empty(),
    }
}

pub fn analyze_select_editability_fallback(sql: &str) -> Option<String> {
    let upper = sql.trim().to_uppercase();

    if !upper.starts_with("SELECT") {
        return None;
    }

    let complex_keywords = [
        " JOIN ",
        " INNER JOIN ",
        " LEFT JOIN ",
        " RIGHT JOIN ",
        " OUTER JOIN ",
        " CROSS JOIN ",
        " FULL JOIN ",
        " UNION ",
        " INTERSECT ",
        " EXCEPT ",
        " GROUP BY ",
        " HAVING ",
        "DISTINCT",
        " DISTINCT ",
    ];

    for keyword in &complex_keywords {
        if upper.contains(keyword) {
            return None;
        }
    }

    let aggregate_functions = [
        "COUNT(",
        "SUM(",
        "AVG(",
        "MAX(",
        "MIN(",
        "GROUP_CONCAT(",
        "STRING_AGG(",
    ];

    for func in &aggregate_functions {
        if upper.contains(func) {
            return None;
        }
    }

    if let Some(from_pos) = upper.find(" FROM ") {
        let after_from = &sql[from_pos + 6..].trim();
        let table_name = after_from
            .split_whitespace()
            .next()?
            .trim_end_matches(';')
            .trim_matches('`')
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();

        if table_name.contains('(') || table_name.contains(',') {
            return None;
        }

        return Some(table_name);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{ExecResult, SqlErrorInfo};
    use crate::mysql::MySqlPlugin;
    use sqlparser::dialect::MySqlDialect;
    use sqlparser::parser::Parser;

    // ==================== capabilities tests ====================

    #[test]
    fn default_capabilities_support_functions_and_procedures() {
        let plugin = MySqlPlugin::new();
        let capabilities = DatabasePlugin::capabilities(&plugin);
        assert!(capabilities.supports_functions);
        assert!(capabilities.supports_procedures);
    }

    #[test]
    fn database_user_operation_request_keeps_context() {
        let request = DatabaseUserOperationRequest {
            user_name: "alice".to_string(),
            host: Some("10.%".to_string()),
            database: Some("appdb".to_string()),
            field_values: HashMap::from([("password".to_string(), "secret".to_string())]),
        };

        assert_eq!("alice", request.user_name);
        assert_eq!(Some("10.%"), request.host.as_deref());
        assert_eq!(Some("appdb"), request.database.as_deref());
        assert_eq!(
            Some("secret"),
            request.field_values.get("password").map(String::as_str)
        );
    }

    #[test]
    fn mysql_plugin_exposes_split_plugin_traits() {
        let plugin = MySqlPlugin::new();
        assert_eq!(DatabaseType::MySQL, plugin.name());
        assert_eq!("`users`", plugin.quote_identifier("users"));
        assert!(plugin.capabilities().supports_functions);

        assert_eq!(" LIMIT 10 OFFSET 20", plugin.format_pagination(10, 20, ""));
    }

    #[test]
    fn table_data_total_count_requires_a_scalar_integer_query_result() {
        let valid = SqlResult::Query(QueryResult {
            sql: "SELECT COUNT(*)".to_string(),
            columns: vec!["count".to_string()],
            column_meta: vec![],
            rows: vec![vec![Some(" 42 ".to_string())]],
            binary_cells: vec![],
            elapsed_ms: 0,
        });
        assert_eq!(42, parse_table_data_total_count(valid).unwrap());

        let missing = SqlResult::Query(QueryResult {
            sql: "SELECT COUNT(*)".to_string(),
            columns: vec!["count".to_string()],
            column_meta: vec![],
            rows: vec![],
            binary_cells: vec![],
            elapsed_ms: 0,
        });
        assert!(parse_table_data_total_count(missing).is_err());

        let null = SqlResult::Query(QueryResult {
            sql: "SELECT COUNT(*)".to_string(),
            columns: vec!["count".to_string()],
            column_meta: vec![],
            rows: vec![vec![None]],
            binary_cells: vec![],
            elapsed_ms: 0,
        });
        assert!(parse_table_data_total_count(null).is_err());

        let invalid = SqlResult::Query(QueryResult {
            sql: "SELECT COUNT(*)".to_string(),
            columns: vec!["count".to_string()],
            column_meta: vec![],
            rows: vec![vec![Some("many".to_string())]],
            binary_cells: vec![],
            elapsed_ms: 0,
        });
        assert!(parse_table_data_total_count(invalid).is_err());

        let exec = SqlResult::Exec(ExecResult {
            sql: "SELECT COUNT(*)".to_string(),
            rows_affected: 0,
            elapsed_ms: 0,
            message: None,
        });
        assert!(parse_table_data_total_count(exec).is_err());

        let error = SqlResult::Error(SqlErrorInfo {
            sql: "SELECT COUNT(*)".to_string(),
            message: "count failed".to_string(),
        });
        assert_eq!(
            "count failed",
            parse_table_data_total_count(error).unwrap_err().to_string()
        );
    }

    #[test]
    fn copy_sql_preserves_null_empty_and_literal_null_text() {
        let plugin = MySqlPlugin::new();
        let columns = ["nullable", "empty", "literal"]
            .into_iter()
            .map(|name| ColumnInfo {
                name: name.to_string(),
                data_type: "VARCHAR".to_string(),
                is_nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
            })
            .collect();
        let row = vec![None, Some(String::new()), Some("NULL".to_string())];
        let request = CopySqlRequest::new("states", columns)
            .with_rows(vec![row.clone()])
            .with_original_rows(vec![row]);

        assert_eq!(
            plugin.generate_copy_insert_sql(&request),
            "INSERT INTO `states` (`nullable`, `empty`, `literal`) VALUES (NULL, '', 'NULL');"
        );
        assert_eq!(
            plugin.generate_copy_update_sql(&request),
            "UPDATE `states` SET `nullable` = NULL, `empty` = '', `literal` = 'NULL' WHERE `nullable` IS NULL AND `empty` = '' AND `literal` = 'NULL';"
        );
        assert_eq!(
            plugin.generate_copy_delete_sql(&request),
            "DELETE FROM `states` WHERE `nullable` IS NULL AND `empty` = '' AND `literal` = 'NULL';"
        );
    }

    #[test]
    fn copy_sql_uses_database_typed_literals() {
        let plugin = MySqlPlugin::new();
        let columns = vec![
            ColumnInfo {
                name: "id".to_string(),
                data_type: "INT".to_string(),
                is_nullable: false,
                is_primary_key: true,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
            },
            ColumnInfo {
                name: "enabled".to_string(),
                data_type: "BIT(1)".to_string(),
                is_nullable: false,
                is_primary_key: false,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
            },
        ];
        let request = CopySqlRequest::new("features", columns)
            .with_rows(vec![vec![Some("1".to_string()), Some("0".to_string())]]);

        assert_eq!(
            plugin.generate_copy_insert_sql(&request),
            "INSERT INTO `features` (`id`, `enabled`) VALUES (1, 0);"
        );
    }

    #[test]
    fn copy_sql_preserves_typed_binary_and_uses_original_values_for_where() {
        let plugin = MySqlPlugin::new();
        let columns = vec![
            ColumnInfo {
                name: "label".to_string(),
                data_type: "VARCHAR".to_string(),
                is_nullable: false,
                is_primary_key: true,
                default_value: None,
                comment: None,
                charset: Some("utf8mb4".to_string()),
                collation: Some("utf8mb4_0900_ai_ci".to_string()),
            },
            ColumnInfo {
                name: "payload".to_string(),
                data_type: "LONGBLOB".to_string(),
                is_nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
                charset: None,
                collation: Some("binary".to_string()),
            },
        ];
        let current = vec![
            TableCellValue::Text("changed".to_string()),
            TableCellValue::Binary(vec![1, 2, 3]),
        ];
        let original = vec![
            TableCellValue::Text("original".to_string()),
            TableCellValue::Binary(vec![4, 5, 6]),
        ];
        let request = CopySqlRequest::new("binary_values", columns)
            .with_typed_rows(vec![current])
            .with_typed_original_rows(vec![original]);

        assert_eq!(
            plugin.generate_copy_insert_sql(&request),
            "INSERT INTO `binary_values` (`label`, `payload`) VALUES ('changed', X'010203');"
        );
        assert_eq!(
            plugin.generate_copy_update_sql(&request),
            "UPDATE `binary_values` SET `label` = 'changed', `payload` = X'010203' WHERE `label` = 'original';"
        );
        assert_eq!(
            plugin.generate_copy_delete_sql(&request),
            "DELETE FROM `binary_values` WHERE `label` = 'original';"
        );
    }

    #[test]
    fn copy_sql_distinguishes_empty_binary_null_and_binary_like_text() {
        let plugin = MySqlPlugin::new();
        let columns = [
            "empty_binary",
            "nullable",
            "plain_true",
            "plain_number",
            "plain_base64",
        ]
        .into_iter()
        .map(|name| ColumnInfo {
            name: name.to_string(),
            data_type: "VARCHAR".to_string(),
            is_nullable: true,
            is_primary_key: false,
            default_value: None,
            comment: None,
            charset: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_0900_ai_ci".to_string()),
        })
        .collect();
        let request = CopySqlRequest::new("typed_values", columns).with_typed_rows(vec![vec![
            TableCellValue::Binary(Vec::new()),
            TableCellValue::Null,
            TableCellValue::Text("true".to_string()),
            TableCellValue::Text("8000".to_string()),
            TableCellValue::Text("AQID".to_string()),
        ]]);

        assert_eq!(
            plugin.generate_copy_insert_sql(&request),
            "INSERT INTO `typed_values` (`empty_binary`, `nullable`, `plain_true`, `plain_number`, `plain_base64`) VALUES (X'', NULL, 'true', '8000', 'AQID');"
        );
    }

    #[test]
    fn copy_sql_rejects_malformed_row_shapes_and_never_uses_unbounded_where() {
        let plugin = MySqlPlugin::new();
        let columns = vec![ColumnInfo {
            name: "id".to_string(),
            data_type: "INT".to_string(),
            is_nullable: false,
            is_primary_key: true,
            default_value: None,
            comment: None,
            charset: None,
            collation: None,
        }];

        let mismatched = CopySqlRequest::new("items", columns.clone())
            .with_typed_rows(vec![
                vec![TableCellValue::Text("1".to_string())],
                vec![TableCellValue::Text("2".to_string())],
            ])
            .with_typed_original_rows(vec![vec![TableCellValue::Text("1".to_string())]]);
        assert!(plugin.generate_copy_update_sql(&mismatched).is_empty());

        let missing_key = CopySqlRequest::new("items", columns.clone())
            .with_typed_rows(vec![vec![TableCellValue::Text("changed".to_string())]])
            .with_typed_original_rows(vec![vec![]]);
        assert!(plugin.generate_copy_update_sql(&missing_key).is_empty());
        assert!(plugin.generate_copy_delete_sql(&missing_key).is_empty());

        let malformed_insert = CopySqlRequest::new("items", columns).with_typed_rows(vec![vec![]]);
        assert!(
            plugin
                .generate_copy_insert_sql(&malformed_insert)
                .is_empty()
        );
        assert!(
            plugin
                .generate_copy_insert_with_comments_sql(&malformed_insert)
                .is_empty()
        );
    }

    #[test]
    fn copy_delete_can_use_original_rows_when_current_rows_are_empty() {
        let plugin = MySqlPlugin::new();
        let columns = vec![ColumnInfo {
            name: "id".to_string(),
            data_type: "INT".to_string(),
            is_nullable: false,
            is_primary_key: true,
            default_value: None,
            comment: None,
            charset: None,
            collation: None,
        }];
        let request = CopySqlRequest::new("items", columns)
            .with_typed_original_rows(vec![vec![TableCellValue::Text("42".to_string())]]);

        assert_eq!(
            plugin.generate_copy_delete_sql(&request),
            "DELETE FROM `items` WHERE `id` = 42;"
        );
    }

    #[test]
    fn legacy_string_insert_builder_never_treats_text_as_sql_null() {
        let plugin = MySqlPlugin::new();
        let columns = vec!["empty".to_string(), "literal".to_string()];
        let values = vec![String::new(), "NULL".to_string()];

        assert_eq!(
            plugin.build_insert_statement("states", &columns, &values),
            "INSERT INTO `states` (`empty`, `literal`) VALUES ('', 'NULL')"
        );
    }

    // ==================== is_query_stmt tests (AST-based) ====================

    #[test]
    fn test_is_query_stmt_select() {
        let stmts = Parser::parse_sql(&MySqlDialect {}, "SELECT * FROM users").unwrap();
        assert!(is_query_stmt(&stmts[0]));
    }

    #[test]
    fn test_is_query_stmt_show() {
        let stmts = Parser::parse_sql(&MySqlDialect {}, "SHOW TABLES").unwrap();
        assert!(is_query_stmt(&stmts[0]));
    }

    #[test]
    fn test_is_query_stmt_explain() {
        let stmts = Parser::parse_sql(&MySqlDialect {}, "EXPLAIN SELECT * FROM users").unwrap();
        assert!(is_query_stmt(&stmts[0]));
    }

    #[test]
    fn test_is_query_stmt_insert() {
        let stmts = Parser::parse_sql(&MySqlDialect {}, "INSERT INTO users VALUES (1)").unwrap();
        assert!(!is_query_stmt(&stmts[0]));
    }

    #[test]
    fn test_is_query_stmt_update() {
        let stmts = Parser::parse_sql(&MySqlDialect {}, "UPDATE users SET name = 'test'").unwrap();
        assert!(!is_query_stmt(&stmts[0]));
    }

    #[test]
    fn test_is_query_stmt_delete() {
        let stmts = Parser::parse_sql(&MySqlDialect {}, "DELETE FROM users").unwrap();
        assert!(!is_query_stmt(&stmts[0]));
    }

    // ==================== is_query_statement_fallback tests ====================

    #[test]
    fn test_is_query_statement_fallback_select() {
        assert!(is_query_statement_fallback("SELECT * FROM users"));
        assert!(is_query_statement_fallback("  select id from t  "));
    }

    #[test]
    fn test_is_query_statement_fallback_show() {
        assert!(is_query_statement_fallback("SHOW TABLES"));
        assert!(is_query_statement_fallback("SHOW DATABASES"));
    }

    #[test]
    fn test_is_query_statement_fallback_describe() {
        assert!(is_query_statement_fallback("DESCRIBE users"));
        assert!(is_query_statement_fallback("DESC users"));
    }

    #[test]
    fn test_is_query_statement_fallback_explain() {
        assert!(is_query_statement_fallback("EXPLAIN SELECT * FROM users"));
    }

    #[test]
    fn test_is_query_statement_fallback_with() {
        assert!(is_query_statement_fallback(
            "WITH cte AS (SELECT 1) SELECT * FROM cte"
        ));
    }

    #[test]
    fn test_is_query_statement_fallback_pragma() {
        assert!(is_query_statement_fallback("PRAGMA table_info(users)"));
    }

    #[test]
    fn test_is_query_statement_fallback_non_query() {
        assert!(!is_query_statement_fallback("INSERT INTO users VALUES (1)"));
        assert!(!is_query_statement_fallback(
            "UPDATE users SET name = 'test'"
        ));
        assert!(!is_query_statement_fallback("DELETE FROM users"));
        assert!(!is_query_statement_fallback("CREATE TABLE t (id INT)"));
    }

    #[test]
    fn test_split_sql_statements_ignores_comment_only_script() {
        let plugin = MySqlPlugin::new();
        let sql = r#"/**
 sql脚本文件命名规则:
 V: 前缀
 1: 自增长序列，新增的以 2 开始
 readme: 本文档的 版本号等说明
 版本号 V8.0SP2
 */

 ------------------------------------------------------------------------
"#;

        assert!(DatabasePlugin::split_sql_statements(&plugin, sql).is_empty());
    }

    // ==================== classify_stmt tests (AST-based) ====================

    #[test]
    fn test_classify_stmt_query() {
        let stmts = Parser::parse_sql(&MySqlDialect {}, "SELECT * FROM users").unwrap();
        assert_eq!(classify_stmt(&stmts[0]), StatementType::Query);
    }

    #[test]
    fn test_classify_stmt_dml() {
        let insert = Parser::parse_sql(&MySqlDialect {}, "INSERT INTO users VALUES (1)").unwrap();
        assert_eq!(classify_stmt(&insert[0]), StatementType::Dml);

        let update = Parser::parse_sql(&MySqlDialect {}, "UPDATE users SET name = 'test'").unwrap();
        assert_eq!(classify_stmt(&update[0]), StatementType::Dml);

        let delete = Parser::parse_sql(&MySqlDialect {}, "DELETE FROM users").unwrap();
        assert_eq!(classify_stmt(&delete[0]), StatementType::Dml);
    }

    #[test]
    fn test_classify_stmt_ddl() {
        let create = Parser::parse_sql(&MySqlDialect {}, "CREATE TABLE t (id INT)").unwrap();
        assert_eq!(classify_stmt(&create[0]), StatementType::Ddl);

        let alter = Parser::parse_sql(
            &MySqlDialect {},
            "ALTER TABLE t ADD COLUMN name VARCHAR(100)",
        )
        .unwrap();
        assert_eq!(classify_stmt(&alter[0]), StatementType::Ddl);

        let drop = Parser::parse_sql(&MySqlDialect {}, "DROP TABLE t").unwrap();
        assert_eq!(classify_stmt(&drop[0]), StatementType::Ddl);
    }

    #[test]
    fn test_classify_stmt_transaction() {
        let commit = Parser::parse_sql(&MySqlDialect {}, "COMMIT").unwrap();
        assert_eq!(classify_stmt(&commit[0]), StatementType::Transaction);

        let rollback = Parser::parse_sql(&MySqlDialect {}, "ROLLBACK").unwrap();
        assert_eq!(classify_stmt(&rollback[0]), StatementType::Transaction);
    }

    #[test]
    fn test_classify_stmt_command() {
        let use_stmt = Parser::parse_sql(&MySqlDialect {}, "USE mydb").unwrap();
        assert_eq!(classify_stmt(&use_stmt[0]), StatementType::Command);

        let set = Parser::parse_sql(&MySqlDialect {}, "SET autocommit = 1").unwrap();
        assert_eq!(classify_stmt(&set[0]), StatementType::Command);
    }

    // ==================== classify_fallback tests ====================

    #[test]
    fn test_classify_fallback_query() {
        assert_eq!(
            classify_fallback("SELECT * FROM users"),
            StatementType::Query
        );
        assert_eq!(classify_fallback("SHOW TABLES"), StatementType::Query);
        assert_eq!(classify_fallback("DESCRIBE users"), StatementType::Query);
    }

    #[test]
    fn test_classify_fallback_dml() {
        assert_eq!(
            classify_fallback("INSERT INTO users VALUES (1)"),
            StatementType::Dml
        );
        assert_eq!(
            classify_fallback("UPDATE users SET name = 'test'"),
            StatementType::Dml
        );
        assert_eq!(classify_fallback("DELETE FROM users"), StatementType::Dml);
        assert_eq!(
            classify_fallback("REPLACE INTO users VALUES (1)"),
            StatementType::Dml
        );
    }

    #[test]
    fn test_classify_fallback_ddl() {
        assert_eq!(
            classify_fallback("CREATE TABLE users (id INT)"),
            StatementType::Ddl
        );
        assert_eq!(
            classify_fallback("ALTER TABLE users ADD COLUMN name VARCHAR(100)"),
            StatementType::Ddl
        );
        assert_eq!(classify_fallback("DROP TABLE users"), StatementType::Ddl);
        assert_eq!(
            classify_fallback("TRUNCATE TABLE users"),
            StatementType::Ddl
        );
        assert_eq!(
            classify_fallback("RENAME TABLE old TO new"),
            StatementType::Ddl
        );
    }

    #[test]
    fn test_classify_fallback_transaction() {
        assert_eq!(classify_fallback("BEGIN"), StatementType::Transaction);
        assert_eq!(classify_fallback("COMMIT"), StatementType::Transaction);
        assert_eq!(classify_fallback("ROLLBACK"), StatementType::Transaction);
        assert_eq!(
            classify_fallback("START TRANSACTION"),
            StatementType::Transaction
        );
    }

    #[test]
    fn test_classify_fallback_command() {
        assert_eq!(classify_fallback("USE mydb"), StatementType::Command);
        assert_eq!(
            classify_fallback("SET autocommit = 1"),
            StatementType::Command
        );
    }

    #[test]
    fn test_classify_fallback_exec() {
        assert_eq!(
            classify_fallback("CALL my_procedure()"),
            StatementType::Exec
        );
        assert_eq!(
            classify_fallback("EXECUTE my_statement"),
            StatementType::Exec
        );
    }

    // ==================== analyze_query_editability tests (AST-based) ====================

    #[test]
    fn test_analyze_query_editability_simple() {
        let stmts = Parser::parse_sql(&MySqlDialect {}, "SELECT * FROM users").unwrap();
        if let Statement::Query(query) = &stmts[0] {
            let result = analyze_query_editability(query);
            assert!(result.is_some());
            assert!(result.unwrap().contains("users"));
        }
    }

    #[test]
    fn test_analyze_query_editability_with_where() {
        let stmts =
            Parser::parse_sql(&MySqlDialect {}, "SELECT * FROM users WHERE id = 1").unwrap();
        if let Statement::Query(query) = &stmts[0] {
            let result = analyze_query_editability(query);
            assert!(result.is_some());
        }
    }

    #[test]
    fn test_analyze_query_editability_with_join() {
        let stmts = Parser::parse_sql(
            &MySqlDialect {},
            "SELECT * FROM users JOIN orders ON users.id = orders.user_id",
        )
        .unwrap();
        if let Statement::Query(query) = &stmts[0] {
            let result = analyze_query_editability(query);
            assert!(result.is_none());
        }
    }

    #[test]
    fn test_analyze_query_editability_with_group_by() {
        let stmts = Parser::parse_sql(
            &MySqlDialect {},
            "SELECT name, COUNT(*) FROM users GROUP BY name",
        )
        .unwrap();
        if let Statement::Query(query) = &stmts[0] {
            let result = analyze_query_editability(query);
            assert!(result.is_none());
        }
    }

    #[test]
    fn test_analyze_query_editability_with_distinct() {
        let stmts = Parser::parse_sql(&MySqlDialect {}, "SELECT DISTINCT name FROM users").unwrap();
        if let Statement::Query(query) = &stmts[0] {
            let result = analyze_query_editability(query);
            assert!(result.is_none());
        }
    }

    #[test]
    fn test_analyze_query_editability_with_aggregate() {
        let stmts = Parser::parse_sql(&MySqlDialect {}, "SELECT COUNT(*) FROM users").unwrap();
        if let Statement::Query(query) = &stmts[0] {
            let result = analyze_query_editability(query);
            assert!(result.is_none());
        }
    }

    #[test]
    fn test_analyze_select_query_does_not_preserve_quotes_in_table_name() {
        let plugin = MySqlPlugin::new();

        // Backtick-quoted table name must not carry the quote characters into
        // the analyzed table name (otherwise `quote_identifier` doubles them
        // when generating UPDATE/INSERT/DELETE statements).
        let analysis = plugin.analyze_select_query("SELECT * FROM `ADDRESSBOOK`");
        assert_eq!(analysis.table_name.as_deref(), Some("ADDRESSBOOK"));
        assert!(analysis.editable);
        assert!(analysis.schema_metadata_safe);

        // Quoted qualified name keeps the dotted structure but drops quotes.
        let analysis = plugin.analyze_select_query("SELECT * FROM `ai_app`.`ADDRESSBOOK`");
        assert_eq!(analysis.table_name.as_deref(), Some("ai_app.ADDRESSBOOK"));
        assert!(analysis.editable);

        // Unquoted names are unaffected.
        let analysis = plugin.analyze_select_query("SELECT * FROM users");
        assert_eq!(analysis.table_name.as_deref(), Some("users"));
    }

    #[test]
    fn test_analyze_select_query_unquotes_other_dialect_identifiers() {
        let plugin = MySqlPlugin::new();

        // Double-quoted identifiers (PostgreSQL/Oracle/SQLite/DuckDB style).
        let analysis = plugin.analyze_select_query("SELECT * FROM \"orders\"");
        assert_eq!(analysis.table_name.as_deref(), Some("orders"));

        // Escaped backtick inside a quoted identifier must be unescaped to the
        // real value (`` a``b `` is the table named `a`b`).
        let analysis = plugin.analyze_select_query("SELECT * FROM `a``b`");
        assert_eq!(analysis.table_name.as_deref(), Some("a`b"));

        // Bracket-quoted identifiers (MSSQL style) need the MSSQL dialect so
        // the parser recognizes `[orders]` as a quoted identifier.
        let plugin = crate::mssql::MsSqlPlugin::new();
        let analysis = plugin.analyze_select_query("SELECT * FROM [orders]");
        assert_eq!(analysis.table_name.as_deref(), Some("orders"));

        // Double-quoted identifiers in a PostgreSQL dialect.
        let plugin = crate::postgresql::PostgresPlugin::new();
        let analysis = plugin.analyze_select_query("SELECT * FROM \"orders\"");
        assert_eq!(analysis.table_name.as_deref(), Some("orders"));
    }

    // ==================== analyze_select_editability_fallback tests ====================

    #[test]
    fn test_analyze_select_editability_fallback_simple() {
        assert_eq!(
            analyze_select_editability_fallback("SELECT * FROM users"),
            Some("users".to_string())
        );
    }

    #[test]
    fn test_analyze_select_editability_fallback_quoted() {
        assert_eq!(
            analyze_select_editability_fallback("SELECT * FROM `users`"),
            Some("users".to_string())
        );
        assert_eq!(
            analyze_select_editability_fallback("SELECT * FROM \"users\""),
            Some("users".to_string())
        );
    }

    #[test]
    fn test_analyze_select_editability_fallback_with_where() {
        assert_eq!(
            analyze_select_editability_fallback("SELECT * FROM users WHERE id = 1"),
            Some("users".to_string())
        );
    }

    #[test]
    fn test_analyze_select_editability_fallback_with_join() {
        assert_eq!(
            analyze_select_editability_fallback(
                "SELECT * FROM users JOIN orders ON users.id = orders.user_id"
            ),
            None
        );
        assert_eq!(
            analyze_select_editability_fallback("SELECT * FROM users INNER JOIN orders"),
            None
        );
        assert_eq!(
            analyze_select_editability_fallback("SELECT * FROM users LEFT JOIN orders"),
            None
        );
    }

    #[test]
    fn test_analyze_select_editability_fallback_with_group_by() {
        assert_eq!(
            analyze_select_editability_fallback("SELECT * FROM users GROUP BY name"),
            None
        );
    }

    #[test]
    fn test_analyze_select_editability_fallback_with_aggregate() {
        assert_eq!(
            analyze_select_editability_fallback("SELECT COUNT(*) FROM users"),
            None
        );
        assert_eq!(
            analyze_select_editability_fallback("SELECT SUM(amount) FROM orders"),
            None
        );
        assert_eq!(
            analyze_select_editability_fallback("SELECT AVG(price) FROM products"),
            None
        );
    }

    #[test]
    fn test_analyze_select_editability_fallback_with_distinct() {
        assert_eq!(
            analyze_select_editability_fallback("SELECT DISTINCT * FROM users"),
            None
        );
        assert_eq!(
            analyze_select_editability_fallback("SELECT DISTINCT name FROM users"),
            None
        );
    }

    #[test]
    fn test_analyze_select_editability_fallback_with_union() {
        assert_eq!(
            analyze_select_editability_fallback("SELECT * FROM users UNION SELECT * FROM admins"),
            None
        );
    }

    #[test]
    fn test_analyze_select_editability_fallback_non_select() {
        assert_eq!(
            analyze_select_editability_fallback("INSERT INTO users VALUES (1)"),
            None
        );
        assert_eq!(
            analyze_select_editability_fallback("UPDATE users SET name = 'test'"),
            None
        );
    }
}
