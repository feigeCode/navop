use crate::duckdb_session::DuckDbSession;
use anyhow::Result;
use duckdb::Connection;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Serialize)]
struct DatabaseInfo {
    name: String,
    charset: Option<String>,
    collation: Option<String>,
    size: Option<String>,
    table_count: Option<i64>,
    comment: Option<String>,
}

#[derive(Debug, Serialize)]
struct TableInfo {
    name: String,
    schema: Option<String>,
    comment: Option<String>,
    engine: Option<String>,
    row_count: Option<i64>,
    create_time: Option<String>,
    charset: Option<String>,
    collation: Option<String>,
}

#[derive(Debug, Serialize)]
struct ColumnInfo {
    name: String,
    data_type: String,
    is_nullable: bool,
    is_primary_key: bool,
    default_value: Option<String>,
    comment: Option<String>,
    charset: Option<String>,
    collation: Option<String>,
}

#[derive(Debug, Serialize)]
struct IndexInfo {
    name: String,
    columns: Vec<String>,
    is_unique: bool,
    index_type: Option<String>,
}

#[derive(Debug, Serialize)]
struct ViewInfo {
    name: String,
    schema: Option<String>,
    definition: Option<String>,
    comment: Option<String>,
}

pub fn handle(session: &DuckDbSession, method: &str, params: &Value) -> Result<Option<Value>> {
    let connection = session.connection()?;
    match method {
        "metadata.list_databases" => Ok(Some(json!(vec!["main"]))),
        "metadata.list_databases_detailed" => to_value(list_databases_detailed()).map(Some),
        "metadata.list_schemas" => to_value(list_schemas(connection)?).map(Some),
        "metadata.list_tables" => to_value(list_tables(connection, params)?).map(Some),
        "metadata.list_columns" => to_value(list_columns(connection, params)?).map(Some),
        "metadata.list_indexes" => to_value(list_indexes(connection, params)?).map(Some),
        "metadata.list_views" => to_value(list_views(connection, params)?).map(Some),
        "metadata.list_functions"
        | "metadata.list_procedures"
        | "metadata.list_triggers"
        | "metadata.list_sequences"
        | "metadata.list_foreign_keys"
        | "metadata.list_table_triggers"
        | "metadata.list_table_checks" => Ok(Some(json!([]))),
        _ => Ok(None),
    }
}

fn list_databases_detailed() -> Vec<DatabaseInfo> {
    vec![DatabaseInfo {
        name: "main".to_string(),
        charset: None,
        collation: None,
        size: None,
        table_count: None,
        comment: None,
    }]
}

fn list_schemas(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection
        .prepare("SELECT schema_name FROM information_schema.schemata ORDER BY schema_name")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    collect_rows(rows)
}

fn list_tables(connection: &Connection, params: &Value) -> Result<Vec<TableInfo>> {
    let filters = metadata_filters(params, "database_name", "schema_name");
    let sql = format!(
        "SELECT table_name, schema_name FROM duckdb_tables() WHERE internal = FALSE \
         AND temporary = FALSE{} ORDER BY schema_name, table_name",
        filters.sql
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(duckdb::params_from_iter(filters.values.iter()), |row| {
        Ok(TableInfo {
            name: row.get(0)?,
            schema: row.get(1)?,
            comment: None,
            engine: None,
            row_count: None,
            create_time: None,
            charset: None,
            collation: None,
        })
    })?;
    collect_rows(rows)
}

fn list_columns(connection: &Connection, params: &Value) -> Result<Vec<ColumnInfo>> {
    let table = string_param(params, "table")?;
    let filters = metadata_filters(params, "c.database_name", "c.schema_name");
    let sql = format!(
        "SELECT c.column_name, c.data_type, c.is_nullable, \
         (pk.column_name IS NOT NULL) AS is_primary_key, c.column_default \
         FROM duckdb_columns() AS c \
         LEFT JOIN ( \
           SELECT DISTINCT kcu.table_schema, kcu.table_name, kcu.column_name \
           FROM information_schema.table_constraints AS tc \
           JOIN information_schema.key_column_usage AS kcu \
             ON tc.constraint_name = kcu.constraint_name \
            AND tc.table_schema = kcu.table_schema \
            AND tc.table_name = kcu.table_name \
           WHERE tc.constraint_type = 'PRIMARY KEY' \
         ) AS pk ON pk.table_schema = c.schema_name \
          AND pk.table_name = c.table_name AND pk.column_name = c.column_name \
         WHERE c.table_name = ? AND c.internal = FALSE{} ORDER BY c.column_index",
        filters.sql
    );
    let mut values = vec![table.to_string()];
    values.extend(filters.values);
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(duckdb::params_from_iter(values.iter()), |row| {
        Ok(ColumnInfo {
            name: row.get(0)?,
            data_type: row.get(1)?,
            is_nullable: row.get(2)?,
            is_primary_key: row.get(3)?,
            default_value: row.get(4)?,
            comment: None,
            charset: None,
            collation: None,
        })
    })?;
    collect_rows(rows)
}

fn list_indexes(connection: &Connection, params: &Value) -> Result<Vec<IndexInfo>> {
    let table = string_param(params, "table")?;
    let filters = metadata_filters(params, "database_name", "schema_name");
    let sql = format!(
        "SELECT index_name, is_unique, sql FROM duckdb_indexes() \
         WHERE table_name = ?{} ORDER BY index_name",
        filters.sql
    );
    let mut values = vec![table.to_string()];
    values.extend(filters.values);
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(duckdb::params_from_iter(values.iter()), |row| {
        let sql: Option<String> = row.get(2)?;
        Ok(IndexInfo {
            name: row.get(0)?,
            is_unique: row.get(1)?,
            columns: sql.map(|sql| parse_index_columns(&sql)).unwrap_or_default(),
            index_type: None,
        })
    })?;
    collect_rows(rows)
}

fn list_views(connection: &Connection, params: &Value) -> Result<Vec<ViewInfo>> {
    let filters = metadata_filters(params, "database_name", "schema_name");
    let sql = format!(
        "SELECT view_name, schema_name, sql FROM duckdb_views() WHERE internal = FALSE \
         AND temporary = FALSE{} ORDER BY schema_name, view_name",
        filters.sql
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(duckdb::params_from_iter(filters.values.iter()), |row| {
        Ok(ViewInfo {
            name: row.get(0)?,
            schema: row.get(1)?,
            definition: row.get(2)?,
            comment: None,
        })
    })?;
    collect_rows(rows)
}

struct Filters {
    sql: String,
    values: Vec<String>,
}

fn metadata_filters(params: &Value, database_column: &str, schema_column: &str) -> Filters {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    if let Some(database) = params
        .get("database")
        .and_then(Value::as_str)
        .filter(|database| !database.is_empty() && *database != "main")
    {
        clauses.push(format!(" AND {database_column} = ?"));
        values.push(database.to_string());
    }
    if let Some(schema) = params
        .get("schema")
        .and_then(Value::as_str)
        .filter(|schema| should_filter_schema(schema))
    {
        clauses.push(format!(" AND {schema_column} = ?"));
        values.push(schema.to_string());
    }
    Filters {
        sql: clauses.join(""),
        values,
    }
}

fn parse_index_columns(sql: &str) -> Vec<String> {
    let Some(open) = sql.rfind('(') else {
        return Vec::new();
    };
    let Some(close) = sql.rfind(')') else {
        return Vec::new();
    };
    if close <= open {
        return Vec::new();
    }
    sql[open + 1..close]
        .split(',')
        .map(|column| column.trim().trim_matches('"').to_string())
        .filter(|column| !column.is_empty())
        .collect()
}

fn string_param<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("{key} is required"))
}

fn should_filter_schema(schema: &str) -> bool {
    !schema.trim().is_empty()
}

fn collect_rows<T>(rows: impl Iterator<Item = duckdb::Result<T>>) -> Result<Vec<T>> {
    rows.collect::<duckdb::Result<Vec<_>>>().map_err(Into::into)
}

fn to_value(value: impl Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_index_columns_from_sql() {
        let columns = parse_index_columns("CREATE INDEX idx ON users (id, \"name\")");

        assert_eq!(columns, vec!["id".to_string(), "name".to_string()]);
    }
}
