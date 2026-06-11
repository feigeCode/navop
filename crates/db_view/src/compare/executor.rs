use db::compare::{
    ColumnSchema, DataCompareOptions, DataCompareResult, ForeignKeySchema, IndexSchema, RowData,
    SchemaCompareOptions, SchemaCompareResult, SyncPlan, TableSchema, build_data_sync_plan,
    build_schema_sync_plan, compare_data_rows, compare_schemas,
};
use db::{
    ColumnInfo, FieldType, ForeignKeyDefinition, GlobalDbState, IndexInfo, QueryColumnMeta,
    QueryResult, TableDataRequest, TableDataResponse, TableInfo,
};
use gpui::AsyncApp;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::compare::CompareProgress;

const DATA_COMPARE_PAGE_SIZE: usize = 10_000;

/// 数据比较的阶段总数(加载源列、目标列、源数据、目标数据、比较)
const DATA_COMPARE_TOTAL_STEPS: usize = 5;

fn report(progress_tx: &mpsc::UnboundedSender<CompareProgress>, progress: CompareProgress) {
    // 接收端可能因取消而提前关闭,忽略发送错误
    let _ = progress_tx.send(progress);
}

/// 数据比较任务参数
#[derive(Debug, Clone)]
pub struct DataCompareParams {
    pub source_connection_id: String,
    pub source_database: String,
    pub source_schema: Option<String>,
    pub source_table: String,
    pub target_connection_id: String,
    pub target_database: String,
    pub target_schema: Option<String>,
    pub target_table: String,
    pub key_columns: Vec<String>,
}

/// 结构比较任务参数
#[derive(Debug, Clone)]
pub struct SchemaCompareParams {
    pub source_connection_id: String,
    pub source_database: String,
    pub source_schema: Option<String>,
    pub target_connection_id: String,
    pub target_database: String,
    pub target_schema: Option<String>,
}

/// 执行数据比较任务（简化版本）
pub async fn execute_data_compare(
    params: DataCompareParams,
    db_state: Arc<GlobalDbState>,
    progress_tx: mpsc::UnboundedSender<CompareProgress>,
    cx: &mut AsyncApp,
) -> anyhow::Result<DataCompareResult> {
    report(
        &progress_tx,
        CompareProgress::steps("正在读取源表结构", 1, DATA_COMPARE_TOTAL_STEPS),
    );
    let source_columns = load_table_columns(
        &db_state,
        cx,
        &params.source_connection_id,
        &params.source_database,
        params.source_schema.clone(),
        &params.source_table,
    )
    .await?;
    report(
        &progress_tx,
        CompareProgress::steps("正在读取目标表结构", 2, DATA_COMPARE_TOTAL_STEPS),
    );
    let target_columns = load_table_columns(
        &db_state,
        cx,
        &params.target_connection_id,
        &params.target_database,
        params.target_schema.clone(),
        &params.target_table,
    )
    .await?;
    let key_columns = resolve_key_columns(&params.key_columns, &source_columns, &target_columns)?;

    report(
        &progress_tx,
        CompareProgress::steps("正在加载源表数据", 3, DATA_COMPARE_TOTAL_STEPS),
    );
    let source_response =
        load_table_data(&db_state, cx, SourceSide::Source, &params, &key_columns).await?;
    report(
        &progress_tx,
        CompareProgress::steps("正在加载目标表数据", 4, DATA_COMPARE_TOTAL_STEPS),
    );
    let target_response =
        load_table_data(&db_state, cx, SourceSide::Target, &params, &key_columns).await?;

    report(
        &progress_tx,
        CompareProgress::steps("正在比较数据", 5, DATA_COMPARE_TOTAL_STEPS),
    );
    build_data_compare_result(params, key_columns, source_response, target_response)
}

/// 生成数据同步计划
pub fn generate_data_sync_plan(result: &DataCompareResult) -> SyncPlan {
    build_data_sync_plan(result)
}

/// 执行结构比较任务（简化版本）
pub async fn execute_schema_compare(
    params: SchemaCompareParams,
    db_state: Arc<GlobalDbState>,
    progress_tx: mpsc::UnboundedSender<CompareProgress>,
    cx: &mut AsyncApp,
) -> anyhow::Result<SchemaCompareResult> {
    let options = SchemaCompareOptions::default();
    let source_tables = load_schema_tables(
        &db_state,
        cx,
        params.source_connection_id,
        params.source_database,
        params.source_schema,
        &progress_tx,
        "源",
    )
    .await?;
    let target_tables = load_schema_tables(
        &db_state,
        cx,
        params.target_connection_id,
        params.target_database,
        params.target_schema,
        &progress_tx,
        "目标",
    )
    .await?;
    report(&progress_tx, CompareProgress::phase("正在比较结构"));
    let result = compare_schemas(source_tables, target_tables, options)?;
    Ok(result)
}

/// 生成结构同步计划
pub fn generate_schema_sync_plan(result: &SchemaCompareResult, target_db_type: &str) -> SyncPlan {
    build_schema_sync_plan(result, target_db_type)
}

#[derive(Debug, Clone, Copy)]
enum SourceSide {
    Source,
    Target,
}

async fn load_table_columns(
    db_state: &Arc<GlobalDbState>,
    cx: &mut AsyncApp,
    connection_id: &str,
    database: &str,
    schema: Option<String>,
    table: &str,
) -> anyhow::Result<Vec<ColumnInfo>> {
    db_state
        .list_columns(
            cx,
            connection_id.to_string(),
            database.to_string(),
            schema,
            table.to_string(),
        )
        .await
}

async fn load_table_data(
    db_state: &Arc<GlobalDbState>,
    cx: &mut AsyncApp,
    side: SourceSide,
    params: &DataCompareParams,
    key_columns: &[String],
) -> anyhow::Result<TableDataResponse> {
    let (connection_id, database, schema, table) = match side {
        SourceSide::Source => (
            &params.source_connection_id,
            &params.source_database,
            params.source_schema.clone(),
            &params.source_table,
        ),
        SourceSide::Target => (
            &params.target_connection_id,
            &params.target_database,
            params.target_schema.clone(),
            &params.target_table,
        ),
    };

    db_state
        .query_table_data(
            cx,
            connection_id.clone(),
            table_data_request(database.clone(), schema, table.clone(), key_columns),
        )
        .await
}

fn build_data_compare_result(
    params: DataCompareParams,
    key_columns: Vec<String>,
    source_response: TableDataResponse,
    target_response: TableDataResponse,
) -> anyhow::Result<DataCompareResult> {
    let columns = common_columns(
        &source_response.query_result.columns,
        &target_response.query_result.columns,
    );
    if columns.is_empty() {
        anyhow::bail!("No common columns to compare");
    }

    let mut result = compare_data_rows(
        rows_from_query_result(&source_response.query_result),
        rows_from_query_result(&target_response.query_result),
        DataCompareOptions {
            source_table: params.source_table,
            target_table: params.target_table,
            key_columns,
            columns,
        },
    )?;
    result.source_truncated = source_response.query_result.rows.len() < source_response.total_count;
    result.target_truncated = target_response.query_result.rows.len() < target_response.total_count;
    Ok(result)
}

async fn load_schema_tables(
    db_state: &Arc<GlobalDbState>,
    cx: &mut AsyncApp,
    connection_id: String,
    database: String,
    schema: Option<String>,
    progress_tx: &mpsc::UnboundedSender<CompareProgress>,
    side_label: &str,
) -> anyhow::Result<Vec<TableSchema>> {
    report(
        progress_tx,
        CompareProgress::phase(format!("正在加载{side_label}表列表")),
    );
    let tables = db_state
        .list_tables(cx, connection_id.clone(), database.clone(), schema.clone())
        .await?;
    let total = tables.len();
    let mut schemas = Vec::with_capacity(total);

    for (index, table) in tables.into_iter().enumerate() {
        report(
            progress_tx,
            CompareProgress::steps(format!("正在读取{side_label}表结构"), index + 1, total),
        );
        schemas.push(
            load_single_table_schema(
                db_state,
                cx,
                &connection_id,
                &database,
                schema.clone(),
                table,
            )
            .await?,
        );
    }

    Ok(schemas)
}

async fn load_single_table_schema(
    db_state: &Arc<GlobalDbState>,
    cx: &mut AsyncApp,
    connection_id: &str,
    database: &str,
    schema: Option<String>,
    table: TableInfo,
) -> anyhow::Result<TableSchema> {
    let table_name = table.name.clone();
    let columns = load_table_columns(
        db_state,
        cx,
        connection_id,
        database,
        schema.clone(),
        &table_name,
    )
    .await?;
    let indexes = db_state
        .list_indexes(
            cx,
            connection_id.to_string(),
            database.to_string(),
            schema.clone(),
            table_name.clone(),
        )
        .await?;
    let foreign_keys = db_state
        .list_foreign_keys(
            cx,
            connection_id.to_string(),
            database.to_string(),
            schema,
            table_name,
        )
        .await?;

    Ok(table_schema_from_metadata(
        table,
        columns,
        indexes,
        foreign_keys,
    ))
}

fn table_schema_from_metadata(
    table: TableInfo,
    columns: Vec<ColumnInfo>,
    indexes: Vec<IndexInfo>,
    foreign_keys: Vec<ForeignKeyDefinition>,
) -> TableSchema {
    TableSchema {
        name: table.name,
        columns: columns
            .into_iter()
            .map(|column| ColumnSchema {
                name: column.name,
                data_type: column.data_type,
                nullable: column.is_nullable,
                default_value: column.default_value,
                comment: column.comment,
            })
            .collect(),
        indexes: indexes
            .into_iter()
            .map(|index| IndexSchema {
                name: index.name,
                columns: index.columns,
                unique: index.is_unique,
            })
            .collect(),
        foreign_keys: foreign_keys
            .into_iter()
            .map(|foreign_key| ForeignKeySchema {
                name: foreign_key.name,
                columns: foreign_key.columns,
                ref_table: foreign_key.ref_table,
                ref_columns: foreign_key.ref_columns,
            })
            .collect(),
        comment: table.comment,
    }
}

fn resolve_key_columns(
    requested: &[String],
    source_columns: &[ColumnInfo],
    target_columns: &[ColumnInfo],
) -> anyhow::Result<Vec<String>> {
    let source_names = column_names(source_columns);
    let target_names = column_names(target_columns);

    if !requested.is_empty() {
        for key_column in requested {
            if !source_names.contains(key_column.as_str()) {
                anyhow::bail!("Key column `{}` does not exist on source table", key_column);
            }
            if !target_names.contains(key_column.as_str()) {
                anyhow::bail!("Key column `{}` does not exist on target table", key_column);
            }
        }
        return Ok(requested.to_vec());
    }

    let target_primary_names = target_columns
        .iter()
        .filter(|column| column.is_primary_key)
        .map(|column| column.name.as_str())
        .collect::<HashSet<_>>();
    let key_columns = source_columns
        .iter()
        .filter(|column| {
            column.is_primary_key && target_primary_names.contains(column.name.as_str())
        })
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();

    if key_columns.is_empty() {
        anyhow::bail!("Key columns are required when no common primary key can be inferred");
    }

    Ok(key_columns)
}

fn column_names(columns: &[ColumnInfo]) -> HashSet<&str> {
    columns.iter().map(|column| column.name.as_str()).collect()
}

fn table_data_request(
    database: String,
    schema: Option<String>,
    table: String,
    key_columns: &[String],
) -> TableDataRequest {
    let mut request = TableDataRequest::new(database, table).with_page(1, DATA_COMPARE_PAGE_SIZE);
    if let Some(schema) = schema {
        request = request.with_schema(schema);
    }
    if !key_columns.is_empty() {
        request = request.with_order_by_clause(key_columns.join(", "));
    }
    request
}

fn common_columns(source_columns: &[String], target_columns: &[String]) -> Vec<String> {
    let target = target_columns
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    source_columns
        .iter()
        .filter(|column| target.contains(column.as_str()))
        .cloned()
        .collect()
}

fn rows_from_query_result(result: &QueryResult) -> Vec<RowData> {
    result
        .rows
        .iter()
        .map(|row| {
            result
                .columns
                .iter()
                .enumerate()
                .filter_map(|(index, column)| {
                    let value = row.get(index)?;
                    let cell = value_to_cell(value.as_deref(), result.column_meta.get(index));
                    Some((column.clone(), cell))
                })
                .collect()
        })
        .collect()
}

fn value_to_cell(value: Option<&str>, meta: Option<&QueryColumnMeta>) -> serde_json::Value {
    let Some(value) = value else {
        return serde_json::Value::Null;
    };
    match meta.map(|meta| meta.field_type) {
        Some(FieldType::Integer) => parse_integer_cell(value),
        Some(FieldType::Decimal) => parse_decimal_cell(value),
        Some(FieldType::Boolean) => parse_boolean_cell(value),
        Some(FieldType::Json) => parse_json_cell(value),
        _ => serde_json::Value::String(value.to_string()),
    }
}

fn parse_integer_cell(value: &str) -> serde_json::Value {
    value
        .parse::<i64>()
        .map(serde_json::Value::from)
        .unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
}

fn parse_decimal_cell(value: &str) -> serde_json::Value {
    let Ok(number) = value.parse::<f64>() else {
        return serde_json::Value::String(value.to_string());
    };
    serde_json::Number::from_f64(number)
        .map(serde_json::Value::Number)
        .unwrap_or_else(|| serde_json::Value::String(value.to_string()))
}

fn parse_boolean_cell(value: &str) -> serde_json::Value {
    match value.to_ascii_lowercase().as_str() {
        "true" | "t" | "1" | "yes" => serde_json::Value::Bool(true),
        "false" | "f" | "0" | "no" => serde_json::Value::Bool(false),
        _ => serde_json::Value::String(value.to_string()),
    }
}

fn parse_json_cell(value: &str) -> serde_json::Value {
    serde_json::from_str(value).unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::{
        ColumnInfo, ForeignKeyDefinition, IndexInfo, QueryColumnMeta, QueryResult, TableInfo,
    };

    #[test]
    fn table_schema_from_metadata_preserves_columns_indexes_and_foreign_keys() {
        let table = TableInfo {
            name: "orders".to_string(),
            schema: Some("public".to_string()),
            comment: Some("order table".to_string()),
            engine: None,
            row_count: None,
            create_time: None,
            charset: None,
            collation: None,
        };
        let columns = vec![ColumnInfo {
            name: "id".to_string(),
            data_type: "int".to_string(),
            is_nullable: false,
            is_primary_key: true,
            default_value: None,
            comment: None,
            charset: None,
            collation: None,
        }];
        let indexes = vec![IndexInfo {
            name: "idx_orders_id".to_string(),
            columns: vec!["id".to_string()],
            is_unique: true,
            index_type: None,
        }];
        let foreign_keys = vec![ForeignKeyDefinition {
            name: "fk_orders_user".to_string(),
            columns: vec!["user_id".to_string()],
            ref_table: "users".to_string(),
            ref_columns: vec!["id".to_string()],
            on_delete: "CASCADE".to_string(),
            on_update: "NO ACTION".to_string(),
        }];

        let schema = table_schema_from_metadata(table, columns, indexes, foreign_keys);

        assert_eq!(schema.name, "orders");
        assert_eq!(schema.comment.as_deref(), Some("order table"));
        assert_eq!(schema.columns[0].name, "id");
        assert_eq!(schema.columns[0].nullable, false);
        assert_eq!(schema.indexes[0].name, "idx_orders_id");
        assert_eq!(schema.indexes[0].unique, true);
        assert_eq!(schema.foreign_keys[0].name, "fk_orders_user");
    }

    #[test]
    fn rows_from_query_result_maps_nulls_and_strings_by_column_name() {
        let result = QueryResult {
            sql: "select id, name from users".to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            column_meta: vec![
                QueryColumnMeta::new("id", "int"),
                QueryColumnMeta::new("name", "text"),
            ],
            rows: vec![vec![Some("1".to_string()), None]],
            elapsed_ms: 0,
        };

        let rows = rows_from_query_result(&result);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("id"), Some(&serde_json::json!(1)));
        assert_eq!(rows[0].get("name"), Some(&serde_json::Value::Null));
    }

    #[test]
    fn rows_from_query_result_uses_column_meta_for_common_types() {
        let result = QueryResult {
            sql: "select id, price, active, payload from products".to_string(),
            columns: vec![
                "id".to_string(),
                "price".to_string(),
                "active".to_string(),
                "payload".to_string(),
            ],
            column_meta: vec![
                QueryColumnMeta::new("id", "bigint"),
                QueryColumnMeta::new("price", "decimal"),
                QueryColumnMeta::new("active", "boolean"),
                QueryColumnMeta::new("payload", "json"),
            ],
            rows: vec![vec![
                Some("42".to_string()),
                Some("19.5".to_string()),
                Some("true".to_string()),
                Some("{\"sku\":\"A-1\"}".to_string()),
            ]],
            elapsed_ms: 0,
        };

        let rows = rows_from_query_result(&result);

        assert_eq!(rows[0].get("id"), Some(&serde_json::json!(42)));
        assert_eq!(rows[0].get("price"), Some(&serde_json::json!(19.5)));
        assert_eq!(rows[0].get("active"), Some(&serde_json::json!(true)));
        assert_eq!(
            rows[0].get("payload"),
            Some(&serde_json::json!({"sku": "A-1"}))
        );
    }

    #[test]
    fn resolve_key_columns_rejects_requested_columns_missing_on_target() {
        let source = vec![column_info("id", true), column_info("tenant_id", false)];
        let target = vec![column_info("id", true)];
        let requested = vec!["tenant_id".to_string()];

        let result = resolve_key_columns(&requested, &source, &target);

        assert!(result.is_err());
    }

    #[test]
    fn resolve_key_columns_infers_only_common_primary_keys() {
        let source = vec![column_info("id", true), column_info("tenant_id", true)];
        let target = vec![column_info("id", true), column_info("tenant_id", false)];

        let result = resolve_key_columns(&[], &source, &target).unwrap();

        assert_eq!(result, vec!["id".to_string()]);
    }

    fn column_info(name: &str, primary_key: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: "int".to_string(),
            is_nullable: false,
            is_primary_key: primary_key,
            default_value: None,
            comment: None,
            charset: None,
            collation: None,
        }
    }
}
