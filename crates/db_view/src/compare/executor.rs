use db::compare::{
    ColumnSchema, DataCompareOptions, DataCompareResult, ForeignKeySchema, IndexSchema, RowData,
    SchemaCompareOptions, SchemaCompareResult, SyncPlan, SyncPlanSummary, TableSchema,
    build_data_sync_plan, build_schema_sync_plan, compare_data_rows, compare_schemas,
};
use db::{
    ColumnInfo, FieldType, ForeignKeyDefinition, GlobalDbState, IndexInfo, QueryColumnMeta,
    QueryResult, TableDataRequest, TableDataResponse, TableInfo, plugin::DatabasePlugin,
};
use gpui::AsyncApp;
use rust_i18n::t;
use std::collections::{HashMap, HashSet};
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
    pub target_connection_id: String,
    pub target_database: String,
    pub target_schema: Option<String>,
    pub table_pairs: Vec<DataCompareTablePair>,
    pub key_columns: Vec<String>,
    pub case_sensitive_identifiers: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCompareTablePair {
    pub source_table: String,
    pub target_table: String,
}

#[derive(Debug, Clone)]
pub struct DataCompareBatchResult {
    pub table_results: Vec<DataCompareResult>,
}

impl DataCompareBatchResult {
    pub fn has_truncated_tables(&self) -> bool {
        self.table_results
            .iter()
            .any(|table| table.source_truncated || table.target_truncated)
    }
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
    pub case_sensitive_identifiers: bool,
}

/// 执行数据比较任务（简化版本）
pub async fn execute_data_compare(
    params: DataCompareParams,
    db_state: Arc<GlobalDbState>,
    progress_tx: mpsc::UnboundedSender<CompareProgress>,
    cx: &mut AsyncApp,
) -> anyhow::Result<DataCompareBatchResult> {
    let total_steps = params.table_pairs.len() * DATA_COMPARE_TOTAL_STEPS;
    let mut table_results = Vec::with_capacity(params.table_pairs.len());
    for (index, pair) in params.table_pairs.iter().cloned().enumerate() {
        let step_offset = index * DATA_COMPARE_TOTAL_STEPS;
        table_results.push(
            execute_data_compare_pair(
                &params,
                pair,
                &db_state,
                step_offset,
                total_steps,
                &progress_tx,
                cx,
            )
            .await?,
        );
    }
    Ok(DataCompareBatchResult { table_results })
}

async fn execute_data_compare_pair(
    params: &DataCompareParams,
    pair: DataCompareTablePair,
    db_state: &Arc<GlobalDbState>,
    step_offset: usize,
    total_steps: usize,
    progress_tx: &mpsc::UnboundedSender<CompareProgress>,
    cx: &mut AsyncApp,
) -> anyhow::Result<DataCompareResult> {
    report(
        progress_tx,
        CompareProgress::steps(
            t!("Compare.reading_source_table_schema").to_string(),
            step_offset + 1,
            total_steps,
        ),
    );
    let source_columns = load_table_columns(
        db_state,
        cx,
        &params.source_connection_id,
        &params.source_database,
        params.source_schema.clone(),
        &pair.source_table,
    )
    .await?;
    report(
        progress_tx,
        CompareProgress::steps(
            t!("Compare.reading_target_table_schema").to_string(),
            step_offset + 2,
            total_steps,
        ),
    );
    let target_columns = load_table_columns(
        db_state,
        cx,
        &params.target_connection_id,
        &params.target_database,
        params.target_schema.clone(),
        &pair.target_table,
    )
    .await?;
    let key_columns = resolve_key_columns(
        &params.key_columns,
        &source_columns,
        &target_columns,
        params.case_sensitive_identifiers,
    )?;
    let target_key_columns = matching_target_columns(
        &key_columns,
        &target_columns,
        params.case_sensitive_identifiers,
    );

    report(
        progress_tx,
        CompareProgress::steps(
            t!("Compare.loading_source_table_data").to_string(),
            step_offset + 3,
            total_steps,
        ),
    );
    let source_response = load_table_data(
        db_state,
        cx,
        &params.source_connection_id,
        &params.source_database,
        params.source_schema.clone(),
        &pair.source_table,
        &key_columns,
    )
    .await?;
    report(
        progress_tx,
        CompareProgress::steps(
            t!("Compare.loading_target_table_data").to_string(),
            step_offset + 4,
            total_steps,
        ),
    );
    let target_response = load_table_data(
        db_state,
        cx,
        &params.target_connection_id,
        &params.target_database,
        params.target_schema.clone(),
        &pair.target_table,
        &target_key_columns,
    )
    .await?;

    report(
        progress_tx,
        CompareProgress::steps(
            t!("Compare.comparing_data").to_string(),
            step_offset + 5,
            total_steps,
        ),
    );
    build_data_compare_result(
        pair,
        key_columns,
        source_response,
        target_response,
        params.case_sensitive_identifiers,
    )
}

/// 生成数据同步计划
pub fn generate_data_sync_plan(result: &DataCompareBatchResult) -> SyncPlan {
    if result.has_truncated_tables() {
        return truncated_data_sync_plan(result);
    }

    combine_sync_plans(
        result
            .table_results
            .iter()
            .map(build_data_sync_plan)
            .collect(),
    )
}

pub fn generate_data_sync_plan_for_target(
    result: &DataCompareBatchResult,
    db_state: &GlobalDbState,
    target_connection_id: &str,
    target_database: &str,
    target_schema: Option<&str>,
) -> anyhow::Result<SyncPlan> {
    if result.has_truncated_tables() {
        return Ok(truncated_data_sync_plan(result));
    }

    let config = db_state
        .get_config(target_connection_id)
        .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", target_connection_id))?;
    let plugin = db_state.get_plugin(&config.database_type)?;
    Ok(combine_sync_plans(
        result
            .table_results
            .iter()
            .map(|table_result| {
                db::compare::build_data_sync_plan_with_plugin(
                    table_result,
                    target_database,
                    target_schema,
                    plugin.as_ref(),
                )
            })
            .collect(),
    ))
}

fn truncated_data_sync_plan(result: &DataCompareBatchResult) -> SyncPlan {
    let target_table = match result.table_results.as_slice() {
        [] => String::new(),
        [table_result] => table_result.target_table.clone(),
        _ => format!("{} tables", result.table_results.len()),
    };
    SyncPlan {
        id: uuid::Uuid::new_v4().to_string(),
        target_table,
        statements: vec![],
        summary: SyncPlanSummary {
            insert_count: 0,
            update_count: 0,
            delete_count: 0,
            ddl_count: 0,
            total_count: 0,
        },
        warnings: vec![
            "Data compare result is truncated; sync SQL generation is disabled.".to_string(),
        ],
        sql_text: String::new(),
    }
}

fn combine_sync_plans(plans: Vec<SyncPlan>) -> SyncPlan {
    let target_table = match plans.as_slice() {
        [] => String::new(),
        [plan] => plan.target_table.clone(),
        _ => format!("{} tables", plans.len()),
    };
    let summary = plans.iter().fold(
        SyncPlanSummary {
            insert_count: 0,
            update_count: 0,
            delete_count: 0,
            ddl_count: 0,
            total_count: 0,
        },
        |mut summary, plan| {
            summary.insert_count += plan.summary.insert_count;
            summary.update_count += plan.summary.update_count;
            summary.delete_count += plan.summary.delete_count;
            summary.ddl_count += plan.summary.ddl_count;
            summary.total_count += plan.summary.total_count;
            summary
        },
    );
    let warnings = plans
        .iter()
        .flat_map(|plan| plan.warnings.iter().cloned())
        .collect::<Vec<_>>();
    let statements = plans
        .into_iter()
        .flat_map(|plan| plan.statements.into_iter())
        .collect::<Vec<_>>();
    let sql_text = statements
        .iter()
        .map(|statement| statement.sql.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    SyncPlan {
        id: uuid::Uuid::new_v4().to_string(),
        target_table,
        statements,
        summary,
        warnings,
        sql_text,
    }
}

/// 执行结构比较任务（简化版本）
pub async fn execute_schema_compare(
    params: SchemaCompareParams,
    db_state: Arc<GlobalDbState>,
    progress_tx: mpsc::UnboundedSender<CompareProgress>,
    cx: &mut AsyncApp,
) -> anyhow::Result<SchemaCompareResult> {
    let options = SchemaCompareOptions {
        case_sensitive_identifiers: params.case_sensitive_identifiers,
        ..SchemaCompareOptions::default()
    };
    let source_label = t!("Compare.source").to_string();
    let target_label = t!("Compare.target").to_string();
    let source_tables = load_schema_tables(
        &db_state,
        cx,
        params.source_connection_id,
        params.source_database,
        params.source_schema,
        &progress_tx,
        &source_label,
    )
    .await?;
    let target_tables = load_schema_tables(
        &db_state,
        cx,
        params.target_connection_id,
        params.target_database,
        params.target_schema,
        &progress_tx,
        &target_label,
    )
    .await?;
    report(
        &progress_tx,
        CompareProgress::phase(t!("Compare.comparing_schema").to_string()),
    );
    let result = compare_schemas(source_tables, target_tables, options)?;
    Ok(result)
}

/// 生成结构同步计划
pub fn generate_schema_sync_plan(result: &SchemaCompareResult, target_db_type: &str) -> SyncPlan {
    build_schema_sync_plan(result, target_db_type)
}

pub fn generate_schema_sync_plan_for_target(
    result: &SchemaCompareResult,
    db_state: &GlobalDbState,
    target_connection_id: &str,
    target_database: &str,
    target_schema: Option<&str>,
) -> anyhow::Result<SyncPlan> {
    let config = db_state
        .get_config(target_connection_id)
        .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", target_connection_id))?;
    let plugin = db_state.get_plugin(&config.database_type)?;
    Ok(db::compare::build_schema_sync_plan_with_plugin(
        result,
        target_database,
        target_schema,
        plugin.as_ref(),
    ))
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
    connection_id: &str,
    database: &str,
    schema: Option<String>,
    table: &str,
    key_columns: &[String],
) -> anyhow::Result<TableDataResponse> {
    let order_by_clause = order_by_clause_for_connection(db_state, connection_id, key_columns)?;
    db_state
        .query_table_data(
            cx,
            connection_id.to_string(),
            table_data_request(
                database.to_string(),
                schema,
                table.to_string(),
                order_by_clause.as_deref(),
            ),
        )
        .await
}

fn build_data_compare_result(
    pair: DataCompareTablePair,
    key_columns: Vec<String>,
    source_response: TableDataResponse,
    target_response: TableDataResponse,
    case_sensitive_identifiers: bool,
) -> anyhow::Result<DataCompareResult> {
    validate_unique_query_columns(
        &source_response.query_result.columns,
        "source result columns",
        case_sensitive_identifiers,
    )?;
    validate_unique_query_columns(
        &target_response.query_result.columns,
        "target result columns",
        case_sensitive_identifiers,
    )?;
    let columns = common_column_mappings(
        &source_response.query_result.columns,
        &target_response.query_result.columns,
        case_sensitive_identifiers,
    );
    if columns.is_empty() {
        anyhow::bail!("No common columns to compare");
    }
    let compare_columns = columns
        .iter()
        .map(|column| column.source.clone())
        .collect::<Vec<_>>();

    let mut result = compare_data_rows(
        rows_from_query_result_with_mappings(
            &source_response.query_result,
            &columns
                .iter()
                .map(|column| ColumnMapping {
                    source: column.source.clone(),
                    target: column.source.clone(),
                })
                .collect::<Vec<_>>(),
            case_sensitive_identifiers,
        ),
        rows_from_query_result_with_mappings(
            &target_response.query_result,
            &columns,
            case_sensitive_identifiers,
        ),
        DataCompareOptions {
            source_table: pair.source_table,
            target_table: pair.target_table,
            key_columns,
            columns: compare_columns,
        },
    )?;
    result.source_truncated = source_response.query_result.rows.len() < source_response.total_count;
    result.target_truncated = target_response.query_result.rows.len() < target_response.total_count;
    Ok(remap_data_compare_result_to_target_columns(
        result, &columns,
    ))
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
        CompareProgress::phase(
            t!("Compare.loading_table_list", side = side_label.to_string()).to_string(),
        ),
    );
    let tables = db_state
        .list_tables(cx, connection_id.clone(), database.clone(), schema.clone())
        .await?;
    let total = tables.len();
    let mut schemas = Vec::with_capacity(total);

    for (index, table) in tables.into_iter().enumerate() {
        report(
            progress_tx,
            CompareProgress::steps(
                t!(
                    "Compare.reading_table_schema",
                    side = side_label.to_string()
                )
                .to_string(),
                index + 1,
                total,
            ),
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
    let primary_columns = columns
        .iter()
        .filter(|column| column.is_primary_key)
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let mut indexes = indexes
        .into_iter()
        .map(|index| IndexSchema {
            name: index.name,
            columns: index.columns,
            unique: index.is_unique,
        })
        .collect::<Vec<_>>();
    if !primary_columns.is_empty()
        && !indexes
            .iter()
            .any(|index| index.name.eq_ignore_ascii_case("PRIMARY"))
    {
        indexes.push(IndexSchema {
            name: "PRIMARY".to_string(),
            columns: primary_columns,
            unique: true,
        });
    }

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
        indexes,
        foreign_keys: foreign_keys
            .into_iter()
            .map(|foreign_key| ForeignKeySchema {
                name: foreign_key.name,
                columns: foreign_key.columns,
                ref_table: foreign_key.ref_table,
                ref_columns: foreign_key.ref_columns,
                on_delete: non_empty_string(foreign_key.on_delete),
                on_update: non_empty_string(foreign_key.on_update),
            })
            .collect(),
        comment: table.comment,
    }
}

fn non_empty_string(value: String) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn resolve_key_columns(
    requested: &[String],
    source_columns: &[ColumnInfo],
    target_columns: &[ColumnInfo],
    case_sensitive_identifiers: bool,
) -> anyhow::Result<Vec<String>> {
    validate_unique_column_infos(
        source_columns,
        "source table columns",
        case_sensitive_identifiers,
    )?;
    validate_unique_column_infos(
        target_columns,
        "target table columns",
        case_sensitive_identifiers,
    )?;
    let source_names = column_map_by_identifier_key(source_columns, case_sensitive_identifiers);
    let target_names = column_map_by_identifier_key(target_columns, case_sensitive_identifiers);

    if !requested.is_empty() {
        let mut resolved = Vec::with_capacity(requested.len());
        for key_column in requested {
            let key = identifier_key(key_column, case_sensitive_identifiers);
            let Some(source_name) = source_names.get(&key) else {
                anyhow::bail!("Key column `{}` does not exist on source table", key_column);
            };
            if !target_names.contains_key(&key) {
                anyhow::bail!("Key column `{}` does not exist on target table", key_column);
            }
            resolved.push(source_name.clone());
        }
        return Ok(resolved);
    }

    let target_primary_names = target_columns
        .iter()
        .filter(|column| column.is_primary_key)
        .map(|column| identifier_key(&column.name, case_sensitive_identifiers))
        .collect::<HashSet<_>>();
    let key_columns = source_columns
        .iter()
        .filter(|column| {
            column.is_primary_key
                && target_primary_names
                    .contains(&identifier_key(&column.name, case_sensitive_identifiers))
        })
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();

    if key_columns.is_empty() {
        anyhow::bail!("Key columns are required when no common primary key can be inferred");
    }

    Ok(key_columns)
}

fn matching_target_columns(
    source_names: &[String],
    target_columns: &[ColumnInfo],
    case_sensitive_identifiers: bool,
) -> Vec<String> {
    let target_names = column_map_by_identifier_key(target_columns, case_sensitive_identifiers);
    source_names
        .iter()
        .filter_map(|name| {
            target_names
                .get(&identifier_key(name, case_sensitive_identifiers))
                .cloned()
        })
        .collect()
}

fn column_map_by_identifier_key(
    columns: &[ColumnInfo],
    case_sensitive_identifiers: bool,
) -> HashMap<String, String> {
    columns
        .iter()
        .map(|column| {
            (
                identifier_key(&column.name, case_sensitive_identifiers),
                column.name.clone(),
            )
        })
        .collect()
}

fn validate_unique_column_infos(
    columns: &[ColumnInfo],
    scope: &str,
    case_sensitive_identifiers: bool,
) -> anyhow::Result<()> {
    validate_unique_identifier_names(
        columns.iter().map(|column| column.name.as_str()),
        scope,
        case_sensitive_identifiers,
    )
}

fn validate_unique_query_columns(
    columns: &[String],
    scope: &str,
    case_sensitive_identifiers: bool,
) -> anyhow::Result<()> {
    validate_unique_identifier_names(
        columns.iter().map(String::as_str),
        scope,
        case_sensitive_identifiers,
    )
}

fn validate_unique_identifier_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
    scope: &str,
    case_sensitive_identifiers: bool,
) -> anyhow::Result<()> {
    let mut seen = HashMap::new();
    for name in names {
        let key = identifier_key(name, case_sensitive_identifiers);
        if let Some(previous) = seen.insert(key, name.to_string()) {
            anyhow::bail!(
                "Duplicate case-insensitive column names in {}: `{}` and `{}`",
                scope,
                previous,
                name
            );
        }
    }
    Ok(())
}

fn table_data_request(
    database: String,
    schema: Option<String>,
    table: String,
    order_by_clause: Option<&str>,
) -> TableDataRequest {
    let mut request = TableDataRequest::new(database, table).with_page(1, DATA_COMPARE_PAGE_SIZE);
    if let Some(schema) = schema {
        request = request.with_schema(schema);
    }
    if let Some(order_by_clause) = order_by_clause {
        request = request.with_order_by_clause(order_by_clause);
    }
    request
}

fn order_by_clause_for_connection(
    db_state: &GlobalDbState,
    connection_id: &str,
    key_columns: &[String],
) -> anyhow::Result<Option<String>> {
    if key_columns.is_empty() {
        return Ok(None);
    }
    let config = db_state
        .get_config(connection_id)
        .ok_or_else(|| anyhow::anyhow!("Connection not found: {}", connection_id))?;
    let plugin = db_state.get_plugin(&config.database_type)?;
    Ok(quoted_order_by_clause(plugin.as_ref(), key_columns))
}

fn quoted_order_by_clause(plugin: &dyn DatabasePlugin, key_columns: &[String]) -> Option<String> {
    if key_columns.is_empty() {
        return None;
    }
    Some(
        key_columns
            .iter()
            .map(|column| plugin.quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

#[derive(Clone)]
struct ColumnMapping {
    source: String,
    target: String,
}

fn common_column_mappings(
    source_columns: &[String],
    target_columns: &[String],
    case_sensitive_identifiers: bool,
) -> Vec<ColumnMapping> {
    let target = target_columns
        .iter()
        .map(|column| {
            (
                identifier_key(column, case_sensitive_identifiers),
                column.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    source_columns
        .iter()
        .filter_map(|source| {
            target
                .get(&identifier_key(source, case_sensitive_identifiers))
                .map(|target| ColumnMapping {
                    source: source.clone(),
                    target: target.clone(),
                })
        })
        .collect()
}

#[cfg(test)]
fn rows_from_query_result(result: &QueryResult) -> Vec<RowData> {
    let mappings = result
        .columns
        .iter()
        .map(|column| ColumnMapping {
            source: column.clone(),
            target: column.clone(),
        })
        .collect::<Vec<_>>();
    rows_from_query_result_with_mappings(result, &mappings, false)
}

fn rows_from_query_result_with_mappings(
    result: &QueryResult,
    mappings: &[ColumnMapping],
    case_sensitive_identifiers: bool,
) -> Vec<RowData> {
    let index_by_column = result
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| (identifier_key(column, case_sensitive_identifiers), index))
        .collect::<HashMap<_, _>>();
    result
        .rows
        .iter()
        .map(|row| {
            mappings
                .iter()
                .filter_map(|mapping| {
                    let index = *index_by_column
                        .get(&identifier_key(&mapping.target, case_sensitive_identifiers))?;
                    let value = row.get(index)?;
                    let cell = value_to_cell(value.as_deref(), result.column_meta.get(index));
                    Some((mapping.source.clone(), cell))
                })
                .collect()
        })
        .collect()
}

fn remap_data_compare_result_to_target_columns(
    mut result: DataCompareResult,
    mappings: &[ColumnMapping],
) -> DataCompareResult {
    result.key_columns = result
        .key_columns
        .iter()
        .map(|column| target_column_name(column, mappings))
        .collect();
    result.columns = result
        .columns
        .iter()
        .map(|column| target_column_name(column, mappings))
        .collect();
    result.added = result
        .added
        .into_iter()
        .map(|row| remap_row_to_target_columns(row, mappings))
        .collect();
    result.removed = result
        .removed
        .into_iter()
        .map(|row| remap_row_to_target_columns(row, mappings))
        .collect();
    result.modified = result
        .modified
        .into_iter()
        .map(|row| remap_modified_row_to_target_columns(row, mappings))
        .collect();
    result
}

fn remap_modified_row_to_target_columns(
    row: db::compare::DataCompareModifiedRow,
    mappings: &[ColumnMapping],
) -> db::compare::DataCompareModifiedRow {
    db::compare::DataCompareModifiedRow {
        key_values: row
            .key_values
            .into_iter()
            .map(|(column, value)| (target_column_name(&column, mappings), value))
            .collect(),
        source_values: remap_row_to_target_columns(row.source_values, mappings),
        target_values: remap_row_to_target_columns(row.target_values, mappings),
        changes: row
            .changes
            .into_iter()
            .map(|(column, values)| (target_column_name(&column, mappings), values))
            .collect(),
    }
}

fn remap_row_to_target_columns(row: RowData, mappings: &[ColumnMapping]) -> RowData {
    mappings
        .iter()
        .filter_map(|mapping| {
            row.get(&mapping.source)
                .cloned()
                .map(|value| (mapping.target.clone(), value))
        })
        .collect()
}

fn target_column_name(source_column: &str, mappings: &[ColumnMapping]) -> String {
    mappings
        .iter()
        .find(|mapping| mapping.source == source_column)
        .map(|mapping| mapping.target.clone())
        .unwrap_or_else(|| source_column.to_string())
}

fn identifier_key(value: &str, case_sensitive_identifiers: bool) -> String {
    if case_sensitive_identifiers {
        value.trim().to_string()
    } else {
        value.trim().to_lowercase()
    }
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
    let Some(number) = canonical_decimal_literal(value) else {
        return serde_json::Value::String(value.to_string());
    };
    serde_json::Value::Number(serde_json::Number::from_string_unchecked(number))
}

fn canonical_decimal_literal(value: &str) -> Option<String> {
    let value = value.trim();
    let (negative, digits) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let (integer, fraction) = match digits.split_once('.') {
        Some((integer, fraction)) => (integer, fraction),
        None => (digits, ""),
    };
    if integer.is_empty() && fraction.is_empty() {
        return None;
    }
    if !integer.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    if !fraction.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let fraction = fraction.trim_end_matches('0');
    let mut number = if fraction.is_empty() {
        integer.to_string()
    } else {
        format!("{integer}.{fraction}")
    };
    if negative && number != "0" {
        number.insert(0, '-');
    }
    Some(number)
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
    use serde_json::json;

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
            is_primary: false,
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
        assert!(schema.indexes.iter().any(|index| index.name == "PRIMARY"
            && index.columns == vec!["id".to_string()]
            && index.unique));
        assert_eq!(schema.foreign_keys[0].name, "fk_orders_user");
        assert_eq!(schema.foreign_keys[0].on_delete.as_deref(), Some("CASCADE"));
        assert_eq!(
            schema.foreign_keys[0].on_update.as_deref(),
            Some("NO ACTION")
        );
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
    fn rows_from_query_result_preserves_decimal_precision() {
        let result = QueryResult {
            sql: "select price from invoices".to_string(),
            columns: vec!["price".to_string()],
            column_meta: vec![QueryColumnMeta::new("price", "decimal")],
            rows: vec![vec![Some("12345678901234567890.1234500".to_string())]],
            elapsed_ms: 0,
        };

        let rows = rows_from_query_result(&result);

        assert_eq!(
            rows[0].get("price").map(ToString::to_string).as_deref(),
            Some("12345678901234567890.12345")
        );
    }

    #[test]
    fn resolve_key_columns_rejects_requested_columns_missing_on_target() {
        let source = vec![column_info("id", true), column_info("tenant_id", false)];
        let target = vec![column_info("id", true)];
        let requested = vec!["tenant_id".to_string()];

        let result = resolve_key_columns(&requested, &source, &target, false);

        assert!(result.is_err());
    }

    #[test]
    fn resolve_key_columns_infers_only_common_primary_keys() {
        let source = vec![column_info("id", true), column_info("tenant_id", true)];
        let target = vec![column_info("id", true), column_info("tenant_id", false)];

        let result = resolve_key_columns(&[], &source, &target, false).unwrap();

        assert_eq!(result, vec!["id".to_string()]);
    }

    #[test]
    fn order_by_clause_quotes_key_columns_with_plugin() {
        let plugin = db::mysql::MySqlPlugin::new();
        let key_columns = vec!["order".to_string(), "id".to_string()];

        let clause = quoted_order_by_clause(&plugin, &key_columns);

        assert_eq!(clause.as_deref(), Some("`order`, `id`"));
    }

    #[test]
    fn build_data_compare_result_matches_column_names_case_insensitively() {
        let source = table_data_response(
            vec!["ID", "Name"],
            vec![
                QueryColumnMeta::new("ID", "int"),
                QueryColumnMeta::new("Name", "text"),
            ],
            vec![vec![Some("1".to_string()), Some("Ada".to_string())]],
        );
        let target = table_data_response(
            vec!["id", "name"],
            vec![
                QueryColumnMeta::new("id", "int"),
                QueryColumnMeta::new("name", "text"),
            ],
            vec![vec![Some("1".to_string()), Some("Ada".to_string())]],
        );

        let result = build_data_compare_result(
            DataCompareTablePair {
                source_table: "Users".to_string(),
                target_table: "users".to_string(),
            },
            vec!["ID".to_string()],
            source,
            target,
            false,
        )
        .unwrap();

        assert_eq!(result.columns, vec!["id".to_string(), "name".to_string()]);
        assert!(result.added.is_empty());
        assert!(result.removed.is_empty());
        assert!(result.modified.is_empty());
    }

    #[test]
    fn build_data_compare_result_can_match_column_names_case_sensitively() {
        let source = table_data_response(
            vec!["ID", "Name"],
            vec![
                QueryColumnMeta::new("ID", "int"),
                QueryColumnMeta::new("Name", "text"),
            ],
            vec![vec![Some("1".to_string()), Some("Ada".to_string())]],
        );
        let target = table_data_response(
            vec!["id", "name"],
            vec![
                QueryColumnMeta::new("id", "int"),
                QueryColumnMeta::new("name", "text"),
            ],
            vec![vec![Some("1".to_string()), Some("Ada".to_string())]],
        );

        let result = build_data_compare_result(
            DataCompareTablePair {
                source_table: "Users".to_string(),
                target_table: "users".to_string(),
            },
            vec!["ID".to_string()],
            source,
            target,
            true,
        );

        assert!(result.is_err());
    }

    #[test]
    fn build_data_compare_result_uses_target_column_names_for_sync_sql() {
        let source = table_data_response(
            vec!["ID", "Name"],
            vec![
                QueryColumnMeta::new("ID", "int"),
                QueryColumnMeta::new("Name", "text"),
            ],
            vec![
                vec![Some("1".to_string()), Some("Ada".to_string())],
                vec![Some("2".to_string()), Some("Grace".to_string())],
            ],
        );
        let target = table_data_response(
            vec!["id", "name"],
            vec![
                QueryColumnMeta::new("id", "int"),
                QueryColumnMeta::new("name", "text"),
            ],
            vec![vec![Some("1".to_string()), Some("Adele".to_string())]],
        );

        let table_result = build_data_compare_result(
            DataCompareTablePair {
                source_table: "Users".to_string(),
                target_table: "users".to_string(),
            },
            vec!["ID".to_string()],
            source,
            target,
            false,
        )
        .unwrap();
        let plan = generate_data_sync_plan(&DataCompareBatchResult {
            table_results: vec![table_result],
        });

        assert!(plan.sql_text.contains("INSERT INTO users (id, name)"));
        assert!(plan.sql_text.contains("UPDATE users SET name = 'Ada'"));
        assert!(plan.sql_text.contains("WHERE id = 1"));
        assert!(!plan.sql_text.contains("ID"));
        assert!(!plan.sql_text.contains("Name"));
    }

    #[test]
    fn build_data_compare_result_rejects_duplicate_normalized_column_names() {
        let source = table_data_response(
            vec!["ID", "id"],
            vec![
                QueryColumnMeta::new("ID", "int"),
                QueryColumnMeta::new("id", "int"),
            ],
            vec![vec![Some("1".to_string()), Some("2".to_string())]],
        );
        let target = table_data_response(
            vec!["id"],
            vec![QueryColumnMeta::new("id", "int")],
            vec![vec![Some("1".to_string())]],
        );

        let result = build_data_compare_result(
            DataCompareTablePair {
                source_table: "Users".to_string(),
                target_table: "users".to_string(),
            },
            vec!["ID".to_string()],
            source,
            target,
            false,
        );

        assert!(result.is_err());
    }

    #[test]
    fn generate_data_sync_plan_blocks_truncated_results() {
        let result = DataCompareBatchResult {
            table_results: vec![DataCompareResult {
                source_table: "users".to_string(),
                target_table: "users".to_string(),
                key_columns: vec!["id".to_string()],
                columns: vec!["id".to_string(), "name".to_string()],
                added: vec![row_data(vec![("id", json!(1)), ("name", json!("Ada"))])],
                removed: vec![],
                modified: vec![],
                source_truncated: true,
                target_truncated: false,
            }],
        };

        let plan = generate_data_sync_plan(&result);

        assert_eq!(plan.summary.total_count, 0);
        assert!(plan.statements.is_empty());
        assert!(plan.sql_text.is_empty());
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("truncated"))
        );
    }

    #[test]
    fn generate_data_sync_plan_combines_multiple_table_results() {
        let result = DataCompareBatchResult {
            table_results: vec![
                DataCompareResult {
                    source_table: "users".to_string(),
                    target_table: "users".to_string(),
                    key_columns: vec!["id".to_string()],
                    columns: vec!["id".to_string(), "name".to_string()],
                    added: vec![row_data(vec![("id", json!(1)), ("name", json!("Ada"))])],
                    removed: vec![],
                    modified: vec![],
                    source_truncated: false,
                    target_truncated: false,
                },
                DataCompareResult {
                    source_table: "orders".to_string(),
                    target_table: "orders".to_string(),
                    key_columns: vec!["id".to_string()],
                    columns: vec!["id".to_string(), "total".to_string()],
                    added: vec![row_data(vec![("id", json!(7)), ("total", json!(19.5))])],
                    removed: vec![],
                    modified: vec![],
                    source_truncated: false,
                    target_truncated: false,
                },
            ],
        };

        let plan = generate_data_sync_plan(&result);

        assert_eq!(plan.target_table, "2 tables");
        assert_eq!(plan.summary.insert_count, 2);
        assert_eq!(plan.summary.total_count, 2);
        assert!(plan.sql_text.contains("INSERT INTO users"));
        assert!(plan.sql_text.contains("INSERT INTO orders"));
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

    fn table_data_response(
        columns: Vec<&str>,
        column_meta: Vec<QueryColumnMeta>,
        rows: Vec<Vec<Option<String>>>,
    ) -> TableDataResponse {
        TableDataResponse {
            total_count: rows.len(),
            page: 1,
            page_size: rows.len(),
            duration: 0,
            query_result: QueryResult {
                sql: String::new(),
                columns: columns.into_iter().map(ToString::to_string).collect(),
                column_meta,
                rows,
                elapsed_ms: 0,
            },
        }
    }

    fn row_data(values: Vec<(&str, serde_json::Value)>) -> RowData {
        values
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }
}
